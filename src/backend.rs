use std::time::{Duration, Instant};
use reqwest::{StatusCode, Method, Client, header::HeaderMap};
use hyper;
use futures_util::stream::StreamExt;
use futures_util::Stream;
use std::pin::Pin;
use tracing::{info, error};

/// Runtime options for the backend request.
#[derive(Clone, Copy, Debug)]
pub struct ReqOpt {
    pub timeout: u32,
    pub timeout_ft: u32,
    pub time_measure: u32,
}
#[derive(Debug)]
pub struct PerformanceInfo {
    pub first_token_time: Instant,
    // TODO: we can't use token/s because float is not supported by max_by_key
    pub duration_tokens: usize,
}

pub struct RepackedResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub stream: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
}

impl RepackedResponse {
    pub async fn into_string(self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let max_preview = 100;
        let mut body = String::new();
        let mut stream = self.stream;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            body.push_str(&String::from_utf8_lossy(&chunk));
            if body.len() > max_preview {
                body.push_str("...");
                break;
            }
        }
        Ok(format!("RepackedResponse {{ status: {:?}, headers: {:?}, body: \"{}\" }}", self.status, self.headers, body))
    }
}

pub type UnpackedRequest = (String, Method, String, Option<hyper::HeaderMap>, Option<bytes::Bytes>);

pub async fn send_request_monitored(
    req: UnpackedRequest,
    backend_url: &str,
    opts: ReqOpt,
) -> Result<(PerformanceInfo, RepackedResponse), Box<dyn std::error::Error + Send + Sync>> {
    let (uri, req_method, _path, headers, whole_body) = req;
    let uri = format!("{}{}", backend_url, uri);

    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(opts.timeout.into()));
    if opts.timeout_ft == 0 {
        builder = builder.pool_idle_timeout(None);
    } else {
        let timeout = Duration::from_secs(opts.timeout_ft.into());
        builder = builder.read_timeout(timeout).pool_idle_timeout(timeout);
    }
    let client = builder.build().unwrap();
    let mut request_builder = client.request(req_method, &uri);
    if let Some(headers) = headers {
        for (k, v) in headers.iter() {
            request_builder = request_builder.header(k.as_str(), v.to_str().unwrap());
        }
    }
    if let Some(whole_body) = whole_body {
        request_builder = request_builder.body(whole_body);
    }

    let response = match request_builder.send().await {
        Ok(resp) => resp,
        Err(e) => {
            error!("Error sending request to {}: {}", backend_url, e);
            return Err(e.into());
        }
    };
    let status = response.status();
    let resp_headers = response.headers().clone();
    let mut stream = response.bytes_stream().boxed();
    let mut buffer = Vec::new();
    let mut bytes_count = 0;
    let mut ftt: Option<Instant> = None;
    let t_measure = Duration::from_secs(opts.time_measure.into());
    loop {
        let res = stream.next().await;
        let now = Instant::now();
        match res {
            Some(Ok(chunk)) => {
                buffer.extend_from_slice(&chunk);
                bytes_count += chunk.len();
                match ftt {
                    None => {
                        ftt = Some(now);
                    }
                    Some(ftt) => {
                        if now.duration_since(ftt) > t_measure {
                            break;
                        }
                    },
                }
            },
            Some(Err(e)) => {
                error!("Error reading chunk from {}: {}", backend_url, e);
                break;
            },
            None => break,
        }
    }
    let ftt = match ftt {
        None => {
            return Err("No data received from backend".into());
        },
        Some(ftt) => ftt,
    };

    info!("Backend {} received {} bytes in {} seconds", backend_url, bytes_count, ftt.elapsed().as_secs_f32());
    let buf_stream = futures_util::stream::iter(vec![Ok(bytes::Bytes::from(buffer))]);
    stream = buf_stream.chain(stream).boxed();
    
    let perf = PerformanceInfo {
        first_token_time: ftt,
        duration_tokens: bytes_count,
    };
    let repacked = RepackedResponse {
        status,
        headers: resp_headers,
        stream,
    };
    Ok((perf, repacked))
}

pub async fn send_request(
    req: UnpackedRequest,
    backend_url: &str,
    timeout_secs: u32,
) -> Result<reqwest::Response, Box<dyn std::error::Error + Send + Sync>> {
    let (uri, req_method, _path, headers, whole_body) = req;
    let uri = format!("{}{}", backend_url, uri);

    let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(1));
    if timeout_secs == 0 {
        builder = builder.pool_idle_timeout(None);
    } else {
        let timeout = Duration::from_secs(timeout_secs.into());
        builder = builder.read_timeout(timeout).pool_idle_timeout(timeout);
    }
    let client = builder.build()?;
    let mut request_builder = client.request(req_method, &uri);

    if let Some(headers) = headers {
        for (k, v) in headers.iter() {
            request_builder = request_builder.header(k.as_str(), v.to_str().unwrap());
        }
    }
    if let Some(whole_body) = whole_body {
        request_builder = request_builder.body(whole_body);
    }

    let response = request_builder.send().await?;
    Ok(response)
}