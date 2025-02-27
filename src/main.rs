mod config;
mod state;
mod handler;

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
use config::{Args, ServerConfig};
use state::{FailureRecord, ServerState, OllamaServer, print_server_statuses, SharedServerList};
use handler::{handle_request};

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
