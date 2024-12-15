// src/error.rs
use thiserror::Error;
use tokio::time::error::Elapsed;

#[derive(Error, Debug)]
pub enum BitTorrentError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Tracker error: {0}")]
    Tracker(String),

    #[error("Peer error: {0}")]
    Peer(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("error: {0}")]
    Piece(String),

    #[error("Client error: {0}")]
    Client(String),

    #[error("Download error: {0}")]
    Download(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),

    #[error("Timeout error")]
    Timeout(#[from] Elapsed),

    #[error("Join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}

impl BitTorrentError {
    pub fn is_connection_error(&self) -> bool {
        matches!(self, BitTorrentError::Peer(msg) if msg.contains("connection"))
    }
}

pub type Result<T> = std::result::Result<T, BitTorrentError>;
