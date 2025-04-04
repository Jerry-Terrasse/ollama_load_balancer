use ordermap::OrderMap;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use serde_json::Value;
use rand::{self, Rng};
use tracing::{info, warn, error};

use crate::config::ServerConfig;
use crate::api::{api_tags, api_ps};
use crate::utils::efraimidis_spirakis_sample;

#[derive(Clone, Debug)]
pub enum FailureRecord {
    Reliable,
    Unreliable,
    SecondChanceGiven,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Health {
    Dead,
    Healthy(f32),
}

#[derive(Debug, Clone)]
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

pub struct ServerSnapshot {
    pub state: ServerState,
    pub name: String,
    pub models: HashMap<String, Option<ModelConfig>>,
    pub actives: HashMap<String, Option<ModelConfig>>,
}

#[derive(Debug, Clone)]
pub struct ModelConfig {
    pub name: String,
    pub detail: Value,
}

pub type SharedServerList = Arc<Mutex<OrderMap<String, OllamaServer>>>;

/// Prints a nicely formatted list of the servers, their name, busy status, and reliability.
pub fn print_server_statuses(servers: &OrderMap<String, OllamaServer>) {
    info!("Current server statuses:");
    for (i, (address, srv)) in servers.iter().enumerate() {
        let busy_status = if srv.state.busy { "Busy" } else { "Available" };
        let reliability = match srv.state.failure_record {
            FailureRecord::Reliable => "Reliable",
            FailureRecord::Unreliable => "Unreliable",
            FailureRecord::SecondChanceGiven => "SecondChanceGiven",
        };
        info!("{}. Address: {} ({}), Busy: {}, Reliability: {}", i + 1, address, srv.name, busy_status, reliability);
    }
}

pub fn add_server(servers_shared: SharedServerList, server: &ServerConfig) {
    let mut servers = servers_shared.lock().unwrap();
    if servers.contains_key(&server.address) {
        warn!("Server {} already exists, updating name to {}", server.address, server.name);
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
    info!("Added server ({}) {} with name {}", servers.len(), server.address, server.name);
}

pub fn mark_server(servers: SharedServerList, target: &str, health: Health) {
    let mut servers = servers.lock().unwrap();
    if let Some(server) = servers.get_mut(target) {
        server.state.health = health;
        info!("Marked server {} as dead", target);
    } else {
        warn!("Server {} not found", target);
    }
}
pub fn mark_server_dead(servers: SharedServerList, target: &str) {
    mark_server(servers, target, Health::Dead);
}
pub fn mark_server_healthy(servers: SharedServerList, target: &str, health: f32) {
    mark_server(servers, target, Health::Healthy(health));
}
pub fn mark_server_more_healthy(servers: SharedServerList, target: &str, is_best: bool) {
    let h_inc = if is_best { 4.0 } else { 2.0 };
    let mut servers = servers.lock().unwrap();
    if let Some(server) = servers.get_mut(target) {
        if let Health::Healthy(h) = server.state.health {
            server.state.health = Health::Healthy(h + h_inc);
        } else {
            info!("Server {} is resurrected", target);
            server.state.health = Health::Healthy(1.0);
        }
        info!(
            "Marked server {} as more healthy{}, now: {:?}", 
            target,
            if is_best { " (best)" } else { "" },
            server.state.health
        );
    } else {
        warn!("Server {} not found", target);
    }
}
pub fn mark_server_less_healthy(servers: SharedServerList, target: &str) {
    let h_div = 2.0;
    let mut servers = servers.lock().unwrap();
    if let Some(server) = servers.get_mut(target) {
        if let Health::Healthy(h) = server.state.health {
            let new_h = h / h_div;
            if new_h < 1.0 {
                info!("Server {} passed away", target);
                server.state.health = Health::Dead;
            } else {
                server.state.health = Health::Healthy(new_h);
            }
        }
        info!("Marked server {} as less healthy, now: {:?}", target, server.state.health);
    } else {
        warn!("Server {} not found", target);
    }
}

pub async fn sync_server(
    servers: SharedServerList,
    target: String,
    timeout_secs: u32,
) -> Health {
    let target = target.as_str();
    let models = api_tags(target, timeout_secs);
    let active_models = api_ps(target, timeout_secs); // send this request ahead

    let models = match models.await {
        Ok(models) => models,
        Err(e) => {
            warn!("Failed to fetch models from {}: {}", target, e);
            mark_server_dead(servers, target);
            return Health::Dead;
        }
    };

    let active_models = match active_models.await {
        Ok(active_models) => active_models,
        Err(e) => {
            warn!("Failed to fetch active models from {}: {}", target, e);
            mark_server_dead(servers, target);
            return Health::Dead;
        }
    };

    let mut servers = servers.lock().unwrap();
    if let Some(server) = servers.get_mut(target) {
        server.models = models.into_iter().map(|m| (m.name.clone(), m)).collect();
        server.actives = active_models.into_iter().map(|m| (m.name.clone(), m)).collect();
        server.state.health = Health::Healthy(1.0); // default to 1.0
        let active_summary = server.actives.keys().map(String::as_str).collect::<Vec<&str>>().join(", ");
        let model_summary = server.models.keys().map(String::as_str).collect::<Vec<&str>>().join(", ");
        info!("Synced server {}, found models: {}\n> All models: [{}]\n> Active models: [{}]",
            target, server.models.len(), model_summary, active_summary);
        Health::Healthy(1.0)
    } else {
        warn!("Server {} not found", target);
        Health::Dead
    }
}

pub fn snapshot_servers(servers: SharedServerList, need_detail: bool) -> HashMap<String, ServerSnapshot> {
    let servers = servers.lock().unwrap();
    servers.iter().map(|(addr, srv)| {
        let models: HashMap<String, Option<ModelConfig>> = if need_detail {
            srv.models.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect()
        } else {
            srv.models.keys().map(|k| (k.clone(), None)).collect()
        };
        // let actives = srv.actives.keys().map(|k| (k.clone(), ())).collect();
        let actives: HashMap<String, Option<ModelConfig>> = if need_detail {
            srv.actives.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect()
        } else {
            srv.actives.keys().map(|k| (k.clone(), None)).collect()
        };
        (addr.clone(), ServerSnapshot {
            state: srv.state.clone(),
            name: srv.name.clone(),
            models,
            actives,
        })
    }).collect()
}

#[derive(Default)]
pub struct SelOpt {
    pub count: (usize, usize),
    pub resurrect_p: f32,
    pub resurrect_n: usize,
}

pub fn sample_by_health<'a>(
    snaps: &HashMap<String, ServerSnapshot>,
    source: &[&'a String],
    count: usize,
    rng: &mut rand::rngs::ThreadRng,
) -> Vec<&'a String> {
    let healths = source.iter().map(|name| {
        let health = match snaps.get(name.as_str()).unwrap().state.health {
            Health::Healthy(h) => h,
            _ => 0.1,
        };
        health
    }).collect::<Vec<_>>();
    let indices = efraimidis_spirakis_sample(&healths, count, rng);
    indices.into_iter().map(|i| source[i]).collect()
}

pub fn select_servers(
    servers: SharedServerList,
    model: String,
    opts: SelOpt,
) -> Vec<String> {
    let mut rng = rand::rng();
    let (mut min_sel, mut max_sel) = opts.count;
    let mut resurrect_n = if rng.random::<f32>() < opts.resurrect_p {
        min_sel -= opts.resurrect_n;
        max_sel -= opts.resurrect_n;
        opts.resurrect_n
    } else {
        0
    };

    let snaps = snapshot_servers(servers, false);
    let mut selected: Vec<(&str, Vec<&String>)> = Vec::new();
    let mut num_selected = 0; // NOTE: num_selected means not selected.len()

    info!("Server snapshots:");
    for (addr, snap) in snaps.iter() {
        let actives = snap.actives.keys().map(|k| k.as_str()).collect::<Vec<&str>>().join(", ");
        info!("> {}: health: {:?}, actives: [{}]", addr, snap.state.health, actives);
    }
    info!("Selecting servers with min: {} max: {} resurrect: {}", min_sel, max_sel, resurrect_n);

    // 1. choose from alive servers with the model activated
    // NOTE: servers that are alive but do not have the target model are NEVER selected
    let alives = snaps.iter().filter_map(|(addr, snap)| {
        if snap.state.health != Health::Dead && snap.models.contains_key(&model) {
            Some(addr)
        } else {
            None
        }
    }).collect::<Vec<_>>();
    let actives = alives.iter().filter(|name| {
        snaps.get(name.as_str()).unwrap().actives.contains_key(&model)
    }).cloned().collect::<Vec<_>>();
    if actives.len() <= max_sel { 
        selected.push(("active", actives));
    } else {
        selected.push(("active", sample_by_health(&snaps, &actives, max_sel, &mut rng)));
    }
    num_selected += selected.last().unwrap().1.len();

    // 2. choose from alive but inactive servers
    if num_selected < min_sel {
        let inactives = alives.iter().filter(|name| {
            !snaps.get(name.as_str()).unwrap().actives.contains_key(&model)
        }).cloned().collect::<Vec<_>>();
        if num_selected + inactives.len() <= min_sel {
            selected.push(("inactive", inactives));
        } else {
            selected.push(("inactive", sample_by_health(&snaps, &inactives, min_sel - num_selected, &mut rng)));
        }
        num_selected += selected.last().unwrap().1.len();
    }

    // 3. choose from dead servers
    if num_selected < min_sel {
        resurrect_n += min_sel - num_selected;
    }
    if resurrect_n > 0 {
        let deads = snaps.iter().filter_map(|(addr, snap)| {
            if snap.state.health == Health::Dead {
                Some(addr)
            } else {
                None
            }
        }).collect::<Vec<_>>();
        selected.push(("resurrect", sample_by_health(&snaps, &deads, resurrect_n, &mut rng)));
        num_selected += selected.last().unwrap().1.len();
    }

    // make a summary
    let summary = selected.iter().map(|(tag, addrs)| {
        let names = addrs.iter().map(|a| snaps.get(a.as_str()).unwrap().name.as_str()).collect::<Vec<&str>>();
        if names.len() > 0 {
            format!("> {} ({}): {}", tag, names.len(), names.join(", "))
        } else {
            format!("> {} (0): none", tag)
        }
    }).collect::<Vec<String>>().join("\n");
    info!("Selected {} servers for model {}:\n{}", num_selected, model, summary);

    selected.into_iter().flat_map(|(_, addrs)| addrs).map(|s| s.clone()).collect()
}