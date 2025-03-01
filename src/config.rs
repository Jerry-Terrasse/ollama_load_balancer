use clap::Parser;
/// Struct to hold the user-supplied server address and its human-readable name.
/// Format on the command line should be:  ip:port=Name
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub address: String,
    pub name: String,
}

impl std::str::FromStr for ServerConfig {
    type Err = String;

    /// We expect the user to provide something like "127.0.0.1:11433=LocalOllama"
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.splitn(2, '=').collect();
        if parts.len() != 2 {
            return Err("Invalid server format. Use ip:port=Name".to_string());
        }
        Ok(ServerConfig {
            address: parts[0].trim().to_string(),
            name: parts[1].trim().to_string(),
        })
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Syntax is --server IP:PORT=NAME --server IP:PORT=NAME ...
    ///
    /// This is a required argument. It specifies the addresses of the Ollama servers
    /// that the load balancer will distribute requests to, plus a friendly name.
    #[arg(short, long, required = true)]
    pub server: Vec<ServerConfig>,

    /// Max seconds to allow Ollama server to pause.
    ///
    /// Don't set this too low because if the delay is too great at the beginning of response generation that will cause failure.
    /// Pass 0 to disable timeout.
    /// 
    /// This is an optional argument. It specifies the maximum number of seconds to wait for a response from the Ollama server before considering it unavailable
    #[arg(short, long, default_value_t = 30)]
    pub timeout: u32,

    /// A server must return some tokens before t0.
    #[arg(long, default_value_t = 5)]
    pub t0: u32,
    /// Number of tokens in t0~t1 is counted.
    #[arg(long, default_value_t = 10)]
    pub t1: u32,

    /// Listening address. Defaults to "0.0.0.0:11434"
    #[arg(short = 'l', long, default_value = "0.0.0.0:11434")]
    pub listen: String,
}
