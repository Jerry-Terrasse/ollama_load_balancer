use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server, StatusCode, server::conn::AddrStream};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use futures_util::stream::StreamExt;
use futures_util::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long, required = true)]
    server: Vec<String>,
}

#[derive(Clone, Debug)]
enum FailureRecord {
    Reliable,
    Unreliable,
    SecondChanceGiven,
}

#[derive(Debug)]
struct ServerState {
    busy: bool,
    failure_record: FailureRecord,
}

#[derive(Clone, Debug)]
struct OllamaServer {
    address: String,
    state: Arc<Mutex<ServerState>>,
}

type SharedServerList = Arc<RwLock<Vec<Arc<OllamaServer>>>>;

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let servers = args.server.into_iter().map(|address| {
        Arc::new(OllamaServer {
            address,
            state: Arc::new(Mutex::new(ServerState {
                busy: false,
                failure_record: FailureRecord::Reliable,
            })),
        })
    }).collect::<Vec<_>>();
    
    println!("📒 Ollama servers list:");
    for (index, server) in servers.iter().enumerate() {
        println!("{}. {}", index + 1, server.address);
    }
    println!("");
    
    let servers = Arc::new(RwLock::new(servers));
    
    let make_svc = make_service_fn(|conn: &AddrStream| {
        let remote_addr = conn.remote_addr();
        let servers = servers.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let servers = servers.clone();
                handle_request(req, servers, remote_addr)
            }))
        }
    });
    
    let addr = ([0, 0, 0, 0], 11434).into();
    
    let server = Server::bind(&addr).serve(make_svc);
    
    // Implement graceful shutdown
    let graceful = server.with_graceful_shutdown(shutdown_signal());
    
    println!("👂 Ollama Load Balancer listening on http://{}", addr);
    
    if let Err(e) = graceful.await {
        eprintln!("Server error: {}", e);
    }
}

async fn shutdown_signal() {
    // Wait for the CTRL+C signal
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl_c");
    
    println!("☠️  Received CTRL+C, shutting down gracefully...");
    // The future returned by ctrl_c() will resolve when CTRL+C is pressed
    // Hyper will then stop accepting new connections
}

async fn handle_request(
    req: Request<Body>,
    servers: SharedServerList,
    remote_addr: std::net::SocketAddr,
) -> Result<Response<Body>, Infallible> {
    // Only handle POST requests
    if req.method() != hyper::Method::POST {
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Body::from("Only POST requests are allowed"))
            .unwrap());
    }
    
    // Get the path
    let path = req.uri().path();
    
    // Select an available server
    let server = select_available_server(&servers, &remote_addr).await;
    
    if let Some(server) = server {
        // As long as guard object is alive, the server will be marked as "in use"
        let _guard = ServerGuard {
            server: server.clone(),
        };
        
        // Build the request to the Ollama server
        let uri = format!("{}{}", server.address, path);
        
        let client = reqwest::Client::builder()
            // Timeout so that upon Ollama server crash / sudden shutdown
            // we're not stuck forever (literally)
            .read_timeout(std::time::Duration::from_secs(30))
            .build().unwrap();
        let mut request_builder = client.request(reqwest::Method::POST, &uri);
        
        // Copy headers
        for (key, value) in req.headers() {
            request_builder = request_builder.header(key.as_str(), value.as_bytes());
        }
        
        // Set up streaming body
        let body_stream = req.into_body().map(|chunk_result| match chunk_result {
            Ok(chunk) => Ok(chunk.to_vec()),
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
        });
        
        let reqwest_body = reqwest::Body::wrap_stream(body_stream);
        
        request_builder = request_builder.body(reqwest_body);
        
        // Send the request and handle the response
        match request_builder.send().await {
            Ok(response) => {
                // If the server was previously Unreliable or SecondChanceGiven, set it back to Reliable
                {
                    let mut state = server.state.lock().unwrap();
                    if !matches!(state.failure_record, FailureRecord::Reliable) {
                        state.failure_record = FailureRecord::Reliable;
                        println!("🙏⚕️ Server {} has recovered and is now marked Reliable", server.address);
                    }
                }
                
                let status = response.status();
                let mut resp_builder = Response::builder().status(u16::from(status));
                
                // Copy headers
                for (key, value) in response.headers() {
                    resp_builder = resp_builder.header(key.to_string(), value.to_str().unwrap());
                }
                
                // Wrap the response body stream with our custom stream.
                // The purpose of our custom stream as opposed to directly using response.bytes_stream()
                // is so we can keep track of the stream lifetime- to mark the server as available once again.
                let resp_body = ResponseBodyWithGuard {
                    stream: response.bytes_stream(),
                    _guard,
                };
                
                // Convert our custom stream to hyper::Body
                let hyper_body = Body::wrap_stream(resp_body);
                
                let response = resp_builder.body(hyper_body).unwrap();
                
                Ok(response)
            }
            Err(e) => {
                {
                    let mut state = server.state.lock().unwrap();
                    // Sever just failed our request, it's obviously not Reliable
                    if matches!(state.failure_record, FailureRecord::Reliable) {
                        state.failure_record = FailureRecord::Unreliable;
                        println!("⛔😱 Server {} has failed, now marked unreliable. Error: {}", server.address, e);
                    }
                    else {
                        println!("⛔😞 Server {} has failed again. Error: {}", server.address, e);
                    }
                }
                
                // Return an error to the client
                let response = Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Body::from(format!("Error connecting to Ollama server: {}", e)))
                    .unwrap();
                Ok(response)
            }
        }
    } else {
        println!("🤷 No available servers to serve client {}", remote_addr);
        let response = Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(Body::from("No available servers"))
            .unwrap();
        Ok(response)
    }
}

async fn select_available_server(servers: &SharedServerList, remote_addr: &std::net::SocketAddr) -> Option<Arc<OllamaServer>> {
    let servers = servers.read().await;

    // 1st choice: Find an available reliable server
    for server in servers.iter() {
        let mut state = server.state.lock().unwrap();
        if matches!(state.failure_record, FailureRecord::Reliable) && !state.busy {
            state.busy = true;
            println!("🤖🦸 Chose reliable server: {} to serve client {}", server.address, remote_addr);
            return Some(server.clone());
        }
    }
    
    // 2nd choice: If no reliable servers are available, select an untrusted available server that has
    // only failed once in a row.
    for server in servers.iter() {
        let mut state = server.state.lock().unwrap();
        if matches!(state.failure_record, FailureRecord::Unreliable) && !state.busy {
            state.busy = true;
            state.failure_record = FailureRecord::SecondChanceGiven;
            println!("🤖😇 Giving server {} another chance with client {}", server.address, remote_addr);
            return Some(server.clone());
        }
    }
    
    // If all untrusted available servers have been given a second chance,
    // reset the SecondChanceGiven mark so that we can again cycle through the untrusted servers-
    // This ensures that we cycle equally through all untrusted servers- give everyone
    // their chance
    for server in servers.iter() {
        let mut state = server.state.lock().unwrap();
        if matches!(state.failure_record, FailureRecord::SecondChanceGiven) && !state.busy {
            state.failure_record = FailureRecord::Unreliable;
        }
    }
    // 3rd choice: Select any untrusted server, because we're out of options at this point
    for server in servers.iter() {
        let mut state = server.state.lock().unwrap();
        if matches!(state.failure_record, FailureRecord::Unreliable) && !state.busy {
            state.busy = true;
            state.failure_record = FailureRecord::SecondChanceGiven;
            println!("🤖😇 Giving server {} a 3rd+ chance with client {}", server.address, remote_addr);
            return Some(server.clone());
        }
    }
    
    // No servers available
    None
}

struct ServerGuard {
    server: Arc<OllamaServer>,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        println!("🟢 Server {} now available", self.server.address);
        let mut state = self.server.state.lock().unwrap();
        state.busy = false;
    }
}

// Custom stream that holds the guard
struct ResponseBodyWithGuard<S> {
    stream: S,
    _guard: ServerGuard,
}

impl<S> Stream for ResponseBodyWithGuard<S>
where
    S: Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
{
    type Item = Result<bytes::Bytes, std::io::Error>;
    
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let stream = Pin::new(&mut self.stream);
        match stream.poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => Poll::Ready(Some(Ok(bytes))),
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(std::io::Error::new(std::io::ErrorKind::Other, e)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
