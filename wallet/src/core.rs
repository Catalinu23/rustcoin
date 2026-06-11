use anyhow::{Result, anyhow};
use lib::crypto::{PrivateKey, PublicKey, Signature};
use lib::network::Message;
use lib::types::{Transaction, TransactionInput, TransactionOutput};
use lib::util::Saveable;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone)]
pub struct Key {
    pub private_key_path: PathBuf,
    pub public_key_path: PathBuf,
}

impl Key {
    /// Deserializes the private and public key files named in this config entry.
    pub fn load(&self) -> Result<LoadedKey> {
        let private_key = PrivateKey::load_from_file(&self.private_key_path)?;
        let public_key = PublicKey::load_from_file(&self.public_key_path)?;
        Ok(LoadedKey {
            private_key,
            public_key,
        })
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LoadedKey {
    pub private_key: PrivateKey,
    pub public_key: PublicKey,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Recipient {
    pub name: String,
    pub public_key_path: PathBuf,
}

impl Recipient {
    /// Deserializes the public key file named in this contact entry.
    pub fn load(&self) -> Result<LoadedRecipient> {
        let public_key = PublicKey::load_from_file(&self.public_key_path)?;
        Ok(LoadedRecipient {
            name: self.name.clone(),
            public_key,
        })
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LoadedRecipient {
    pub name: String,
    pub public_key: PublicKey,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum FeeType {
    Fixed,
    Percentage,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FeeConfig {
    pub fee_type: FeeType,
    pub value: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub node: String,
    pub keys: Vec<Key>,
    pub contacts: Vec<Recipient>,
    pub fee: FeeConfig,
}

/// Holds all key pairs managed by this wallet and the last-known UTXOs for each,
/// keyed by the CBOR-encoded public key bytes (used in place of PublicKey directly
/// because PublicKey does not implement Ord or Hash).
pub(crate) struct UtxoStore {
    my_keys: Vec<LoadedKey>,
    utxos: HashMap<Vec<u8>, Vec<(TransactionOutput, bool)>>,
}

impl UtxoStore {
    /// Creates an empty store with no keys or UTXOs.
    fn new() -> Self {
        Self {
            my_keys: Vec::new(),
            utxos: HashMap::new(),
        }
    }

    /// Registers a key pair so that its UTXOs will be fetched and tracked.
    fn add_key(&mut self, key: LoadedKey) {
        self.my_keys.push(key);
    }

    /// Serialises a public key to bytes for use as a map key.
    fn pubkey_bytes(key: &PublicKey) -> Vec<u8> {
        let mut buf = Vec::new();
        key.save(&mut buf).unwrap_or_default();
        buf
    }
}

pub struct Core {
    pub config: Config,
    utxo_store: RwLock<UtxoStore>,
    /// Flat, ordered list of all UTXOs across every managed key, refreshed by `fetch_utxos`.
    pub utxos: RwLock<Vec<(TransactionOutput, bool)>>,
    pub tx_sender: kanal::AsyncSender<Transaction>,
}

impl Core {
    /// Returns the node address from the config (may be overridden after construction).
    pub fn node_addr(&self) -> &str {
        &self.config.node
    }

    /// Constructs a Core from an already-loaded config and UTXO store.
    /// `tx_sender` is the write end of the outbound transaction channel; the caller
    /// keeps the matching `AsyncReceiver` and passes it to `handle_transactions`.
    pub fn new(
        config: Config,
        utxos: UtxoStore,
        tx_sender: kanal::AsyncSender<Transaction>,
    ) -> Self {
        Self {
            config,
            utxo_store: RwLock::new(utxos),
            utxos: RwLock::new(Vec::new()),
            tx_sender,
        }
    }

    /// Reads `wallet.json` (or the path provided), deserialises the config, loads every
    /// key pair from disk, and wires up the transaction channel.
    /// Returns the Core together with the receiver end of the transaction channel so the
    /// caller can hand it off to `handle_transactions`.
    pub fn load(config_path: PathBuf) -> Result<(Self, kanal::AsyncReceiver<Transaction>)> {
        let config_str = std::fs::read_to_string(&config_path)
            .map_err(|e| anyhow!("Failed to read '{}': {e}", config_path.display()))?;
        let config: Config = serde_json::from_str(&config_str)?;

        let mut store = UtxoStore::new();
        for key in &config.keys {
            store.add_key(key.load()?);
        }

        let (tx_s, tx_r) = kanal::bounded::<Transaction>(100);
        Ok((Self::new(config, store, tx_s.to_async()), tx_r.to_async()))
    }

    /// Connects to the node once per managed key, sends `FetchUTXOs`, and merges the
    /// results into both the per-key map (inside `UtxoStore`) and the flat `self.utxos`
    /// cache that the rest of the wallet reads. Returns an error if any connection fails.
    pub async fn fetch_utxos(&self) -> Result<()> {
        // Collect public keys first so the immutable borrow on `store` is released
        // before we take a mutable borrow to write the results back.
        let pub_keys: Vec<PublicKey> = {
            let store = self.utxo_store.read().await;
            store.my_keys.iter().map(|k| k.public_key.clone()).collect()
        };

        let mut all_utxos: Vec<(TransactionOutput, bool)> = Vec::new();
        let mut per_key: Vec<(Vec<u8>, Vec<(TransactionOutput, bool)>)> = Vec::new();

        for pub_key in &pub_keys {
            let mut stream = TcpStream::connect(&self.config.node)
                .await
                .map_err(|e| anyhow!("Failed to connect to node: {e}"))?;

            Message::FetchUTXOs(pub_key.clone())
                .send_async(&mut stream)
                .await
                .map_err(|e| anyhow!("Failed to send FetchUTXOs: {e}"))?;

            if let Ok(Message::UTXOs(utxos)) = Message::receive_async(&mut stream).await {
                per_key.push((UtxoStore::pubkey_bytes(pub_key), utxos.clone()));
                all_utxos.extend(utxos);
            }
        }

        let mut store = self.utxo_store.write().await;
        for (key_bytes, utxos) in per_key {
            store.utxos.insert(key_bytes, utxos);
        }
        *self.utxos.write().await = all_utxos;
        Ok(())
    }

    /// Opens a fresh TCP connection to the node and submits the transaction via
    /// `SubmitTransaction`. The node validates and propagates it to peers.
    pub async fn send_transaction(&self, transaction: Transaction) -> Result<()> {
        let mut stream = TcpStream::connect(&self.config.node)
            .await
            .map_err(|e| anyhow!("Failed to connect to node: {e}"))?;
        Message::SubmitTransaction(transaction)
            .send_async(&mut stream)
            .await
            .map_err(|e| anyhow!("Failed to submit transaction: {e}"))?;
        Ok(())
    }

    /// Sums the values of all unspent UTXOs across every managed key.
    pub async fn get_balance(&self) -> u64 {
        self.utxos
            .read()
            .await
            .iter()
            .filter(|(_, spent)| !spent)
            .map(|(out, _)| out.value)
            .sum()
    }

    /// Selects UTXOs greedily to cover `amount + fee`, signs each input with the key
    /// that owns the corresponding output, and produces a change output back to the
    /// first managed key if there is a surplus. Returns an error if funds are insufficient.
    pub async fn create_transaction(
        &self,
        recipient: &PublicKey,
        amount: u64,
    ) -> Result<Transaction> {
        let fee = self.calculate_fee(amount);
        let total_needed = amount + fee;

        let utxos = self.utxos.read().await;
        let store = self.utxo_store.read().await;

        let mut selected: Vec<TransactionOutput> = Vec::new();
        let mut collected = 0u64;

        for (out, spent) in utxos.iter() {
            if *spent {
                continue;
            }
            collected += out.value;
            selected.push(out.clone());
            if collected >= total_needed {
                break;
            }
        }

        if collected < total_needed {
            return Err(anyhow!(
                "Insufficient funds: have {} satoshis, need {}",
                collected,
                total_needed
            ));
        }

        let inputs: Vec<TransactionInput> = selected
            .iter()
            .map(|out| {
                let signing_key = store
                    .my_keys
                    .iter()
                    .find(|k| k.public_key == out.pubkey)
                    .unwrap_or(&store.my_keys[0]);
                let output_hash = out.hash();
                TransactionInput {
                    prev_transaction_output_hash: output_hash,
                    signature: Signature::sign_output(&output_hash, &signing_key.private_key),
                }
            })
            .collect();

        let mut outputs = vec![TransactionOutput {
            value: amount,
            unique_id: Uuid::new_v4(),
            pubkey: recipient.clone(),
        }];

        let change = collected - total_needed;
        if change > 0 {
            outputs.push(TransactionOutput {
                value: change,
                unique_id: Uuid::new_v4(),
                pubkey: store.my_keys[0].public_key.clone(),
            });
        }

        Ok(Transaction::new(inputs, outputs))
    }

    /// Applies the fee policy from the config: a flat satoshi amount for `Fixed`,
    /// or a percentage of the payment amount for `Percentage`.
    fn calculate_fee(&self, amount: u64) -> u64 {
        match self.config.fee.fee_type {
            FeeType::Fixed => self.config.fee.value as u64,
            FeeType::Percentage => (amount as f64 * self.config.fee.value / 100.0) as u64,
        }
    }
}
