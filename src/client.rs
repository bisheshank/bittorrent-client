// src/client.rs
use crate::{
    download::Download,
    error::{BitTorrentError, Result},
    torrent::Torrent,
};
use std::path::Path;

pub struct Client {
    download: Option<Download>,
}

impl Client {
    pub fn new() -> Self {
        Self { download: None }
    }

    pub async fn add_torrent(&mut self, path: impl AsRef<Path>) -> Result<()> {
        println!("Loading torrent file: {:?}", path.as_ref());
        let torrent = Torrent::from_file(path).await?;
        println!("Torrent loaded successfully. Info: {}", torrent.info);
        let download = Download::new(torrent).await?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_download_single_file() {
        let mut client = Client::new();
        client.add_torrent("src/tests/data/test.torrent").await.unwrap();
        let data = client.start_download().await.unwrap();
        assert!(!data.is_empty());
    }
}
