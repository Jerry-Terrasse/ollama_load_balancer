use crate::state::ModelConfig;
use reqwest::Method;

use crate::backend::send_request;

pub async fn api_tags(
    backend_url: &str, timeout_secs: u32
) -> Result<Vec<ModelConfig>, Box<dyn std::error::Error + Send + Sync>> {
    let uri = "/api/tags";
    let res = send_request(
        (uri.to_string(), Method::GET, uri.to_string(), None, None),
        backend_url, timeout_secs
    ).await?;

    let data = res.json::<serde_json::Value>().await?;
    let models = data["models"].as_array().unwrap().iter().map(|m| {
        let name = m["name"].as_str().unwrap().to_string();
        let detail = m.clone();
        ModelConfig { name, detail }
    }).collect();
    Ok(models)
}

pub async fn api_ps(
    backend_url: &str, timeout_secs: u32
) -> Result<Vec<ModelConfig>, Box<dyn std::error::Error + Send + Sync>> {
    let uri = "/api/ps";
    let res = send_request(
        (uri.to_string(), Method::GET, uri.to_string(), None, None),
        backend_url, timeout_secs
    ).await?;

    let data = res.json::<serde_json::Value>().await?;
    let models = data["models"].as_array().unwrap().iter().map(|m| {
        let name = m["name"].as_str().unwrap().to_string();
        let detail = m.clone();
        ModelConfig { name, detail }
    }).collect();
    Ok(models)
}