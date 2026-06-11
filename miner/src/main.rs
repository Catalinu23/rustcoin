use anyhow::{Result, anyhow};
use clap::Parser;
use lib::crypto::PublicKey;
use lib::network::Message;
use lib::types::Block;
use lib::util::Saveable;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::{Duration, interval};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    address: String,
    #[arg(short, long)]
    public_key_file: String,
}

struct Miner {
    public_key: PublicKey,
    stream: Mutex<TcpStream>,
    current_template: Arc<std::sync::Mutex<Option<Block>>>,
    mining: Arc<AtomicBool>,
    mined_block_sender: flume::Sender<Block>,
    mined_block_receiver: flume::Receiver<Block>,
}

impl Miner {
    // Connects to the node at `address` and initializes the miner with the given public key.
    async fn new(address: String, public_key: PublicKey) -> Result<Self> {
        let stream = TcpStream::connect(&address).await?;
        let (mined_block_sender, mined_block_receiver) = flume::unbounded();
        Ok(Self {
            public_key,
            stream: Mutex::new(stream),
            current_template: Arc::new(std::sync::Mutex::new(None)),
            mining: Arc::new(AtomicBool::new(false)),
            mined_block_sender,
            mined_block_receiver,
        })
    }

    // Main event loop: spawns the mining thread then waits on two events:
    // a 5-second tick to fetch/validate the template, or a mined block ready to submit.
    async fn run(&self) -> Result<()> {
        self.spawn_mining_thread();
        let mut template_interval = interval(Duration::from_secs(5));
        loop {
            let receiver_clone = self.mined_block_receiver.clone();
            tokio::select! {
                _ = template_interval.tick() => {
                    self.fetch_and_validate_template().await?;
                }
                Ok(mined_block) = receiver_clone.recv_async() => {
                    self.submit_block(mined_block).await?;
                }
            }
        }
    }

    // Spawns a dedicated OS thread that spins waiting for the `mining` flag to be set,
    // then sends the current template block to the async runtime via the channel.
    fn spawn_mining_thread(&self) -> thread::JoinHandle<()> {
        let template = self.current_template.clone();
        let mining = self.mining.clone();
        let sender = self.mined_block_sender.clone();
        thread::spawn(move || {
            loop {
                if mining.load(Ordering::Relaxed) {
                    let block_opt = template.lock().unwrap().clone();
                    if let Some(mut block) = block_opt {
                        println!("Mining block with target {}", block.header.target);
                        loop {
                            if !mining.load(Ordering::Relaxed) {
                                break;
                            }
                            if block.header.mine(10_000) {
                                sender.send(block).expect("Failed to send mined block");
                                mining.store(false, Ordering::Relaxed);
                                break;
                            }
                        }
                    }
                }
                thread::yield_now();
            }
        })
    }

    // Decides every 5 seconds whether to fetch a new template (idle) or validate
    // the current one (already mining) to check if it has gone stale.
    async fn fetch_and_validate_template(&self) -> Result<()> {
        if !self.mining.load(Ordering::Relaxed) {
            self.fetch_template().await?;
        } else {
            self.validate_template().await?;
        }
        Ok(())
    }

    // Requests a block template from the node, stores it as the current template,
    // and sets the mining flag to wake up the mining thread.
    async fn fetch_template(&self) -> Result<()> {
        println!("Fetching new template");
        let message = Message::FetchTemplate(self.public_key.clone());
        let mut stream_lock = self.stream.lock().await;
        message.send_async(&mut *stream_lock).await?;
        match Message::receive_async(&mut *stream_lock).await? {
            Message::Template(template) => {
                println!(
                    "Received new template with target: {}",
                    template.header.target
                );
                *self.current_template.lock().unwrap() = Some(template);
                self.mining.store(true, Ordering::Relaxed);
                Ok(())
            }
            _ => Err(anyhow!(
                "Unexpected message received when fetching template"
            )),
        }
    }

    // Asks the node whether the current template is still valid. If not (another block
    // was found), clears the mining flag so a fresh template is fetched next tick.
    async fn validate_template(&self) -> Result<()> {
        if let Some(template) = self.current_template.lock().unwrap().clone() {
            let message = Message::ValidateTemplate(template);
            let mut stream_lock = self.stream.lock().await;
            message.send_async(&mut *stream_lock).await?;
            match Message::receive_async(&mut *stream_lock).await? {
                Message::TemplateValidity(valid) => {
                    if !valid {
                        println!("Current template is no longer valid");
                        self.mining.store(false, Ordering::Relaxed);
                    } else {
                        println!("Current template is still valid");
                    }
                    Ok(())
                }
                _ => Err(anyhow!("Unexpected message received")),
            }
        } else {
            Ok(())
        }
    }

    // Sends a mined block to the node and clears the mining flag so the next
    // tick fetches a new template.
    async fn submit_block(&self, block: Block) -> Result<()> {
        println!("Submitting mined block");
        let message = Message::SubmitTemplate(block);
        let mut stream_lock = self.stream.lock().await;
        message.send_async(&mut *stream_lock).await?;
        self.mining.store(false, Ordering::Relaxed);
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let public_key = PublicKey::load_from_file(&cli.public_key_file)
        .map_err(|_| anyhow!("Error reading public key!"))?;
    let miner = Miner::new(cli.address, public_key).await?;
    miner.run().await
}
