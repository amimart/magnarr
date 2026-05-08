pub mod config;
mod start;
mod version;

use clap::{Parser, Subcommand};

pub use config::{load_config, Config, ConfigError};

#[derive(Debug, Parser)]
#[command(name = "magnarr", about = "Magnarr download orchestrator")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start the magnarr server
    Start(StartArgs),
    /// Display version and build information
    Version,
}

#[derive(Debug, Parser)]
pub struct StartArgs {
    /// Path to the config file
    #[arg(long, default_value = "config.yaml")]
    pub config: String,

    /// Server listen address
    #[arg(long)]
    pub server_listen_addr: Option<String>,

    /// Store path
    #[arg(long)]
    pub store_path: Option<String>,
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    match Cli::parse().command {
        Command::Start(args) => start::run(args),
        Command::Version => version::run(),
    }
}
