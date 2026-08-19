mod cli;
mod server;

use clap::Parser;
use cli::{Cli, Command};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        None | Some(Command::Serve) => server::run_server().await,
        Some(Command::Debug { subcommand }) => cli::run_debug(subcommand).await,
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
