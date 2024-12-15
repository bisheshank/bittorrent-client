use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::path::Path;
use crate::error::BitTorrentError;
use serde::de::{self, Deserializer, Visitor};
use serde::ser::Serializer;
use std::fmt;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Torrent {
    pub announce: String,
    pub info: Info,
    #[serde(skip)]
    info_hash: Option<[u8; 20]>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Info {
    pub name: String,
    #[serde(rename = "piece length")]
    pub piece_length: usize,
    pub pieces: PieceHashes,
    #[serde(flatten)]
    pub files: FileMode,
}

impl fmt::Display for Info {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Torrent: {}\n", self.name)?;
        write!(f, "Piece length: {} bytes\n", self.piece_length)?;
        write!(f, "Number of pieces: {}\n", self.pieces.0.len())?;
        
        match &self.files {
            FileMode::SingleFile { length } => {
                write!(f, "Mode: Single File\n")?;
                write!(f, "Total size: {} bytes", length)
            }
            FileMode::MultiFile { files } => {
                write!(f, "Mode: Multi File\n")?;
                write!(f, "Number of files: {}\n", files.len())?;
                write!(f, "Total size: {} bytes", files.iter().map(|f| f.length).sum::<usize>())?;
                
                // Optionally list the files if you want more detail
                if f.alternate() {
                    for file in files {
                        write!(f, "\n  - {} ({} bytes)", 
                            file.path.join("/"), 
                            file.length
                        )?;
                    }
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum FileMode {
    SingleFile { length: usize },
    MultiFile { files: Vec<FileInfo> },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileInfo {
    pub length: usize,
    pub path: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PieceHashes(pub Vec<[u8; 20]>);

// Custom visitor for PieceHashes deserialization
struct PieceHashesVisitor;

impl<'de> Visitor<'de> for PieceHashesVisitor {
    type Value = PieceHashes;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a byte string whose length is a multiple of 20")
    }

    fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if v.len() % 20 != 0 {
            return Err(E::custom(format!(
                "piece hashes length must be multiple of 20, got {}",
                v.len()
            )));
        }

        let hashes = v
            .chunks_exact(20)
            .map(|chunk| {
                chunk.try_into()
                    .expect("chunk must be exactly 20 bytes due to chunks_exact")
            })
            .collect();

        Ok(PieceHashes(hashes))
    }
}

// Custom Deserialize implementation for PieceHashes
impl<'de> Deserialize<'de> for PieceHashes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_bytes(PieceHashesVisitor)
    }
}

// Custom Serialize implementation for PieceHashes
impl Serialize for PieceHashes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let bytes: Vec<u8> = self.0.iter().flat_map(|hash| hash.iter().copied()).collect();
        serializer.serialize_bytes(&bytes)
    }
}

impl Torrent {
    pub async fn from_file(path: impl AsRef<Path>) -> crate::error::Result<Self> {
        let data = tokio::fs::read(path)
            .await
            .map_err(|e| BitTorrentError::Io(e))?;
            
        let mut torrent: Self = serde_bencode::from_bytes(&data)
            .map_err(|e| BitTorrentError::InvalidData(e.to_string()))?;
            
        torrent.info_hash = Some(torrent.calculate_info_hash());
        Ok(torrent)
    }

    pub fn info_hash(&self) -> [u8; 20] {
        self.info_hash.unwrap_or_else(|| self.calculate_info_hash())
    }

    fn calculate_info_hash(&self) -> [u8; 20] {
        let info_bencoded = serde_bencode::to_bytes(&self.info)
            .expect("Info serialization should never fail");
            
        let mut hasher = Sha1::new();
        hasher.update(&info_bencoded);
        hasher.finalize()
            .try_into()
            .expect("SHA1 output is always 20 bytes")
    }

    pub fn total_length(&self) -> usize {
        match &self.info.files {
            FileMode::SingleFile { length } => *length,
            FileMode::MultiFile { files } => files.iter().map(|f| f.length).sum(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_bencode;

    #[test]
    fn test_piece_hashes_serde() {
        // Create sample piece hashes
        let hashes = vec![[1u8; 20], [2u8; 20]];
        let piece_hashes = PieceHashes(hashes.clone());

        // Serialize
        let encoded = serde_bencode::to_bytes(&piece_hashes).unwrap();
        
        // Deserialize
        let decoded: PieceHashes = serde_bencode::from_bytes(&encoded).unwrap();
        
        // Verify
        assert_eq!(decoded.0, hashes);
    }

    #[test]
    fn test_invalid_piece_hashes() {
        // Create invalid data (not multiple of 20 bytes)
        let invalid_data = vec![1u8; 30];
        let result: Result<PieceHashes, _> = serde_bencode::from_bytes(&invalid_data);
        assert!(result.is_err());
    }
}
