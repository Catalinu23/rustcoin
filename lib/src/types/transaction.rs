use crate::crypto::{PublicKey, Signature};
use crate::sha256::Hash;
use crate::util::Saveable;
use serde::{Deserialize, Serialize};
use std::io::{Error as IoError, ErrorKind as IoErrorKind, Read, Result as IoResult, Write};
use uuid::Uuid;

/// A Bitcoin transaction, moving value from inputs to outputs.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Transaction {
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
}

impl Transaction {
    /// Creates a new transaction from the given inputs and outputs.
    pub fn new(inputs: Vec<TransactionInput>, outputs: Vec<TransactionOutput>) -> Self {
        Transaction { inputs, outputs }
    }

    /// Returns the SHA-256 hash of this transaction.
    pub fn hash(&self) -> Hash {
        Hash::hash(self)
    }
}

impl Saveable for Transaction {
    fn load<I: Read>(reader: I) -> IoResult<Self> {
        ciborium::de::from_reader(reader)
            .map_err(|e| IoError::new(IoErrorKind::InvalidData, e.to_string()))
    }

    fn save<O: Write>(&self, writer: O) -> IoResult<()> {
        ciborium::ser::into_writer(self, writer)
            .map_err(|e| IoError::new(IoErrorKind::InvalidData, e.to_string()))
    }
}

/// A reference to a previous transaction output being spent.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TransactionInput {
    /// Hash of the transaction output this input is spending.
    pub prev_transaction_output_hash: Hash,
    /// ECDSA signature proving the owner authorized this spend (64 bytes, compact form).
    pub signature: Signature,
}

/// A destination for value in a transaction.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TransactionOutput {
    /// Amount of satoshis being sent.
    pub value: u64,
    /// Unique identifier for this output, used to reference it in future inputs.
    pub unique_id: Uuid,
    /// Recipient's compressed public key (33 bytes).
    pub pubkey: PublicKey,
}

impl TransactionOutput {
    /// Returns the SHA-256 hash of this transaction output.
    pub fn hash(&self) -> Hash {
        Hash::hash(self)
    }
}
