mod config;
mod state;
mod handler;
mod backend;
mod api;
mod utils;

use futures_util::future;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Server, server::conn::AddrStream};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use clap::Parser;
use ordermap::OrderMap;
use tracing::info;
use tracing_subscriber;
use time::{self, macros::format_description};

use config::Args;
use state::{add_server, sync_server};
use handler::dispatch;
use backend::ReqOpt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // tracing_subscriber::fmt::init();
    // my timer format: 03-31 15:10:11
    let time_format = format_description!("[month]-[day] [hour]:[minute]:[second]");
    let time_offset = time::UtcOffset::current_local_offset().unwrap_or_else(|_| time::UtcOffset::UTC);
    let timer = tracing_subscriber::fmt::time::OffsetTime::new(time_offset, time_format);
    tracing_subscriber::fmt()
        .with_timer(timer)
        .with_target(false)
        // .with_file(true).with_line_number(true)
        .init();

    let args = Args::parse();

    info!("Timeout settings: t0={}, t1={}, timeout_load={}", args.t0, args.t1, args.timeout_load);

    let servers = Arc::new(Mutex::new(OrderMap::new()));
    args.servers.iter().for_each(|s| { add_server(servers.clone(), s); });

    if let Some(file) = &args.server_file {
        let contents = std::fs::read_to_string(file)?;
        let configs: Vec<config::ServerConfig> = contents.lines().map(|line| line.parse().unwrap()).collect();
        configs.iter().for_each(|s| { add_server(servers.clone(), s); });
    }

    let server_addrs = servers.lock().unwrap().keys().cloned().collect::<Vec<String>>();
    assert!(!server_addrs.is_empty(), "Fatal Error: No servers provided");

    // initialize all servers
    let sync_tasks = server_addrs.into_iter().map(
        |s| tokio::spawn(sync_server(servers.clone(), s, 5))
    ).collect::<Vec<_>>();
    let healths = future::join_all(sync_tasks).await;

    let (healthy, dead): (Vec<_>, Vec<_>) = healths
        .into_iter().partition(|h|
            *h.as_ref().unwrap_or(&state::Health::Dead) != state::Health::Dead);
    info!("Initial health summary: {} healthy, {} dead", healthy.len(), dead.len());

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

    info!("Ollama Load Balancer listening on http://{}", addr);

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

    info!("Received CTRL+C, shutting down gracefully...");
    // The future returned by ctrl_c() will resolve when CTRL+C is pressed
    // Hyper will then stop accepting new connections
}
