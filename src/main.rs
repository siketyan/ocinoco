mod builder;
mod cmd;
mod fs;
mod io;

use std::process::exit;

use clap::Parser;
use tracing::error;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::util::SubscriberInitExt;

use crate::cmd::Command;

/// Build an OCI image with no container.
#[derive(Clone, Debug, clap::Parser)]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[tokio::main]
async fn main() {
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    tracing_subscriber::fmt()
        .compact()
        .with_env_filter(env_filter)
        .without_time()
        .finish()
        .init();

    let cli = Cli::parse();
    if let Err(err) = cli.command.run().await {
        error!("{}", err);
        exit(1);
    }
}
