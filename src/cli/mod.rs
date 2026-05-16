pub mod config;
mod start;
mod version;

use std::fmt::{Debug, Display};
use std::ops::Deref;
use clap::{Parser, Subcommand};

pub use config::{load_config, Config, ConfigError};

#[derive(Debug, Parser)]
#[command(name = "magnarr", about = "Magnarr download orchestrator")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,


#[derive(Debug, Clone, serde::Deserialize)]
pub struct PathArg(pub std::path::PathBuf);

impl Display for PathArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }}

impl Deref for PathArg {
    type Target = std::path::PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::str::FromStr for PathArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let expanded = shellexpand::tilde(value);
        Ok(Self(std::path::PathBuf::from(expanded.as_ref())))
    }
}

#[derive(clap::ValueEnum, Debug, Clone)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl AsRef<str> for LogLevel {
    fn as_ref(&self) -> &str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
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
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime")
        .block_on(async {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
                )
                .init();

            match Cli::parse().command {
                Command::Start(args) => start::run(args).await,
                Command::Version => version::run(),
            }
        });
}
