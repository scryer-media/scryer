use anyhow::Result;
use clap::{Parser, Subcommand};
use xtask_support::TaskContext;

#[derive(Parser)]
#[command(name = "cargo xtask-trash-guides")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Sync,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ctx = TaskContext::new();

    match cli.command {
        Commands::Sync => xtask_trash_guides::run_sync(&ctx),
    }
}
