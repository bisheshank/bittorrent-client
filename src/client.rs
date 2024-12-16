// src/client.rs
use crate::{
    download::Download, error::{BitTorrentError, Result}, term::TerminalUI, torrent::Torrent
};
use std::path::Path;
use std::sync::Arc;
pub struct Client {
    download: Option<Download>,
    ui: Arc<TerminalUI>,
}

impl Client {
    pub fn new(ui: Arc<TerminalUI>) -> Self {
        Self { 
            download: None,
            ui,
        }
    }

    pub async fn add_torrent(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.ui.add_log(format!("Loading torrent file: {:?}", path.as_ref()));
        let torrent = Torrent::from_file(path).await?;
        self.ui.add_log(format!("Torrent loaded successfully. Info: {}", torrent.info));
        let download = Download::new(torrent, Arc::clone(&self.ui)).await?;
        self.download = Some(download);
        Ok(())
    }

    pub async fn start_download(&mut self) -> Result<Vec<u8>> {
        if let Some(download) = &mut self.download {
            download.download_all().await
        } else {
            Err(BitTorrentError::Client("No torrent loaded".into()))
        }
    }

    pub fn progress(&self) -> Option<f64> {
        self.download.as_ref().map(|d| d.progress())
    }

    pub fn get_active_peer_count(&self) -> Option<usize> {
        self.download.as_ref().map(|d| d.get_active_peer_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
}
