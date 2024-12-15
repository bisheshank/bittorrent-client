use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use tokio;
use tokio::sync::Mutex;
use bittorrent::{client::Client, error::Result};

// Usage:
// cargo run -- path/to/your/file.torrent

#[tokio::main]
async fn main() -> Result<()> {
    // Get torrent file path from command line arguments
    let torrent_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| bittorrent::error::BitTorrentError::Client("Usage: bittorrent <torrent_file>".into()))?;

    let client = Arc::new(Mutex::new(Client::new()));
    {
        let mut client_lock = client.lock().await;
        client_lock.add_torrent(&torrent_path).await?;
    }

    // Start the download
    println!("Starting download...");
    let data = {
        let mut client_lock = client.lock().await;
        client_lock.start_download().await?
    };

    // Write data to output file
    let output_path = torrent_path.with_extension("out");
    tokio::fs::write(&output_path, data).await?;

    println!("Download complete! Saved to: {}", output_path.display());
    Ok(())
}
