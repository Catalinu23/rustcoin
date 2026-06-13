use anyhow::Result;
use argh::FromArgs;
use dashmap::DashMap;
use lib::types::Blockchain;
use static_init::dynamic;
use std::path::Path;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

mod handler;
mod util;

#[dynamic]
pub static BLOCKCHAIN: RwLock<Blockchain> = RwLock::new(Blockchain::new());

#[dynamic]
pub static NODES: DashMap<String, TcpStream> = DashMap::new();

/// Bitcoin node
#[derive(FromArgs)]
struct Args {
    /// port to listen on
    #[argh(option, default = "8765")]
    port: u16,

    /// path to the blockchain file
    #[argh(option, default = "String::from(\"./blockchain.cbor\")")]
    blockchain_file: String,

    /// addresses of known nodes to connect to
    #[argh(positional)]
    nodes: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Args = argh::from_env();

    let port = args.port;
    let blockchain_file = args.blockchain_file;
    let nodes = args.nodes;

    if Path::new(&blockchain_file).exists() {
        util::load_blockchain(&blockchain_file).await?;
    } else {
        println!("Blockchain file doesn not exist!");
        util::populate_connections(&nodes).await?;
        println!("{} known nodes", NODES.len());
        if nodes.is_empty() {
            println!("Starting a seed node");
        } else {
            let (longest_name, longest_count) = util::find_longest_chain_node().await?;
            util::download_blockchain(&longest_name, longest_count).await?;
            println!("Blockchain downloaded from {}", longest_name);
            {
                let mut blockchain = BLOCKCHAIN.write().await;
                blockchain.rebuild_utxos();
            }
            {
                let mut blockchain = BLOCKCHAIN.write().await;
                blockchain.try_adjust_target();
            }
        }
    }

    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await?;
    println!("Listening to {}", addr);

    tokio::spawn(util::cleanup());
    tokio::spawn(util::save(blockchain_file.clone()));

    loop {
        let (socket, _) = listener.accept().await?;
        tokio::spawn(handler::handle_connection(socket));
    }
}
