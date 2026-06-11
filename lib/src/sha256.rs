use crate::U256;
use serde::{Deserialize, Serialize};
use sha256::digest;
use std::fmt;

/// A SHA-256 hash represented as a 256-bit unsigned integer.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub struct Hash(U256);

impl Hash {
    /// Serializes `data` to CBOR and returns its SHA-256 hash.
    pub fn hash<T: serde::Serialize>(data: &T) -> Self {
        let mut serialized: Vec<u8> = vec![];

        if let Err(e) = ciborium::into_writer(data, &mut serialized) {
            panic!("Failed to serialized data: {}", e);
        }
        let hash = digest(&serialized);
        let hash_bytes = hex::decode(hash).unwrap();
        let hash_array: [u8; 32] = hash_bytes.as_slice().try_into().unwrap();
        Hash(U256::from_big_endian(&hash_array))
    }

    /// Returns `true` if this hash is less than or equal to `target`.
    ///
    /// Used in proof-of-work to check whether a block hash meets the difficulty target.
    pub fn matches(&self, target: U256) -> bool {
        self.0 <= target
    }

    pub fn as_bytes(&self) -> [u8; 32] {
        self.0.to_big_endian()
    }

    /// Returns a hash of all zeros, used as a placeholder
    pub fn zero() -> Self {
        Hash(U256::zero())
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:x}", self.0)
    }
}
