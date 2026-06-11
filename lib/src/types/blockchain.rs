use crate::U256;
use crate::error::BlockChainError;
use crate::error::Result;
use crate::sha256::Hash;
use crate::util::{MerkleRoot, Saveable};
use super::block::Block;
use super::transaction::{Transaction, TransactionOutput};
use bigdecimal::BigDecimal;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Error as IoError, ErrorKind as IoErrorKind, Read, Result as IoResult, Write};

/// The full chain of blocks, ordered from genesis to the most recent.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Blockchain {
    // Maps transaction output hashes to their corresponding UTXOs.
    utxos: HashMap<Hash, (bool, TransactionOutput)>,
    target: U256,
    blocks: Vec<Block>,
    #[serde(default, skip_serializing)]
    mempool: Vec<(DateTime<Utc>, Transaction)>,
}

fn transaction_fee(tx: &Transaction, utxos: &HashMap<Hash, (bool, TransactionOutput)>) -> u64 {
    let inputs: u64 = tx
        .inputs
        .iter()
        .map(|i| {
            utxos
                .get(&i.prev_transaction_output_hash)
                .expect("BUG: impossible")
                .1
                .value
        })
        .sum();
    let outputs: u64 = tx.outputs.iter().map(|o| o.value).sum();
    inputs.saturating_sub(outputs)
}

impl Blockchain {
    /// Creates a new empty blockchain with no blocks.
    pub fn new() -> Self {
        Blockchain {
            utxos: HashMap::new(),
            target: crate::MIN_TARGET,
            blocks: vec![],
            mempool: vec![],
        }
    }

    pub fn utxos(&self) -> &HashMap<Hash, (bool, TransactionOutput)> {
        &self.utxos
    }

    pub fn target(&self) -> &U256 {
        &self.target
    }

    pub fn blocks(&self) -> &Vec<Block> {
        &self.blocks
    }

    pub fn block_height(&self) -> usize {
        self.blocks.len()
    }

    pub fn mempool(&self) -> &Vec<(DateTime<Utc>, Transaction)> {
        &self.mempool
    }

    /// Appends a block to the end of the chain.
    pub fn add_block(&mut self, block: Block) -> Result<()> {
        if self.blocks.is_empty() {
            // The first block must have a zero hash as its previous block hash.
            if block.header.prev_block_hash != Hash::zero() {
                println!("zero hash");
                return Err(BlockChainError::InvalidBlock);
            }
        } else {
            // The previous block hash of the new block must match the hash of the last block in the chain.
            if block.header.prev_block_hash != self.blocks.last().unwrap().hash() {
                println!(
                    "prev hash: {:?}, last hash: {:?}",
                    block.header.prev_block_hash,
                    self.blocks.last().unwrap().hash()
                );
                return Err(BlockChainError::InvalidBlockHeader);
            }
            // The hash of the new block must match the target difficulty.
            if !block.header.hash().matches(block.header.target) {
                println!(
                    "Does not match target: hash={:?}, target={:?}",
                    block.header.hash(),
                    block.header.target
                );
                return Err(BlockChainError::InvalidBlockHeader);
            }

            let calculated_merkle_root = MerkleRoot::calculate(&block.transactions);
            if calculated_merkle_root != block.header.merkle_root {
                println!(
                    "Merkle root mismatch: calculated={:?}, expected={:?}",
                    calculated_merkle_root, block.header.merkle_root
                );
                return Err(BlockChainError::InvalidMerkleRoot);
            }

            if block.header.timestamp <= self.blocks.last().unwrap().header.timestamp {
                println!(
                    "Timestamp out of order: new={:?}, last={:?}",
                    block.header.timestamp,
                    self.blocks.last().unwrap().header.timestamp
                );
                return Err(BlockChainError::InvalidBlockHeader);
            }
        }

        let block_transactions: HashSet<Hash> =
            block.transactions.iter().map(|t| t.hash()).collect();
        self.mempool
            .retain(|(_, tx)| !block_transactions.contains(&tx.hash()));

        self.blocks.push(block);
        self.try_adjust_target();
        Ok(())
    }

    pub fn add_to_mempool(&mut self, tx: Transaction) -> Result<()> {
        let mut known_inputs = HashSet::new();
        for input in &tx.inputs {
            if !self.utxos.contains_key(&input.prev_transaction_output_hash) {
                return Err(BlockChainError::InvalidTransaction);
            }
            if known_inputs.contains(&input.prev_transaction_output_hash) {
                return Err(BlockChainError::InvalidTransaction);
            }
            known_inputs.insert(input.prev_transaction_output_hash);
        }

        for input in &tx.inputs {
            if let Some((true, _)) = self.utxos.get(&input.prev_transaction_output_hash) {
                let referencing_transaction =
                    self.mempool
                        .iter()
                        .enumerate()
                        .find(|(_, (_, transaction))| {
                            transaction
                                .outputs
                                .iter()
                                .any(|output| output.hash() == input.prev_transaction_output_hash)
                        });
                if let Some((_idx, (_, referencing_transaction))) = referencing_transaction {
                    for input in &referencing_transaction.inputs {
                        self.utxos
                            .entry(input.prev_transaction_output_hash)
                            .and_modify(|(marked, _)| {
                                *marked = false;
                            });
                    }
                }
            }
        }

        let all_inputs = tx
            .inputs
            .iter()
            .map(|i| {
                self.utxos
                    .get(&i.prev_transaction_output_hash)
                    .expect("BUG: impossible")
                    .1
                    .value
            })
            .sum::<u64>();
        let all_outputs = tx.outputs.iter().map(|o| o.value).sum::<u64>();

        if all_inputs < all_outputs {
            return Err(BlockChainError::InvalidTransaction);
        }

        for input in &tx.inputs {
            self.utxos
                .entry(input.prev_transaction_output_hash)
                .and_modify(|(marked, _)| {
                    *marked = true;
                });
        }

        self.mempool.push((Utc::now(), tx));
        let utxos = &self.utxos;
        self.mempool.sort_by_key(|(_, tx)| {
            std::cmp::Reverse(transaction_fee(tx, utxos))
        });
        Ok(())
    }

    pub fn try_adjust_target(&mut self) {
        if self.blocks.is_empty() {
            return;
        }
        if self.blocks.len() % crate::DIFFICULTY_UPDATE_INTERVAL as usize != 0 {
            return;
        }
        let start_time = self.blocks
            [self.blocks.len() - crate::DIFFICULTY_UPDATE_INTERVAL as usize]
            .header
            .timestamp;
        let end_time = self.blocks.last().unwrap().header.timestamp;
        let elapsed = end_time - start_time;
        let elapsed_seconds = elapsed.num_seconds();
        let target_seconds = crate::IDEAL_BLOCK_TIME * crate::DIFFICULTY_UPDATE_INTERVAL;
        let new_target = BigDecimal::parse_bytes(&self.target.to_string().as_bytes(), 10)
            .expect("BUG: impossible")
            * (BigDecimal::from(elapsed_seconds) / BigDecimal::from(target_seconds));
        let new_target_str = new_target
            .to_string()
            .split('.')
            .next()
            .expect("BUG: Expected a decimal point")
            .to_owned();
        let new_target: U256 = U256::from_str_radix(&new_target_str, 10).expect("BUG: impossible");
        let new_target = if new_target < self.target / 4 {
            self.target / 4
        } else if new_target > self.target * 4 {
            self.target * 4
        } else {
            new_target
        };
        self.target = new_target.min(crate::MIN_TARGET);
    }

    pub fn cleanup_mempool(&mut self) {
        let now = Utc::now();
        self.mempool.retain(|(timestamp, _)| {
            (now - *timestamp).num_seconds() < crate::MAX_MEMPOOL_TRANSACTION_AGE as i64
        });
    }

    /// Rebuilds the UTXO set from the blockchain's blocks.
    pub fn rebuild_utxos(&mut self) {
        for block in &self.blocks {
            for transaction in &block.transactions {
                for input in &transaction.inputs {
                    self.utxos.remove(&input.prev_transaction_output_hash);
                }
                for output in transaction.outputs.iter() {
                    self.utxos
                        .insert(output.hash(), (false, output.clone()));
                }
            }
        }
    }
}

impl Saveable for Blockchain {
    fn load<I: Read>(reader: I) -> IoResult<Self> {
        ciborium::de::from_reader(reader)
            .map_err(|e| IoError::new(IoErrorKind::InvalidData, e.to_string()))
    }

    fn save<O: Write>(&self, writer: O) -> IoResult<()> {
        ciborium::ser::into_writer(self, writer)
            .map_err(|e| IoError::new(IoErrorKind::InvalidData, e.to_string()))
    }
}
