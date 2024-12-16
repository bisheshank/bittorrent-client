use bittorrent::{client::Client, error::Result, term::TerminalUI};
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use tokio;

#[tokio::main]
async fn main() -> Result<()> {
    let torrent_path = env::args().nth(1).map(PathBuf::from).ok_or_else(|| {
        bittorrent::error::BitTorrentError::Client("Usage: bittorrent <torrent_file>".into())
    })?;

    // Initialize terminal UI
    let ui = Arc::new(TerminalUI::new());
    ui.start()?;

    // Create and set up client
    let client = Arc::new(tokio::sync::Mutex::new(Client::new(Arc::clone(&ui))));

    // Clone references for the async block
    let ui_clone = Arc::clone(&ui);
    let client_clone = Arc::clone(&client);

    ui.add_log(format!("Loading torrent file: {:?}", torrent_path));
    {
        let mut client_lock = client.lock().await;
        client_lock.add_torrent(&torrent_path).await?;
    }

    ui.add_log("Starting download...".to_string());

    // Spawn progress update task
    let ui_progress = Arc::clone(&ui);
    let progress_client = Arc::clone(&client);

    let progress_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(100));
        loop {
            interval.tick().await;

            // Lock the client only briefly to get the progress
            if let Ok(client_lock) = progress_client.try_lock() {
                if let Some(progress) = client_lock.progress() {
                    ui_progress.update_progress(progress);
                }
                if let Some(peer_count) = client_lock.get_active_peer_count() {
                    ui_progress.update_peers(peer_count);
                }
            }
        }
    });

    // Start the actual download
    let data = {
        let mut client_lock = client_clone.lock().await;
        client_lock.start_download().await?
    };

    // Write data to output file
    let output_path = torrent_path.with_extension("out");
    tokio::fs::write(&output_path, data).await?;

    // Cleanup
    progress_handle.abort(); // Stop the progress update task

    ui_clone.add_log(format!(
        "Download complete! Saved to: {}",
        output_path.display()
    ));
    std::thread::sleep(std::time::Duration::from_secs(2)); // Show final message

    Ok(())
}

