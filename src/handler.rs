use crate::state::{
    mark_server_more_healthy, mark_server_less_healthy,
    print_server_statuses, select_servers, snapshot_servers, sync_server,
    FailureRecord, SelOpt, SharedServerList
};
use crate::backend::{UnpackedRequest, ReqOpt, send_request_monitored, send_request};
use hyper::{Body, Request, Response, StatusCode};
use serde_json::Value;
use std::collections::HashMap;
use std::convert::Infallible;
use reqwest;
use futures_util::stream::StreamExt;
use futures_util::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
use futures_util::future;
use hyper::body;
use tokio;
use serde_json::json;
use tracing::{info, warn, error};

/// Required because two different versions of crate `http` are being used
/// reqwest is a new version, hyper is an old version and the new API is completely
/// different so for now I chose to stay with the old version of hyper.
pub fn hyper_method_to_reqwest_method(method: hyper::Method) -> Result<reqwest::Method, Box<dyn std::error::Error>> {
    return Ok(method.as_str().parse::<reqwest::Method>()?);
}

async fn unpack_req(mut req: Request<Body>) -> Result<UnpackedRequest, Box<dyn std::error::Error>> {
    let uri = req.uri().to_string();
    let whole_body = body::to_bytes(req.body_mut()).await.unwrap_or_default();
    let req_method = match hyper_method_to_reqwest_method(req.method().clone()) {
        Ok(m) => m,
        Err(e) => {
            return Err(e.into());
        }
    };
    let path = req.uri().path().to_string();
    let headers = req.headers().clone();

    Ok((uri, req_method, path, Some(headers), Some(whole_body)))
}

fn parse_body(body: &bytes::Bytes) -> Result<Value, Box<dyn std::error::Error>> {
    let body = body.to_vec();
    let body = String::from_utf8(body)?;
    let body = serde_json::from_str(&body)?;
    Ok(body)
}

fn make_json_resp(
    status: StatusCode,
    body: Value,
) -> Response<Body> {
    let json_body = serde_json::to_string(&body).unwrap();
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(json_body))
        .unwrap()
}

pub async fn dispatch(
    req: Request<Body>,
    servers: SharedServerList,
    remote_addr: std::net::SocketAddr,
    opts: ReqOpt,
) -> Result<Response<Body>, Infallible> {
    let path = req.uri().path().to_string();
    let remote = remote_addr.to_string();
    let method = req.method().to_string();
    let response = match path.as_str() {
        "/" => Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Body::from("Ollama is running"))
            .unwrap()
        ),
        "/api/tags" => handle_tags(req, servers, remote_addr).await,
        "/api/show" => handle_request_ha(req, servers, remote_addr, opts).await,
        "/api/generate" => handle_generate(req, servers, remote_addr).await,
        "/api/chat" => handle_chat_parallel(req, servers, remote_addr, opts).await,
        _ => handle_return_501(req, servers, remote_addr, format!("Endpoint {} is not implemented", path).as_str()).await,
    };
    let status = response.as_ref().map(|r| r.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    info!("{} - {} {} - {} {}", remote, method, path, status.as_u16(), status.canonical_reason().unwrap_or("Unknown"));
    response
}

// Handle request with high availability
pub async fn handle_request_ha(
    req: Request<Body>,
    servers: SharedServerList,
    remote_addr: std::net::SocketAddr,
    opts: ReqOpt,
) -> Result<Response<Body>, Infallible> {
    let unpacked_req = match unpack_req(req).await {
        Ok(req) => req,
        Err(e) => {
            return Ok(make_json_resp(StatusCode::BAD_REQUEST, json!({ "error": format!("Error handling request: {}", e) })));
        }
    };

    let body = match parse_body(unpacked_req.4.as_ref().unwrap()) {
        Ok(body) => body,
        Err(e) => {
            return Ok(make_json_resp(StatusCode::BAD_REQUEST, json!({ "error": format!("Error parsing request body: {}", e) })));
        }
    };
    let model = body["model"].as_str().unwrap_or_default();
    let model = if model.is_empty() {
        body["name"].as_str().unwrap_or_default()
    } else {
        model
    };
    if model.is_empty() {
        return Ok(make_json_resp(StatusCode::BAD_REQUEST, json!({ "error": "Request body must contain a 'model' field" })));
    }
    let selected_keys = select_servers(servers.clone(), model.to_string(), SelOpt {
        count: (3, 6),
        resurrect_p: 0.1,
        resurrect_n: 1,
    });
    if selected_keys.is_empty() {
        return Ok(make_json_resp(StatusCode::SERVICE_UNAVAILABLE, json!({ "error": "No available servers" })));
    }

    for server_url in selected_keys {
        match send_request(unpacked_req.clone(), &server_url, opts.timeout).await {
            Ok(response) => {
                info!("Chosen server {} to serve client {}", server_url, remote_addr);
                let status = response.status();
                let mut resp_builder = Response::builder().status(u16::from(status));
                for (key_h, value) in response.headers() {
                    resp_builder = resp_builder.header(key_h.to_string(), value.to_str().unwrap());
                }
                let stream = response.bytes_stream().boxed();
                return Ok(resp_builder.body(Body::wrap_stream(stream)).unwrap());
            },
            Err(e) => {
                warn!("Sequential request to server {} failed: {:?}", server_url, e);
                continue;
            }
        }
    }
    Ok(make_json_resp(StatusCode::SERVICE_UNAVAILABLE, json!({ "error": "All chosen backends failed" })))
}

pub async fn handle_chat_parallel(
    req: Request<Body>,
    servers: SharedServerList,
    remote_addr: std::net::SocketAddr,
    opts: ReqOpt,
) -> Result<Response<Body>, Infallible> {
    let unpacked_req = match unpack_req(req).await {
        Ok(req) => req,
        Err(e) => {
            return Ok(make_json_resp(StatusCode::BAD_REQUEST, json!({ "error": format!("Error handling request: {}", e) })));
        }
    };

    let body = match parse_body(unpacked_req.4.as_ref().unwrap()) {
        Ok(body) => body,
        Err(e) => {
            return Ok(make_json_resp(StatusCode::BAD_REQUEST, json!({ "error": format!("Error parsing request body: {}", e) })));
        }
    };
    let model = match body["model"].as_str() {
        Some(model) => model,
        None => {
            return Ok(make_json_resp(StatusCode::BAD_REQUEST, json!({ "error": "Request body must contain a 'model' field" })));
        }
    };
    let selected_keys = select_servers(servers.clone(), model.to_string(), SelOpt {
        count: (3, 6),
        resurrect_p: 0.1,
        resurrect_n: 1,
    });
    if selected_keys.is_empty() {
        return Ok(make_json_resp(StatusCode::SERVICE_UNAVAILABLE, json!({ "error": "No available servers" })));
    }

    let tasks: Vec<_> = selected_keys.iter().map(|server_url| {
        let req = unpacked_req.clone();
        let url = server_url.clone();
        let servers = servers.clone();
        tokio::spawn(async move {
            let health = sync_server(servers, url.to_owned(), opts.timeout).await;
            if health == crate::state::Health::Dead {
                warn!("Server {} is dead", url);
                return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                    std::io::Error::new(std::io::ErrorKind::Other, format!("Server {} is dead", url))
                ));
            }
            info!("Server {} is healthy", url);
            send_request_monitored(req, url.as_str(), opts).await
        })
    }).collect();

    let results = future::join_all(tasks).await;
    // firstly, partition the results into successful and failed
    let (ok_results, failed_results): (Vec<_>, Vec<_>) = 
        results.into_iter().zip(selected_keys).partition(|res_server|
        if let (Ok(Ok((_perf, repacked))), _) = res_server {
            repacked.status.is_success()
        } else {
            false
        }
    );

    if failed_results.len() > 0 {
        warn!("{} parallel requests failed", failed_results.len());
        // log failed requests & mark less healthy asynchrously
        let servers = servers.clone();
        tokio::spawn(async move {
            for (res, server) in failed_results {
                mark_server_less_healthy(servers.clone(), &server);
                match res {
                    Err(e) => {
                        warn!("Parallel request failed: {:?}", e);
                    },
                    Ok(Err(e)) => {
                        warn!("Parallel request failed: {:?}", e);
                    },
                    Ok(Ok((perf, repacked))) => {
                        warn!("Parallel request failed: Performance: {:?}, Response: {:?}", perf, repacked.into_string().await);
                    },
                }
            }
        });
    }

    let ok_servers = ok_results.iter().map(|res_server| res_server.1.clone()).collect::<Vec<String>>();
    let best = ok_results.into_iter().filter_map(|res_server|
        if let Ok(Ok((perf, repacked))) = res_server.0 {
            Some((perf, repacked, res_server.1))
        } else {
            None
        }
    ).max_by_key(|(perf, _, _)| perf.duration_tokens);
    
    if let Some((_, resp, best_server)) = best {
        // mark more healthy asynchronously
        let best_server_clone = best_server.clone();
        let servers = servers.clone();
        tokio::spawn(async move {
            mark_server_more_healthy(servers.clone(), &best_server_clone, true);
            for server in ok_servers {
                if server != best_server_clone {
                    mark_server_more_healthy(servers.clone(), &server, false);
                }
            }
        });

        info!("Chosen server {} to serve client {}", best_server, remote_addr);
        let mut resp_builder = Response::builder().status(u16::from(resp.status));
        for (k, v) in resp.headers.iter() {
            resp_builder = resp_builder.header(k.to_string(), v.to_str().unwrap());
        }
        let hyper_body = Body::wrap_stream(resp.stream);
        let response = resp_builder.body(hyper_body).unwrap();
        Ok(response)
    } else {
        Ok(make_json_resp(StatusCode::SERVICE_UNAVAILABLE, json!({ "error": "All parallel requests failed" })))
    }
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
                info!("Server {} ({}) is now available", self.key, server.name);
            }
            else {
                info!("Connection closed with unreliable server {} ({})", self.key, server.name);
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
                            error!("Server {} ({}) failed during streaming, now marked Unreliable. Error: {}", self.key, server.name, e);
                        }
                        else {
                            server.state.failure_record = FailureRecord::SecondChanceGiven;
                            error!("Unreliable server {} ({}) failed during streaming. Error: {}", self.key, server.name, e);
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
                            info!("Server {} ({}) completed streaming successfully and is now marked Reliable", self.key, server.name);
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

pub async fn handle_tags(
    _req: Request<Body>,
    servers: SharedServerList,
    _remote_addr: std::net::SocketAddr,
) -> Result<Response<Body>, Infallible> {
    let snaps = snapshot_servers(servers, true);
    let mut merged_models = HashMap::new();
    for snap in snaps.values() {
        info!("Server {} has {} models", snap.name, snap.models.len());
        merged_models.extend(snap.models.clone());
    }
    info!("Total models: {}", merged_models.len());
    // collect all model details
    let models: Vec<Value> = merged_models.into_iter().map(|(_name, model)|
        model.unwrap().detail
    ).collect();
    return Ok(make_json_resp(StatusCode::OK, json!({ "models": models })));
}

pub async fn handle_generate(
    req: Request<Body>,
    _servers: SharedServerList,
    _remote_addr: std::net::SocketAddr,
) -> Result<Response<Body>, Infallible> {
    let unpacked_req = match unpack_req(req).await {
        Ok(req) => req,
        Err(e) => {
            return Ok(make_json_resp(StatusCode::BAD_REQUEST, json!({ "error": format!("Error handling request: {}", e) })));
        }
    };
    let body_bytes = unpacked_req.4.as_ref().unwrap();
    let body = match parse_body(body_bytes) {
        Ok(body) => body,
        Err(e) => {
            return Ok(make_json_resp(StatusCode::BAD_REQUEST, json!({ "error": format!("Error parsing request body: {}", e) })));
        }
    };

    let resp_501: fn(&Value, &str) -> Result<Response<Body>, Infallible> = |body, msg| {
        error!("Invalid request body: {}", body);
        Ok(make_json_resp(StatusCode::NOT_IMPLEMENTED, json!({ "error": msg })))
    };

    if !body.is_object() {
        return resp_501(&body, "Request body must be a JSON object");
    }
    let map = body.as_object().unwrap();
    if !map.contains_key("model") || !map.contains_key("prompt") {
        return resp_501(&body, "Request body must contain 'model' and 'prompt' fields");
    }
    let model = map.get("model").unwrap().as_str().unwrap();
    let prompt = map.get("prompt").unwrap().as_str().unwrap();
    // currently only empty prompt is supported
    if !prompt.is_empty() {
        return resp_501(&body, "Non-empty 'prompt' field is not supported yet");
    }

    let res = json!({
        "model": model,
    });
    Ok(make_json_resp(StatusCode::OK, res))
}

pub async fn handle_return_501(
    _req: Request<Body>,
    _servers: SharedServerList,
    _remote_addr: std::net::SocketAddr,
    msg: &str,
) -> Result<Response<Body>, Infallible> {
    Ok(make_json_resp(StatusCode::NOT_IMPLEMENTED, json!({ "error": msg })))
}