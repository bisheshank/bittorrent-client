// src/lib.rs
pub mod error;
pub mod torrent;
pub mod tracker;
pub mod peer;
pub mod piece;
pub mod message;
pub mod download;
pub mod client;
pub mod utils;
pub mod term;

#[cfg(test)]
pub mod tests {
    pub mod common;
}

use std::sync::atomic::{AtomicU64, Ordering};

// Standard BitTorrent constants
pub const BLOCK_SIZE: usize = 16_384; // 16KB
pub const MAX_MESSAGE_SIZE: usize = 1_048_576; // 1MB
pub const DEFAULT_PORT_RANGE: (u16, u16) = (6881, 6889);
pub const PEER_ID_PREFIX: &[u8; 8] = b"-RS0001-";
pub const MAX_PEERS: usize = 100;

