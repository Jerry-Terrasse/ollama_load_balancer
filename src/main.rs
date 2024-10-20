use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};

use std::convert::Infallible;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::RwLock;
use futures_util::stream::StreamExt;

#[derive(Clone, Debug)]
struct OllamaServer {
    address: String,
    available: Arc<AtomicBool>,
}

type SharedServerList = Arc<RwLock<Vec<Arc<OllamaServer>>>>;

#[tokio::main]
async fn main() {
    // Initialize the list of Ollama servers
    let servers = vec![
        Arc::new(OllamaServer {
            address: "http://192.168.150.134:11434".to_string(),
            available: Arc::new(AtomicBool::new(true)),
        }),
        Arc::new(OllamaServer {
            address: "http://192.168.150.135:11434".to_string(),
            available: Arc::new(AtomicBool::new(true)),
        }),
        Arc::new(OllamaServer {
            address: "http://192.168.150.136:11434".to_string(),
            available: Arc::new(AtomicBool::new(true)),
        }),
        // Add more servers as needed
    ];

    let servers = Arc::new(RwLock::new(servers));

    let make_svc = make_service_fn(|_conn| {
        let servers = servers.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let servers = servers.clone();
                handle_request(req, servers)
            }))
        }
    });

    let addr = ([0, 0, 0, 0], 11434).into();

    let server = Server::bind(&addr).serve(make_svc);

    // Implement graceful shutdown
    let graceful = server.with_graceful_shutdown(shutdown_signal());

    println!("Ollama Load Balancer listening on http://{}", addr);

    if let Err(e) = graceful.await {
        eprintln!("Server error: {}", e);
    }
}

async fn shutdown_signal() {
    // Wait for the CTRL+C signal
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl_c");

    println!("Received CTRL+C, shutting down gracefully...");
    // The future returned by ctrl_c() will resolve when CTRL+C is pressed
    // Hyper will then stop accepting new connections
}

async fn handle_request(
    req: Request<Body>,
    servers: SharedServerList,
) -> Result<Response<Body>, Infallible> {
    // Only handle POST requests
    if req.method() != Method::POST {
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Body::from("Only POST requests are allowed"))
            .unwrap());
    }

    // Get the path
    let path = req.uri().path();

    // Check if the path matches /api/chat or /api/generate
    if path != "/api/chat" && path != "/api/generate" {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Not Found"))
            .unwrap());
    }

    // Print the complete contents of servers before selecting one
    {
        let servers_read = servers.read().await;
        println!("Available servers: {:?}", servers_read);
    }
    // Select an available server
    let server = select_available_server(&servers).await;

    if let Some(server) = server {
        println!("Chose server: {}", server.address);
        // Ensure availability is reset when the request handling is complete
        let _guard = ServerGuard {
            server: server.clone(),
        };
        // TODO: This _guard seems to go out of scope too soon.
        // This causes multiple requests to be pushed towards the first server.

        // Build the request to the Ollama server
        let uri = format!("{}{}", server.address, path);

        let client = reqwest::Client::new();
        let mut request_builder = client.request(req.method().clone(), &uri);

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
                let status = response.status();
                let mut resp_builder = Response::builder().status(status);

                // Copy headers
                for (key, value) in response.headers() {
                    resp_builder = resp_builder.header(key, value.clone());
                }

                // Get the response body as a stream
                let resp_body = response.bytes_stream().map(|result| match result {
                    Ok(bytes) => Ok(bytes),
                    Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
                });

                // Convert reqwest::stream::BytesStream to hyper::Body
                let hyper_body = Body::wrap_stream(resp_body);

                let response = resp_builder.body(hyper_body).unwrap();

                Ok(response)
            }
            Err(e) => {
                // Return an error to the client
                let response = Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Body::from(format!("Error connecting to Ollama server: {}", e)))
                    .unwrap();
                Ok(response)
            }
        }
    } else {
        // No available servers
        let response = Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(Body::from("No available servers"))
            .unwrap();
        Ok(response)
    }
}

async fn select_available_server(servers: &SharedServerList) -> Option<Arc<OllamaServer>> {
    let servers = servers.read().await;
    for server in servers.iter() {
        match server
            .available
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(previous) => {
                // If compare_exchange was successful, and the previous value was `true`
                if previous {
                    return Some(server.clone());
                }
            }
            Err(_) => {
                // If compare_exchange failed, we simply continue to the next server
                // No additional logic is required here
            }
        }
    }
    None
}

struct ServerGuard {
    server: Arc<OllamaServer>,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        self.server.available.store(true, Ordering::SeqCst);
    }
}
