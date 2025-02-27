use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server, StatusCode, server::conn::AddrStream};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use futures_util::stream::StreamExt;
use futures_util::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
use clap::Parser;
use ordermap::OrderMap;

/// Struct to hold the user-supplied server address and its human-readable name.
/// Format on the command line should be:  ip:port=Name
#[derive(Debug, Clone)]
struct ServerConfig {
    address: String,
    name: String,
}

impl std::str::FromStr for ServerConfig {
    type Err = String;

    /// We expect the user to provide something like "127.0.0.1:11433=LocalOllama"
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.splitn(2, '=').collect();
        if parts.len() != 2 {
            return Err("Invalid server format. Use ip:port=Name".to_string());
        }
        Ok(ServerConfig {
            address: parts[0].trim().to_string(),
            name: parts[1].trim().to_string(),
        })
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Syntax is --server IP:PORT=NAME --server IP:PORT=NAME ...
    ///
    /// This is a required argument. It specifies the addresses of the Ollama servers
    /// that the load balancer will distribute requests to, plus a friendly name.
    #[arg(short, long, required = true)]
    server: Vec<ServerConfig>,

    /// Max seconds to allow Ollama server to pause.
    ///
    /// Don't set this too low because if the delay is too great at the beginning of response generation that will cause failure.
    /// Pass 0 to disable timeout.
    /// 
    /// This is an optional argument. It specifies the maximum number of seconds to wait for a response from the Ollama server before considering it unavailable
    #[arg(short, long, default_value_t = 30)]
    timeout: u32,

    /// Listening address. Defaults to "0.0.0.0:11434"
    #[arg(short = 'l', long, default_value = "0.0.0.0:11434")]
    listen: String,
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

#[derive(Debug)]
struct OllamaServer {
    state: ServerState,
    name: String,
}

type SharedServerList = Arc<Mutex<OrderMap<String, OllamaServer>>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let mut servers_map = OrderMap::new();
    for config in args.server {
        let address = config.address.clone();
        let name = config.name.clone();

        if servers_map.contains_key(&address) {
            return Err(format!("Duplicate server address found: {}", address).into());
        }
        servers_map.insert(address.clone(), OllamaServer {
            state: ServerState {
                busy: false,
                failure_record: FailureRecord::Reliable,
            },
            name,
        });
    }

    println!("");
    println!("📒 Ollama servers list:");
    for (index, (addr, srv)) in servers_map.iter().enumerate() {
        println!("{}. {} ({})", index + 1, addr, srv.name);
    }
    println!("");
    println!("⚙️  Timeout setting: Will abandon Ollama server after {} seconds of silence", args.timeout);
    println!("");

    let servers = Arc::new(Mutex::new(servers_map));

    let make_svc = make_service_fn(|conn: &AddrStream| {
        let remote_addr = conn.remote_addr();
        let servers = servers.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let servers = servers.clone();
                handle_request(req, servers, remote_addr, args.timeout)
            }))
        }
    });

    let addr: std::net::SocketAddr = args.listen.parse()?;

    let server = Server::bind(&addr).serve(make_svc);

    // Implement graceful shutdown
    let graceful = server.with_graceful_shutdown(shutdown_signal());

    println!("👂 Ollama Load Balancer listening on http://{}", addr);
    println!("");

    if let Err(e) = graceful.await {
        return Err(e.into());
    }

    Ok(())
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
    timeout_secs: u32,
) -> Result<Response<Body>, Infallible> {
    let reqwest_method = match hyper_method_to_reqwest_method(req.method().clone()) {
        Ok(method) => method,
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .body(Body::from(format!("hyper_method_to_reqwest_method failed: {}", e)))
                .unwrap());
        }
    };

    // Get the path
    let path = req.uri().path();

    // Select an available server
    let server_key = select_available_server(&servers, &remote_addr).await;

    if let Some(key) = server_key {
        // As long as guard object is alive, the server will be marked as "in use"
        let _guard = ServerGuard {
            servers: servers.clone(),
            key: key.clone(),
        };

        // Build the request to the Ollama server
        let uri = format!("{}{}", key, path);

        let mut builder = reqwest::Client::builder();
        // Low value for connect timeout, to get an immediate error
        // if the Ollama server isn't even running.
        // Even if the Ollama server takes its time, it should still be
        // able to immediately facilitate a TCP connection with us.
        builder = builder.connect_timeout(std::time::Duration::from_secs(1));
        if timeout_secs == 0 {
            builder = builder.pool_idle_timeout(None);
        }
        else {
            let timeout = std::time::Duration::from_secs(timeout_secs.into());
            builder = builder.read_timeout(timeout).pool_idle_timeout(timeout);
        }
        let client = builder.build().unwrap();
        let mut request_builder = client.request(reqwest_method, &uri);

        // Copy headers
        for (key_h, value) in req.headers() {
            request_builder = request_builder.header(key_h.as_str(), value.as_bytes());
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
                let status = response.status();
                let mut resp_builder = Response::builder().status(u16::from(status));

                // Copy headers
                for (key_h, value) in response.headers() {
                    resp_builder = resp_builder.header(key_h.to_string(), value.to_str().unwrap());
                }

                // Wrap the response body stream with our custom stream.
                // The purpose of our custom stream as opposed to directly using response.bytes_stream()
                // is so we can keep track of the stream lifetime- to mark the server as available once again.
                let resp_body = ResponseBodyWithGuard {
                    stream: response.bytes_stream(),
                    _guard,
                    servers: servers.clone(),
                    key: key.clone(),
                    had_error: false,
                };

                // Convert our custom stream to hyper::Body
                let hyper_body = Body::wrap_stream(resp_body);

                let response = resp_builder.body(hyper_body).unwrap();

                Ok(response)
            }
            Err(e) => {
                {
                    let mut servers_lock = servers.lock().unwrap();
                    if let Some(server) = servers_lock.get_mut(&key) {
                        if matches!(server.state.failure_record, FailureRecord::Reliable) {
                            server.state.failure_record = FailureRecord::Unreliable;
                            println!("⛔😱 Server {} ({}) didn't respond, now marked Unreliable. Error: {}", key, server.name, e);
                        }
                        else {
                            server.state.failure_record = FailureRecord::SecondChanceGiven;
                            println!("⛔😞 Unreliable server {} ({}) didn't respond. Error: {}", key, server.name, e);
                        }
                        print_server_statuses(&servers_lock);
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
        {
            // Print server statuses after failure to find a server
            let servers_lock = servers.lock().unwrap();
            print_server_statuses(&servers_lock);
        }
        let response = Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(Body::from("No available servers"))
            .unwrap();
        Ok(response)
    }
}

async fn select_available_server(servers: &SharedServerList, remote_addr: &std::net::SocketAddr) -> Option<String> {
    let mut servers_lock = servers.lock().unwrap();

    // Define the closure to encapsulate server selection logic
    let mut select_server = || {
        // 1st choice: Find an available reliable server
        for (key, server) in servers_lock.iter_mut() {
            if matches!(server.state.failure_record, FailureRecord::Reliable) && !server.state.busy {
                server.state.busy = true;
                println!("🤖🦸 Chose reliable server: {} ({}) to serve client {}", key, server.name, remote_addr);
                return Some(key.clone());
            }
        }

        // 2nd choice: If no reliable servers are available, select an untrusted available server that has
        // only failed once in a row.
        for (key, server) in servers_lock.iter_mut() {
            if matches!(server.state.failure_record, FailureRecord::Unreliable) && !server.state.busy {
                server.state.busy = true;
                println!("🤖😇 Giving server {} ({}) another chance with client {}", key, server.name, remote_addr);
                return Some(key.clone());
            }
        }

        // If all untrusted available servers have been given a second chance,
        // reset the SecondChanceGiven mark so that we can again cycle through the untrusted servers-
        // This ensures that we cycle equally through all untrusted servers- give everyone
        // their chance
        for server in servers_lock.values_mut() {
            if matches!(server.state.failure_record, FailureRecord::SecondChanceGiven) && !server.state.busy {
                server.state.failure_record = FailureRecord::Unreliable;
            }
        }

        // 3rd choice: Select any untrusted server, because we're out of options at this point
        for (key, server) in servers_lock.iter_mut() {
            if matches!(server.state.failure_record, FailureRecord::Unreliable) && !server.state.busy {
                server.state.busy = true;
                println!("🤖😇 Giving server {} ({}) a 3rd+ chance with client {}", key, server.name, remote_addr);
                return Some(key.clone());
            }
        }

        // No servers available
        None
    };

    // Capture the result of the closure
    let selected_server = select_server();

    print_server_statuses(&servers_lock);

    selected_server
}

struct ServerGuard {
    servers: SharedServerList,
    key: String,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let mut servers_lock = self.servers.lock().unwrap();
        if let Some(server) = servers_lock.get_mut(&self.key) {
            server.state.busy = false;
            if matches!(server.state.failure_record, FailureRecord::Reliable) {
                println!("🟢 Server {} ({}) now available", self.key, server.name);
            }
            else {
                println!("⚠️  Connection closed with Unreliable Server {} ({})", self.key, server.name);
            }
            print_server_statuses(&servers_lock);
        }
    }
}

// Custom stream that holds the guard
struct ResponseBodyWithGuard<S> {
    stream: S,
    _guard: ServerGuard,
    servers: SharedServerList,
    key: String,
    had_error: bool,
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
            Poll::Ready(Some(Err(e))) => {
                // An error occurred during streaming
                self.had_error = true; // Mark that an error has occurred
                {
                    let mut servers_lock = self.servers.lock().unwrap();
                    if let Some(server) = servers_lock.get_mut(&self.key) {
                        if matches!(server.state.failure_record, FailureRecord::Reliable) {
                            server.state.failure_record = FailureRecord::Unreliable;
                            println!("⛔😱 Server {} ({}) failed during streaming, now marked Unreliable. Error: {}", self.key, server.name, e);
                        }
                        else {
                            server.state.failure_record = FailureRecord::SecondChanceGiven;
                            println!("⛔😞 Unreliable server {} ({}) failed during streaming. Error: {}", self.key, server.name, e);
                        }
                        print_server_statuses(&servers_lock);
                    }
                }
                // Return the error to the client
                Poll::Ready(Some(Err(std::io::Error::new(std::io::ErrorKind::Other, e))))
            },
            Poll::Ready(None) => {
                if !self.had_error {
                    // Streaming ended successfully
                    // Mark the server as Reliable
                    let mut servers_lock = self.servers.lock().unwrap();
                    if let Some(server) = servers_lock.get_mut(&self.key) {
                        if !matches!(server.state.failure_record, FailureRecord::Reliable) {
                            server.state.failure_record = FailureRecord::Reliable;
                            println!("🙏⚕️  Server {} ({}) has completed streaming successfully and is now marked Reliable", self.key, server.name);
                            print_server_statuses(&servers_lock);
                        }
                    }
                }
                Poll::Ready(None)
            },
            Poll::Pending => Poll::Pending,
        }
    }
}

// Required because two different versions of crate `http` are being used
// reqwest is a new version, hyper is an old version and the new API is completely
// different so for now I chose to stay with the old version of hyper.
fn hyper_method_to_reqwest_method(method: hyper::Method) -> Result<reqwest::Method, Box<dyn std::error::Error>> {
    return Ok(method.as_str().parse::<reqwest::Method>()?);
}

/// Prints a nicely formatted list of the servers, their name, busy status, and reliability.
fn print_server_statuses(servers: &OrderMap<String, OllamaServer>) {
    println!("🗒  Current server statuses:");
    for (i, (address, srv)) in servers.iter().enumerate() {
        let busy_status = if srv.state.busy { "Busy" } else { "Available" };
        let reliability = match srv.state.failure_record {
            FailureRecord::Reliable => "Reliable",
            FailureRecord::Unreliable => "Unreliable",
            FailureRecord::SecondChanceGiven => "SecondChanceGiven",
        };
        println!(
            "{}. Address: {} ({}), Busy: {}, Reliability: {}",
            i + 1,
            address,
            srv.name,
            busy_status,
            reliability
        );
    }
    println!("");
}
