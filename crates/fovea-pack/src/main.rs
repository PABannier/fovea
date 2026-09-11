use anyhow::Result;
use clap::{Parser, Subcommand};
use fovea_pack::{serve_sources, ServeOptions};

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Serve WSI slides and cell protobufs to Fovea"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve a WSI and optional protobuf cells directly.
    Serve(ServeOptions),
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Serve(options) => serve_sources(options).await,
    }
}
