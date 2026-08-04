#![warn(dead_code_pub_in_binary)]

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use xtask_support::TaskContext;

mod migrations;

#[derive(Parser)]
#[command(name = "cargo xtask-migrations")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Rebaseline(RebaselineArgs),
}

#[derive(Args, Clone)]
pub(crate) struct RebaselineArgs {
    #[arg(long)]
    through: i64,
    #[arg(long)]
    force: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ctx = TaskContext::new();

    match cli.command {
        Commands::Rebaseline(args) => migrations::run_rebaseline(&ctx, args),
    }
}
