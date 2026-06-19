use super::transaction::{Transaction, TransactionOutput};
use crate::U256;
use crate::error::BlockChainError;
use crate::error::Result;
use crate::sha256::Hash;
use crate::util::{MerkleRoot, Saveable};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Error as IoError, ErrorKind as IoErrorKind, Read, Result as IoResult, Write};

/// A single block in the blockchain, containing a header and a list of transactions.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
}

impl Block {
    /// Creates a new block from the given header and transactions.
    pub fn new(header: BlockHeader, transactions: Vec<Transaction>) -> Self {
        Block {
            header,
            transactions,
        }
    }

    /// Returns the SHA-256 hash of this block.
    pub fn hash(&self) -> Hash {
        Hash::hash(self)
    }

    /// Verifies that all transactions in this block are valid against the current UTXO set.
    ///
    /// This must be called before adding a block to the chain. It performs four checks
    /// for every transaction:
    ///
    /// 1. **Referenced UTXOs exist** - every input must point to a known unspent output.
    ///    If an input references a hash not found in `utxos`, the block is rejected.
    ///
    /// 2. **No double-spends** - a UTXO cannot be spent more than once within the same
    ///    block. This catches cases that the global UTXO set alone cannot, since new
    ///    transactions in the same block haven't updated it yet.
    ///
    /// 3. **Signatures are valid** - each input must carry a signature that verifies
    ///    against the public key on the UTXO it is spending, proving the spender
    ///    owns the corresponding private key.
    ///
    /// 4. **Inputs >= outputs** - the total value of inputs must be at least as large
    ///    as the total value of outputs. Any excess is implicitly claimed by the miner
    ///    as a transaction fee.
    ///
    /// # Arguments
    ///
    /// * `utxos` - The current UTXO set, mapping output hashes to their transaction outputs.
    ///
    /// # Errors
    ///
    /// Returns [`BlockChainError::InvalidTransaction`] if any check fails.
    pub fn verify_transactions(
        &self,
        predicted_block_height: u64,
        utxos: &HashMap<Hash, (bool, TransactionOutput)>,
    ) -> Result<()> {
        let mut inputs: HashMap<Hash, TransactionOutput> = HashMap::new();
        if self.transactions.is_empty() {
            println!("Block has no transactions");
            return Err(BlockChainError::InvalidBlock);
        }

        self.verify_coinbase_transaction(predicted_block_height, utxos)?;

        for transaction in self.transactions.iter().skip(1) {
            let mut input_value = 0;
            let mut output_value = 0;

            for input in &transaction.inputs {
                let prev_output = utxos
                    .get(&input.prev_transaction_output_hash)
                    .map(|(_, output)| output);
                if prev_output.is_none() {
                    return Err(BlockChainError::InvalidTransaction);
                }
                let prev_output = prev_output.unwrap();
                if inputs.contains_key(&input.prev_transaction_output_hash) {
                    println!("Duplicate input: {:?}", input.prev_transaction_output_hash);
                    return Err(BlockChainError::InvalidTransaction);
                }
                if !input
                    .signature
                    .verify_output(&input.prev_transaction_output_hash, &prev_output.pubkey)
                {
                    return Err(BlockChainError::InvalidTransaction);
                }
                input_value += prev_output.value;
                inputs.insert(input.prev_transaction_output_hash, prev_output.clone());
            }
            for output in &transaction.outputs {
                output_value += output.value;
            }
            if input_value < output_value {
                println!("Input value is less than output value");
                return Err(BlockChainError::InvalidTransaction);
            }
        }
        Ok(())
    }

    /// Verifies the coinbase transaction in the block.
    ///
    /// Returns an error if the coinbase transaction has inputs or no outputs.
    ///
    /// # Arguments
    ///
    /// * `predicted_block_height` - The predicted height of the block.
    /// * `utxos` - A map of UTXOs to verify against.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the coinbase transaction is valid, otherwise returns an error.
    pub fn verify_coinbase_transaction(
        &self,
        predicted_block_height: u64,
        utxos: &HashMap<Hash, (bool, TransactionOutput)>,
    ) -> Result<()> {
        let coinbase_transaction = &self.transactions[0];
        if !coinbase_transaction.inputs.is_empty() {
            println!(
                "Coinbase transaction has inputs: {:?}",
                coinbase_transaction.inputs
            );
            return Err(BlockChainError::InvalidTransactionInput);
        }
        if coinbase_transaction.outputs.is_empty() {
            println!(
                "Coinbase transaction has outputs: {:?}",
                coinbase_transaction.outputs
            );
            return Err(BlockChainError::InvalidTransactionOutput);
        }
        let miner_fees = self.calculate_miner_fees(utxos)?;
        let block_reward = Block::calculate_block_reward(predicted_block_height);
        let total_coinbase_outputs: u64 = coinbase_transaction
            .outputs
            .iter()
            .map(|output| output.value)
            .sum();
        if total_coinbase_outputs != miner_fees + block_reward {
            println!(
                "Total coinbase outputs do not match miner fees + block reward: total={} vs miner_fees={} + block_reward={}",
                total_coinbase_outputs, miner_fees, block_reward
            );
            return Err(BlockChainError::InvalidTransactionOutput);
        }
        Ok(())
    }

    pub fn calculate_miner_fees(
        &self,
        utxos: &HashMap<Hash, (bool, TransactionOutput)>,
    ) -> Result<u64> {
        let mut inputs: HashMap<Hash, TransactionOutput> = HashMap::new();
        let mut outputs: HashMap<Hash, TransactionOutput> = HashMap::new();
        for transaction in self.transactions.iter().skip(1) {
            for input in &transaction.inputs {
                let prev_output = utxos
                    .get(&input.prev_transaction_output_hash)
                    .map(|(_, output)| output);
                if prev_output.is_none() {
                    return Err(BlockChainError::InvalidTransaction);
                }
                let prev_output = prev_output.unwrap();
                if inputs.contains_key(&input.prev_transaction_output_hash) {
                    return Err(BlockChainError::InvalidTransaction);
                }
                inputs.insert(input.prev_transaction_output_hash, prev_output.clone());
            }
            for output in &transaction.outputs {
                if outputs.contains_key(&output.hash()) {
                    return Err(BlockChainError::InvalidTransaction);
                }
                outputs.insert(output.hash(), output.clone());
            }
        }
        let input_value: u64 = inputs.values().map(|i| i.value).sum();
        let output_value: u64 = outputs.values().map(|o| o.value).sum();
        Ok(input_value - output_value)
    }

    pub fn calculate_block_reward(predicted_block_height: u64) -> u64 {
        crate::INITIAL_REWARD * 10u64.pow(8)
            / 2u64.pow((predicted_block_height / crate::HALVING_INTERVAL) as u32)
    }
}

impl Saveable for Block {
    fn load<I: Read>(reader: I) -> IoResult<Self> {
        ciborium::de::from_reader(reader)
            .map_err(|e| IoError::new(IoErrorKind::InvalidData, e.to_string()))
    }

    fn save<O: Write>(&self, writer: O) -> IoResult<()> {
        ciborium::ser::into_writer(self, writer)
            .map_err(|e| IoError::new(IoErrorKind::InvalidData, e.to_string()))
    }
}

/// Metadata for a block, used in hashing and proof-of-work validation.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlockHeader {
    /// When the block was created.
    pub timestamp: DateTime<Utc>,
    /// The value miners increment to find a hash that meets the target.
    pub nonce: u64,
    /// SHA-256 hash of the previous block, linking the chain together.
    pub prev_block_hash: Hash,
    /// Root of the Merkle tree of all transactions in this block.
    pub merkle_root: MerkleRoot,
    /// The difficulty target: a valid block hash must be less than or equal to this value.
    pub target: U256,
}

impl BlockHeader {
    /// Creates a new block header.
    pub fn new(
        timestamp: DateTime<Utc>,
        nonce: u64,
        prev_block_hash: Hash,
        merkle_root: MerkleRoot,
        target: U256,
    ) -> Self {
        BlockHeader {
            timestamp,
            nonce,
            prev_block_hash,
            merkle_root,
            target,
        }
    }

    pub fn mine(&mut self, steps: usize) -> bool {
        if self.hash().matches(self.target) {
            return true;
        }
        for _ in 0..steps {
            if let Some(new_nonce) = self.nonce.checked_add(1) {
                self.nonce = new_nonce;
            } else {
                self.nonce = 0;
                self.timestamp = Utc::now()
            }
            if self.hash().matches(self.target) {
                return true;
            }
        }
        false
    }

    pub fn mine_range(&mut self, steps: usize, start: u64, end: u64) -> bool {
        for _ in 0..steps {
            if self.hash().matches(self.target) {
                return true;
            }
            self.nonce = if self.nonce + 1 < end {
                self.nonce + 1
            } else {
                start
            };
        }
        false
    }

    /// Returns the SHA-256 hash of this block header.
    pub fn hash(&self) -> Hash {
        Hash::hash(self)
    }
}
