use crate::state::{SharedServerList, print_server_statuses, FailureRecord};
use hyper::{Body, Request, Response, StatusCode};
use std::convert::Infallible;
use reqwest;
use futures_util::stream::StreamExt;
use futures_util::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
use futures_util::future;
use hyper::body;
use tokio;

/// Required because two different versions of crate `http` are being used
/// reqwest is a new version, hyper is an old version and the new API is completely
/// different so for now I chose to stay with the old version of hyper.
pub fn hyper_method_to_reqwest_method(method: hyper::Method) -> Result<reqwest::Method, Box<dyn std::error::Error>> {
    return Ok(method.as_str().parse::<reqwest::Method>()?);
}

pub async fn handle_request(
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

        let is_chat = path == "/api/chat";

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

        let begin_time = std::time::Instant::now();
        let count_interval = std::time::Duration::from_secs(3);

        // Send the request and handle the response
        match request_builder.send().await {
            Ok(response) => {
                let status = response.status();
                let mut resp_builder = Response::builder().status(u16::from(status));

                // Copy headers
                for (key_h, value) in response.headers() {
                    resp_builder = resp_builder.header(key_h.to_string(), value.to_str().unwrap());
                }

                let mut stream = response.bytes_stream().boxed();
                if is_chat {
                    let mut bytes_count = 0;
                    let mut buffer = Vec::new();
                    while begin_time.elapsed() < count_interval {
                        match stream.next().await {
                            Some(Ok(chunk)) => {
                                bytes_count += chunk.len();
                                buffer.extend_from_slice(&chunk);
                            }
                            Some(Err(e)) => {
                                println!("Error reading chunk: {}", e);
                                break;
                            }
                            None => {
                                break;
                            }
                        }
                    }

                    println!("Number of bytes received in {} seconds: {}", count_interval.as_secs(), bytes_count);

                    // Recover the stream
                    let buf_stream = futures_util::stream::iter(vec![Ok(bytes::Bytes::from(buffer))]);
                    stream = buf_stream.chain(stream).boxed();
                }

                // Wrap the response body stream with our custom stream.
                // The purpose of our custom stream as opposed to directly using response.bytes_stream()
                // is so we can keep track of the stream lifetime- to mark the server as available once again.
                let resp_body = ResponseBodyWithGuard {
                    stream,
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

pub async fn handle_request_parallel(
    mut req: Request<Body>,
    servers: SharedServerList,
    remote_addr: std::net::SocketAddr,
    timeout_secs: u32,
) -> Result<Response<Body>, Infallible> {
    let whole_body = body::to_bytes(req.body_mut()).await.unwrap_or_default();
    let req_method = match hyper_method_to_reqwest_method(req.method().clone()) {
        Ok(m) => m,
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .body(Body::from(format!("hyper_method_to_reqwest_method failed: {}", e)))
                .unwrap())
        }
    };
    let path = req.uri().path().to_string();
    let headers = req.headers().clone();

    let mut selected_keys = Vec::new();
    for _ in 0..3 {
        if let Some(key) = select_available_server(&servers, &remote_addr).await {
            selected_keys.push(key);
        }
    }
    if selected_keys.is_empty() {
        let response = Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(Body::from("No available servers"))
            .unwrap();
        return Ok(response);
    }

    let count_interval = std::time::Duration::from_secs(3);
    let tasks: Vec<_> = selected_keys.into_iter().map(|server_url| {
        let method = req_method.clone();
        let path_clone = path.clone();
        let headers_clone = headers.clone();
        let body_bytes = whole_body.clone();
        let servers_clone = servers.clone();
        let remote_addr_clone = remote_addr.clone();
        tokio::spawn(async move {
            let uri = format!("{}{}", server_url, path_clone);
            let mut builder = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(1));
            if timeout_secs == 0 {
                builder = builder.pool_idle_timeout(None);
            } else {
                let timeout = std::time::Duration::from_secs(timeout_secs.into());
                builder = builder.read_timeout(timeout).pool_idle_timeout(timeout);
            }
            let client = builder.build().unwrap();
            let mut request_builder = client.request(method, &uri);
            for (k, v) in headers_clone.iter() {
                request_builder = request_builder.header(k.as_str(), v.as_bytes());
            }
            request_builder = request_builder.body(body_bytes);
            
            let begin_time = std::time::Instant::now();
            let response = match request_builder.send().await {
                Ok(resp) => resp,
                Err(e) => {
                    println!("Error sending request to {}: {}", server_url, e);
                    return None;
                }
            };
            let status = response.status();
            let resp_headers = response.headers().clone();
            let mut stream = response.bytes_stream().boxed();
            let mut bytes_count = 0;
            let mut buffer = Vec::new();
            while begin_time.elapsed() < count_interval {
                match stream.next().await {
                    Some(Ok(chunk)) => {
                        bytes_count += chunk.len();
                        buffer.extend_from_slice(&chunk);
                    },
                    Some(Err(e)) => {
                        println!("Error reading chunk from {}: {}", server_url, e);
                        break;
                    },
                    None => break,
                }
            }
            println!("Server {} received {} bytes in {} seconds", server_url, bytes_count, count_interval.as_secs());
            let buf_stream = futures_util::stream::iter(vec![Ok(bytes::Bytes::from(buffer))]);
            stream = buf_stream.chain(stream).boxed();
            let guard = ServerGuard {
                servers: servers_clone,
                key: server_url.clone(),
            };
            Some((bytes_count, status, resp_headers, stream, guard))
        })
    }).collect();

    let results = future::join_all(tasks).await;
    let best = results.into_iter().filter_map(|res|
        if let Ok(Some((cnt, status, headers, stream, guard))) = res {
            Some((cnt, status, headers, stream, guard))
        } else {
            None
        }
    ).max_by_key(|(cnt, _, _, _, _)| *cnt);

    if let Some((_, status, resp_headers, stream, _guard)) = best {
        let mut resp_builder = Response::builder().status(u16::from(status));
        for (k, v) in resp_headers.iter() {
            resp_builder = resp_builder.header(k.to_string(), v.to_str().unwrap());
        }
        let hyper_body = Body::wrap_stream(stream);
        let response = resp_builder.body(hyper_body).unwrap();
        Ok(response)
    } else {
        let response = Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .body(Body::from("All parallel requests failed"))
            .unwrap();
        Ok(response)
    }
}

pub async fn select_available_server(servers: &SharedServerList, remote_addr: &std::net::SocketAddr) -> Option<String> {
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

pub struct ServerGuard {
    pub servers: SharedServerList,
    pub key: String,
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
pub struct ResponseBodyWithGuard<S> {
    pub stream: S,
    pub _guard: ServerGuard,
    pub servers: SharedServerList,
    pub key: String,
    pub had_error: bool,
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
