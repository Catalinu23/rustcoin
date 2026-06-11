use serde::{Deserialize, Serialize};
use uint::construct_uint;

construct_uint! {
    #[derive(Serialize, Deserialize)]
    pub struct U256(4);
}

pub const INITIAL_REWARD: u64 = 50;
pub const HALVING_INTERVAL: u64 = 210;
pub const IDEAL_BLOCK_TIME: u64 = 10;
pub const DIFFICULTY_UPDATE_INTERVAL: u64 = 50;
pub const MAX_MEMPOOL_TRANSACTION_AGE: u64 = 3600;
pub const BLOCK_TRANSACTION_CAP: usize = 20;

pub const MIN_TARGET: U256 = U256([
    0xFFFF_FFFF_FFFF_FFFF,
    0xFFFF_FFFF_FFFF_FFFF,
    0xFFFF_FFFF_FFFF_FFFF,
    0x0000_0FFF_FFFF_FFFF,
]);

pub mod crypto;
pub mod error;
pub mod network;
pub mod sha256;
pub mod types;
pub mod util;
