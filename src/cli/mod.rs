mod config;
mod start;
mod version;

use std::fmt::{Debug, Display};
use std::ops::Deref;
use clap::{Parser, Subcommand};

pub use config::Config;
use config::load_config;
use crate::cli::start::StartArgs;

#[derive(Debug, Parser)]
#[command(name = "magnarr", about = "Magnet-based torrent download orchestrator")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Path to the Magnarr home directory.
    #[arg(long, env = "MAGNARR_HOME", default_value = "~/.magnarr")]
    pub home: PathArg,

    /// Logging level (trace, debug, info, warn, error), defaults to info. Can also be tuned with RUST_LOG env var.
    #[arg(long)]
    pub log_level: Option<LogLevel>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

pub fn run() {
    let cli = Cli::parse();
    init_tracing(cli.log_level);

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime")
        .block_on(async move {
            match cli.command {
                Command::Start(args) => start::run(&cli.home, args).await,
                Command::Version => version::run(),
            }
        });
}

fn init_tracing(lvl: Option<LogLevel>) {
    let filter = match lvl {
        Some(l) => tracing_subscriber::EnvFilter::new(l),
        None => tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();
}
