mod core;
mod tui;

use crate::core::{Config, Core, FeeConfig, FeeType, Key, Recipient};
use anyhow::Result;
use clap::{Parser, Subcommand};
use lib::types::Transaction;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "wallet", about = "rsbtc wallet")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[arg(short, long, value_name = "ADDRESS")]
    node: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    GenerateConfig {
        #[arg(short, long, value_name = "FILE")]
        output: PathBuf,
    },
}

async fn handle_transactions(rx: kanal::AsyncReceiver<Transaction>, core: Arc<Core>) {
    while let Ok(tx) = rx.recv().await {
        if let Err(e) = core.send_transaction(tx).await {
            eprintln!("Failed to submit transaction: {e}");
        }
    }
}

fn generate_dummy_config(path: &PathBuf) -> Result<()> {
    let config = Config {
        node: "127.0.0.1:8765".to_string(),
        keys: vec![Key {
            private_key_path: PathBuf::from("private_key.bin"),
            public_key_path: PathBuf::from("public_key.bin"),
        }],
        contacts: vec![Recipient {
            name: "alice".to_string(),
            public_key_path: PathBuf::from("alice_pubkey.bin"),
        }],
        fee: FeeConfig {
            fee_type: FeeType::Fixed,
            value: 1000.0,
        },
    };
    let json = serde_json::to_string_pretty(&config)?;
    std::fs::write(path, json)?;
    println!("Config written to {}", path.display());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(Command::GenerateConfig { output }) = cli.command {
        return generate_dummy_config(&output);
    }

    let config_path = cli
        .config
        .unwrap_or_else(|| PathBuf::from("wallet_config.toml"));
    let (mut core, tx_receiver) = Core::load(config_path)?;

    if let Some(node) = cli.node {
        core.config.node = node;
    }

    let core = Arc::new(core);
    let rt = tokio::runtime::Handle::current();

    tokio::spawn(handle_transactions(tx_receiver, Arc::clone(&core)));

    tui::run(core, rt)?;
    Ok(())
}
