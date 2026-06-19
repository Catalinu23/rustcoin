use lib::crypto::{PrivateKey, Signature};
use lib::sha256::Hash;

fn arbitrary_hash() -> Hash {
    Hash::hash(&"test message")
}

#[test]
fn sign_verify_roundtrip() {
    let private_key = PrivateKey::new();
    let public_key = private_key.public_key();
    let hash = arbitrary_hash();

    let signature = Signature::sign_output(&hash, &private_key);

    assert!(signature.verify_output(&hash, &public_key));
}

#[test]
fn verify_fails_wrong_key() {
    let signer = PrivateKey::new();
    let other = PrivateKey::new();
    let hash = arbitrary_hash();

    let signature = Signature::sign_output(&hash, &signer);

    assert!(!signature.verify_output(&hash, &other.public_key()));
}

#[test]
fn verify_fails_tampered_hash() {
    let private_key = PrivateKey::new();
    let public_key = private_key.public_key();
    let hash = arbitrary_hash();
    let different_hash = Hash::hash(&"different message");

    let signature = Signature::sign_output(&hash, &private_key);

    assert!(!signature.verify_output(&different_hash, &public_key));
}
