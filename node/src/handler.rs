use chrono::Utc;
use tokio::net::TcpStream;
use uuid::Uuid;

use lib::crypto::PublicKey;
use lib::network::Message;
use lib::sha256::Hash;
use lib::types::{Block, BlockHeader, Transaction, TransactionOutput};
use lib::util::MerkleRoot;

async fn broadcast_to_peers(message: Message) {
    let nodes = crate::NODES
        .iter()
        .map(|x| x.key().clone())
        .collect::<Vec<_>>();
    for node in nodes {
        if let Some(mut stream) = crate::NODES.get_mut(&node) {
            if message.clone().send_async(&mut *stream).await.is_err() {
                println!("failed to broadcast to {node}");
            }
        }
    }
}

// Main dispatch loop: reads one message per iteration and routes it to the appropriate handler.
// Returns (closes the connection) when a handler signals false or the socket errors.
pub async fn handle_connection(mut socket: TcpStream) {
    loop {
        let message = match Message::receive_async(&mut socket).await {
            Ok(message) => message,
            Err(e) => {
                println!("invalid message from peer: {e}, closing that connection");
                return;
            }
        };

        use lib::network::Message::*;
        let keep_alive = match message {
            UTXOs(_) | Template(_) | Difference(_) | TemplateValidity(_) | NodeList(_) => {
                println!("I am neither a miner nor a wallet! Goodbye");
                false
            }
            FetchBlock(height) => fetch_block(&mut socket, height).await,
            DiscoverNodes => discover_nodes(&mut socket).await,
            AskDifference(height) => ask_difference(&mut socket, height).await,
            FetchUTXOs(key) => fetch_utxos(&mut socket, key).await,
            NewBlock(block) => new_block(block).await,
            NewTransaction(tx) => new_transaction(tx).await,
            ValidateTemplate(block_template) => {
                validate_template(&mut socket, block_template).await
            }
            SubmitTemplate(block) => submit_template(block).await,
            SubmitTransaction(tx) => submit_transaction(tx).await,
            FetchTemplate(pubkey) => fetch_template(&mut socket, pubkey).await,
        };

        if !keep_alive {
            return;
        }
    }
}

// Sends the block at the requested height back to the peer.
// Returns false (closes) if the height is beyond the current chain tip.
async fn fetch_block(socket: &mut TcpStream, height: usize) -> bool {
    let blockchain = crate::BLOCKCHAIN.read().await;
    let Some(block) = blockchain.blocks().get(height).cloned() else {
        return false;
    };
    let message = Message::NewBlock(block);
    message.send_async(socket).await.unwrap();
    true
}

// Responds with the addresses of all currently connected peer nodes.
async fn discover_nodes(socket: &mut TcpStream) -> bool {
    let nodes = crate::NODES
        .iter()
        .map(|x| x.key().clone())
        .collect::<Vec<_>>();
    let message = Message::NodeList(nodes);
    message.send_async(socket).await.unwrap();
    true
}

// Responds with how many blocks ahead (positive) or behind (negative) this node is
// relative to the peer's reported height.
async fn ask_difference(socket: &mut TcpStream, height: u32) -> bool {
    let blockchain = crate::BLOCKCHAIN.read().await;
    let count = blockchain.block_height() as i32 - height as i32;
    let message = Message::Difference(count);
    message.send_async(socket).await.unwrap();
    true
}

// Filters the UTXO set by public key and sends the matching outputs to the peer.
async fn fetch_utxos(socket: &mut TcpStream, key: PublicKey) -> bool {
    println!("received request to fetch UTXOs");
    let blockchain = crate::BLOCKCHAIN.read().await;

    let utxos = blockchain
        .utxos()
        .iter()
        .filter(|(_, (_, txout))| txout.pubkey == key)
        .map(|(_, (marked, txout))| (txout.clone(), *marked))
        .collect::<Vec<_>>();

    let message = Message::UTXOs(utxos);
    message.send_async(socket).await.unwrap();
    false
}

// Adds a block forwarded by another node to the local chain, logging if it is rejected.
async fn new_block(block: Block) -> bool {
    let mut blockchain = crate::BLOCKCHAIN.write().await;
    println!("received new block");
    if blockchain.add_block(block).is_err() {
        println!("block rejected");
    }
    true
}

// Adds a transaction forwarded by another node to the mempool.
// Returns false (closes) if the transaction fails validation.
async fn new_transaction(tx: Transaction) -> bool {
    let mut blockchain = crate::BLOCKCHAIN.write().await;
    println!("received transaction from friend");
    if blockchain.add_to_mempool(tx).is_err() {
        println!("transaction rejected, closing connection");
        return false;
    }
    true
}

// Checks whether the template's prev_block_hash still matches the current chain tip
// and replies with the validity status so the miner can decide to keep or discard it.
async fn validate_template(socket: &mut TcpStream, block_template: Block) -> bool {
    let blockchain = crate::BLOCKCHAIN.read().await;

    let status = block_template.header.prev_block_hash
        == blockchain
            .blocks()
            .last()
            .map(|last_block| last_block.hash())
            .unwrap_or(Hash::zero());

    let message = Message::TemplateValidity(status);
    message.send_async(socket).await.unwrap();
    true
}

// Validates and appends a mined block, rebuilds the UTXO set, then broadcasts
// the block to all known peers. Returns false (closes) if the block is rejected.
async fn submit_template(block: Block) -> bool {
    println!("received allegedly mined template");
    let mut blockchain = crate::BLOCKCHAIN.write().await;
    if let Err(e) = blockchain.add_block(block.clone()) {
        println!("block rejected: {e}, closing connection");
        return false;
    }

    blockchain.rebuild_utxos();
    println!("block looks good, broadcasting");
    drop(blockchain);
    broadcast_to_peers(Message::NewBlock(block)).await;
    true
}

// Adds a wallet-submitted transaction to the mempool and forwards it to all known peers.
// Returns false (closes) if the transaction fails validation.
async fn submit_transaction(tx: Transaction) -> bool {
    let mut blockchain = crate::BLOCKCHAIN.write().await;
    if let Err(e) = blockchain.add_to_mempool(tx.clone()) {
        println!("transaction rejected, closing connection: {e}");
        return false;
    }

    println!("added transaction to mempool");
    drop(blockchain);
    broadcast_to_peers(Message::NewTransaction(tx)).await;
    false
}

// Builds a block template from the current mempool, inserts a coinbase output for the
// requesting miner's public key (value = block reward + fees), and sends it back.
// Returns false (closes) if miner fee calculation fails.
async fn fetch_template(socket: &mut TcpStream, pubkey: PublicKey) -> bool {
    let blockchain = crate::BLOCKCHAIN.read().await;

    let mut transactions = vec![];
    transactions.extend(
        blockchain
            .mempool()
            .iter()
            .take(lib::BLOCK_TRANSACTION_CAP)
            .map(|(_, tx)| tx)
            .cloned()
            .collect::<Vec<_>>(),
    );
    transactions.insert(
        0,
        Transaction {
            inputs: vec![],
            outputs: vec![TransactionOutput {
                pubkey,
                unique_id: Uuid::new_v4(),
                value: 0,
            }],
        },
    );

    let merkle_root = MerkleRoot::calculate(&transactions);

    let mut block = Block::new(
        BlockHeader {
            timestamp: Utc::now(),
            prev_block_hash: blockchain
                .blocks()
                .last()
                .map(|last_block| last_block.hash())
                .unwrap_or(Hash::zero()),
            nonce: 0,
            target: *blockchain.target(),
            merkle_root,
        },
        transactions,
    );

    let miner_fees = match block.calculate_miner_fees(blockchain.utxos()) {
        Ok(fees) => fees,
        Err(e) => {
            eprintln!("{e}");
            return false;
        }
    };

    let reward = Block::calculate_block_reward(blockchain.block_height() as u64);

    block.transactions[0].outputs[0].value = reward + miner_fees;
    block.header.merkle_root = MerkleRoot::calculate(&block.transactions);

    let message = Message::Template(block);
    message.send_async(socket).await.unwrap();
    true
}
