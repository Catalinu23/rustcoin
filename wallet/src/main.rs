mod core;

use crate::core::{Config, Core, FeeConfig, FeeType, Key, Recipient};
use anyhow::Result;
use clap::{Parser, Subcommand};
use lib::types::Transaction;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::{self, Duration};

#[derive(Parser)]
#[command(name = "wallet", about = "Bitcoin wallet CLI")]
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

/// Polls the connected node on a 10-second interval to refresh the wallet's UTXO set.
/// Delegates to `Core::fetch_utxos`, which fetches UTXOs for every managed key and
/// updates the flat in-memory cache. Connection or protocol errors are printed to
/// stderr and the loop continues on the next tick.
async fn update_utxos(core: Arc<Core>) {
    let mut interval = time::interval(Duration::from_secs(10));
    loop {
        interval.tick().await;
        if let Err(e) = core.fetch_utxos().await {
            eprintln!("Failed to refresh UTXOs: {e}");
        }
    }
}

/// Drains the outbound transaction channel and submits each transaction to the node.
/// Runs as a background task for the lifetime of the wallet. Each transaction is sent
/// over a fresh TCP connection using the `SubmitTransaction` message. Errors are
/// printed to stderr but do not stop the loop — subsequent transactions are still attempted.
async fn handle_transactions(rx: kanal::AsyncReceiver<Transaction>, core: Arc<Core>) {
    while let Ok(tx) = rx.recv().await {
        if let Err(e) = core.send_transaction(tx).await {
            eprintln!("Failed to submit transaction: {e}");
        }
    }
}

/// Runs an interactive command loop on stdin until the user types `exit` or `quit`.
///
/// Supported commands:
/// - `balance`                    — print the current unspent balance in satoshis
/// - `send <contact> <amount>`    — build and queue a transaction to a named contact
/// - `contacts`                   — list all contacts from the config
/// - `help`                       — print available commands
/// - `exit` / `quit`              — stop the loop and shut down
///
/// Transaction building selects UTXOs greedily, deducts the configured fee, signs
/// each input with the wallet's private key, and queues the result for `handle_transactions`.
async fn run_cli(core: Arc<Core>) -> Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        print!("> ");
        io::stdout().flush()?;

        let line = match lines.next_line().await? {
            Some(l) => l,
            None => break,
        };

        let parts: Vec<&str> = line.trim().split_whitespace().collect();
        match parts.as_slice() {
            ["balance"] => {
                println!("{} coins", core.get_balance().await);
            }
            ["send", contact, amount_str] => {
                let amount: u64 = match amount_str.parse() {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!("Invalid amount");
                        continue;
                    }
                };
                let recipient = core.config.contacts.iter().find(|r| r.name == *contact);
                let pubkey = match recipient {
                    Some(r) => match r.load() {
                        Ok(lr) => lr.public_key,
                        Err(e) => {
                            eprintln!("Failed to load contact: {e}");
                            continue;
                        }
                    },
                    None => {
                        eprintln!("Contact '{contact}' not found");
                        continue;
                    }
                };
                match core.create_transaction(&pubkey, amount).await {
                    Ok(tx) => match core.tx_sender.send(tx).await {
                        Ok(_) => println!("Transaction queued"),
                        Err(e) => eprintln!("Failed to queue transaction: {e}"),
                    },
                    Err(e) => eprintln!("{e}"),
                }
            }
            ["contacts"] => {
                for c in &core.config.contacts {
                    println!("  {}", c.name);
                }
            }
            ["exit"] | ["quit"] => break,
            [] | ["help"] => {
                println!("Commands: balance | send <contact> <amount> | contacts | exit");
            }
            _ => eprintln!("Unknown command. Type 'help' for usage."),
        }
    }

    Ok(())
}

/// Writes a skeleton `wallet.json` config to `path` with placeholder file paths
/// and a fixed fee of 1 000 satoshis. Intended as a starting point for first-time setup —
/// the user replaces the key/contact paths with real ones before running the wallet.
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

/// Entry point. Parses CLI arguments and either generates a dummy config (and exits)
/// or starts the wallet. Startup sequence:
/// 1. Load and parse `wallet.json` (or the path given with `--config`).
/// 2. Override the node address if `--node` was passed.
/// 3. Deserialize all key pairs from disk via `Core::load`.
/// 4. Spawn `update_utxos` and `handle_transactions` as background tasks.
/// 5. Hand control to `run_cli` until the user exits.
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
    tokio::spawn(update_utxos(Arc::clone(&core)));
    tokio::spawn(handle_transactions(tx_receiver, Arc::clone(&core)));
    run_cli(core).await?;
    Ok(())
}
