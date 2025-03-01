use ordermap::OrderMap;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use serde_json::Value;

use crate::config::ServerConfig;
use crate::api::{api_tags, api_ps};

#[derive(Clone, Debug)]
pub enum FailureRecord {
    Reliable,
    Unreliable,
    SecondChanceGiven,
}

#[derive(Clone, Debug)]
pub enum Health {
    Dead,
    Healthy(f32),
}

#[derive(Debug)]
pub struct ServerState {
    pub busy: bool,
    pub health: Health, // default to 1.0, max 100.0
    pub failure_record: FailureRecord,
}

#[derive(Debug)]
pub struct OllamaServer {
    pub state: ServerState,
    pub name: String,
    pub models: HashMap<String, ModelConfig>,
    pub actives: HashMap<String, ModelConfig>,
}

#[derive(Debug)]
pub struct ModelConfig {
    pub name: String,
    pub detail: Value,
}

pub type SharedServerList = Arc<Mutex<OrderMap<String, OllamaServer>>>;

/// Prints a nicely formatted list of the servers, their name, busy status, and reliability.
pub fn print_server_statuses(servers: &OrderMap<String, OllamaServer>) {
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

pub fn add_server(servers_shared: SharedServerList, server: &ServerConfig) {
    let mut servers = servers_shared.lock().unwrap();
    if servers.contains_key(&server.address) {
        println!("Warning: Server {} already exists, updating name to {}", server.address, server.name);
        servers.get_mut(&server.address).unwrap().name = server.name.clone();
        return;
    }
    servers.insert(server.address.clone(), OllamaServer {
        state: ServerState {
            busy: false,
            health: Health::Dead, // default to dead
            failure_record: FailureRecord::Reliable,
        },
        name: server.name.clone(),
        models: HashMap::new(),
        actives: HashMap::new(),
    });
    println!("Added server ({}) {} with name {}", servers.len(), server.address, server.name);
}

pub fn mark_server(servers: SharedServerList, target: &str, health: Health) {
    let mut servers = servers.lock().unwrap();
    if let Some(server) = servers.get_mut(target) {
        server.state.health = health;
        println!("Marked server {} as dead", target);
    } else {
        println!("Warning: Server {} not found", target);
    }
}
pub fn mark_server_dead(servers: SharedServerList, target: &str) {
    mark_server(servers, target, Health::Dead);
}
pub fn mark_server_healthy(servers: SharedServerList, target: &str, health: f32) {
    mark_server(servers, target, Health::Healthy(health));
}

pub async fn sync_server(
    servers: SharedServerList,
    target: String,
    timeout_secs: u32,
) {
    let target = target.as_str();
    let models = api_tags(target, timeout_secs);
    let active_models = api_ps(target, timeout_secs); // send this request ahead

    let models = match models.await {
        Ok(models) => models,
        Err(e) => {
            println!("Error fetching models from {}: {}", target, e);
            mark_server_dead(servers, target);
            return;
        }
    };

    let active_models = match active_models.await {
        Ok(active_models) => active_models,
        Err(e) => {
            println!("Error fetching active models from {}: {}", target, e);
            mark_server_dead(servers, target);
            return;
        }
    };

    let mut servers = servers.lock().unwrap();
    if let Some(server) = servers.get_mut(target) {
        server.models = models.into_iter().map(|m| (m.name.clone(), m)).collect();
        server.actives = active_models.into_iter().map(|m| (m.name.clone(), m)).collect();
        let active_summary = server.actives.keys().map(String::as_str).collect::<Vec<&str>>().join(", ");
        println!("Synced server {}, found models: {}, active: [{}]", target, server.models.len(), active_summary);
    } else {
        println!("Warning: Server {} not found", target);
    }
}