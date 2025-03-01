mod config;
mod state;
mod handler;
mod backend;

use hyper::service::{make_service_fn, service_fn};
use hyper::{Server, server::conn::AddrStream};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use clap::Parser;
use ordermap::OrderMap;
use config::Args;
use state::add_server;
use handler::dispatch;
use backend::ReqOpt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let servers = Arc::new(Mutex::new(OrderMap::new()));
    args.servers.iter().for_each(|s| { add_server(servers.clone(), s); });

    if let Some(file) = &args.server_file {
        let contents = std::fs::read_to_string(file)?;
        let configs: Vec<config::ServerConfig> = contents.lines().map(|line| line.parse().unwrap()).collect();
        configs.iter().for_each(|s| { add_server(servers.clone(), s); });
    }

    assert!(!servers.lock().unwrap().is_empty(), "Fatal Error: No servers provided");

    println!("");
    println!("⚙️  Timeout setting: t0={}, t1={}, timeout_load={}", args.t0, args.t1, args.timeout_load);
    println!("");

    let global_opts = ReqOpt {
        timeout_load: args.timeout_load,
        t0: args.t0,
        t1: args.t1,
    };

    let make_svc = make_service_fn(|conn: &AddrStream| {
        let remote_addr = conn.remote_addr();
        let servers = servers.clone();
        let opts = global_opts.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let servers = servers.clone();
                // handle_request(req, servers, remote_addr, args.timeout)
                // handle_request_parallel(req, servers, remote_addr, opts)
                dispatch(req, servers, remote_addr, opts)
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
