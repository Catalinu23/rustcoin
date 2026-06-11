use ecdsa::{
    Signature as ECDSASignature, SigningKey, VerifyingKey,
    signature::{Signer, Verifier},
};
use k256::Secp256k1;
use serde::{Deserialize, Serialize};
use std::io::{Error as IoError, ErrorKind as IoErrorKind, Read, Result as IoResult, Write};

use crate::sha256::Hash;
use crate::util::Saveable;

/// An ECDSA signature over the secp256k1 curve, produced when signing a transaction.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Signature(ECDSASignature<Secp256k1>);

impl Signature {
    /// Signs the output hash using the given private key.
    pub fn sign_output(output_hash: &Hash, private_key: &PrivateKey) -> Self {
        let signing_key = &private_key.0;
        let hash_bytes = output_hash.as_bytes();
        let signature = signing_key.sign(&hash_bytes);
        Self(signature)
    }

    /// Verifies the output hash against the given public key.
    pub fn verify_output(&self, output_hash: &Hash, public_key: &PublicKey) -> bool {
        let verifying_key = &public_key.0;
        let hash_bytes = output_hash.as_bytes();
        verifying_key.verify(&hash_bytes, &self.0).is_ok()
    }
}

/// A public key on the secp256k1 curve, used to verify signatures and derive addresses.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PublicKey(VerifyingKey<Secp256k1>);

/// A private key on the secp256k1 curve, used to sign transactions.
///
/// Uses a custom serde implementation (`signkey_serde`) because `SigningKey`
/// doesn't implement `Serialize`/`Deserialize` directly.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PrivateKey(#[serde(with = "signkey_serde")] pub SigningKey<Secp256k1>);

impl PrivateKey {
    pub fn new() -> Self {
        Self(SigningKey::random(&mut rand::thread_rng()))
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.0.verifying_key().clone())
    }
}

impl Saveable for PublicKey {
    fn load<I: Read>(reader: I) -> IoResult<Self> {
        ciborium::de::from_reader(reader)
            .map_err(|e| IoError::new(IoErrorKind::InvalidData, e.to_string()))
    }

    fn save<O: Write>(&self, writer: O) -> IoResult<()> {
        ciborium::ser::into_writer(self, writer)
            .map_err(|e| IoError::new(IoErrorKind::InvalidData, e.to_string()))
    }
}

impl Saveable for PrivateKey {
    fn load<I: Read>(reader: I) -> IoResult<Self> {
        ciborium::de::from_reader(reader)
            .map_err(|e| IoError::new(IoErrorKind::InvalidData, e.to_string()))
    }

    fn save<O: Write>(&self, writer: O) -> IoResult<()> {
        ciborium::ser::into_writer(self, writer)
            .map_err(|e| IoError::new(IoErrorKind::InvalidData, e.to_string()))
    }
}

/// Custom serde implementation for `SigningKey<Secp256k1>`.
///
/// `SigningKey` doesn't implement serde natively, so we manually serialize it
/// as raw bytes and reconstruct it on deserialization.
mod signkey_serde {
    use serde::Deserialize;

    /// Serializes a `SigningKey` as its raw 32-byte scalar representation.
    pub fn serialize<S>(
        key: &super::SigningKey<super::Secp256k1>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bytes(&key.to_bytes())
    }

    /// Deserializes a `SigningKey` from raw bytes.
    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<super::SigningKey<super::Secp256k1>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes: Vec<u8> = Vec::<u8>::deserialize(deserializer)?;
        super::SigningKey::from_slice(&bytes).map_err(serde::de::Error::custom)
    }
}
