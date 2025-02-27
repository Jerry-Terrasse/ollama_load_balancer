use ordermap::OrderMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub enum FailureRecord {
    Reliable,
    Unreliable,
    SecondChanceGiven,
}

#[derive(Debug)]
pub struct ServerState {
    pub busy: bool,
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
