use ordermap::OrderMap;
use std::sync::{Arc, Mutex};

use crate::config::ServerConfig;

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
    pub health: Health,
    pub failure_record: FailureRecord,
}

#[derive(Debug)]
pub struct OllamaServer {
    pub state: ServerState,
    pub name: String,
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

pub fn add_server(servers: SharedServerList, server: &ServerConfig) {
    let mut servers = servers.lock().unwrap();
    if servers.contains_key(&server.address) {
        println!("Warning: Server {} already exists, updating name to {}", server.address, server.name);
        servers.get_mut(&server.address).unwrap().name = server.name.clone();
        return;
    }
    servers.insert(server.address.clone(), OllamaServer {
        state: ServerState {
            busy: false,
            health: Health::Healthy(1.0),
            failure_record: FailureRecord::Reliable,
        },
        name: server.name.clone(),
    });
    println!("Added server ({}) {} with name {}", servers.len(), server.address, server.name);
}
