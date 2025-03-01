use std::time::{Duration, Instant};
use reqwest::{StatusCode, Method, Client, header::HeaderMap};
use hyper;
use futures_util::stream::StreamExt;
use futures_util::Stream;
use std::pin::Pin;

/// Runtime options for the backend request.
#[derive(Clone, Copy)]
pub struct ReqOpt {
    pub timeout_load: u32,
    pub t0: u32,
    pub t1: u32,
}
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

pub type UnpackedRequest = (String, Method, String, hyper::HeaderMap, bytes::Bytes);

pub async fn send_request_monitored(
    req: UnpackedRequest,
    backend_url: &str,
    opts: ReqOpt,
) -> Result<(PerformanceInfo, RepackedResponse), Box<dyn std::error::Error + Send + Sync>> {
    let (uri, req_method, path, headers, whole_body) = req;
    let uri = format!("{}{}", backend_url, uri);

    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(1));
    if opts.t0 == 0 {
        builder = builder.pool_idle_timeout(None);
    } else {
        let timeout = Duration::from_secs(opts.t0.into());
        builder = builder.read_timeout(timeout).pool_idle_timeout(timeout);
    }
    let client = builder.build().unwrap();
    let mut request_builder = client.request(req_method, &uri);
    for (k, v) in headers.iter() {
        request_builder = request_builder.header(k.as_str(), v.to_str().unwrap());
    }
    request_builder = request_builder.body(whole_body);

    let begin_time = Instant::now();
    let t0 = Duration::from_secs(opts.t0.into());
    let t1 = Duration::from_secs(opts.t1.into());
    let response = match request_builder.send().await {
        Ok(resp) => resp,
        Err(e) => {
            println!("Error sending request to {}: {}", backend_url, e);
            return Err(e.into());
        }
    };
    let status = response.status();
    let resp_headers = response.headers().clone();
    let mut stream = response.bytes_stream().boxed();
    let mut buffer = Vec::new();
    let mut bytes_count = 0;
    let mut ftt: Option<Instant> = None;
    loop {
        let res = stream.next().await;
        let elapsed = begin_time.elapsed();
        match res {
            Some(Ok(chunk)) => {
                buffer.extend_from_slice(&chunk);
                if ftt.is_none() {
                    ftt = Some(begin_time + elapsed);
                }
                if elapsed >= t1 {
                    break;
                }
                if elapsed > t0 {
                    bytes_count += chunk.len();
                }
            },
            Some(Err(e)) => {
                println!("Error reading chunk from {}: {}", backend_url, e);
                break;
            },
            None => break,
        }
    }
    println!("time: [{:?}] Server {} received {} bytes in {} seconds", Instant::now(), backend_url, bytes_count, t1.as_secs());
    let buf_stream = futures_util::stream::iter(vec![Ok(bytes::Bytes::from(buffer))]);
    stream = buf_stream.chain(stream).boxed();
    
    let perf = PerformanceInfo {
        first_token_time: ftt.unwrap(),
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
) -> Result<reqwest::Response, Box<dyn std::error::Error>> {
    let (uri, req_method, path, headers, whole_body) = req;
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

    for (key_h, value) in headers.iter() {
        request_builder = request_builder.header(key_h.as_str(), value.as_bytes());
    }
    request_builder = request_builder.body(whole_body);

    let response = request_builder.send().await?;
    Ok(response)
}