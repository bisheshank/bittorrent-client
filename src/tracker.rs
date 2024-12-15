// src/tracker.rs
use crate::{
    error::{BitTorrentError, Result},
    torrent::Torrent,
    utils::generate_peer_id,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddrV4;
use url::Url;

#[derive(Debug, Serialize)]
struct TrackerRequest<'a> {
    info_hash: &'a [u8; 20],
    peer_id: &'a [u8; 20],
    port: u16,
    uploaded: u64,
    downloaded: u64,
    left: u64,
    compact: u8,
    event: &'a str,
}

#[derive(Debug, Deserialize)]
struct TrackerResponse {
    #[serde(default)]
    interval: u32,
    #[serde(default)]
    min_interval: Option<u32>,
    #[serde(default)]
    complete: Option<u32>,
    #[serde(default)]
    incomplete: Option<u32>,
    #[serde(default)]
    warning_message: Option<String>,
    #[serde(default)]
    failure_reason: Option<String>,
    peers: Peers,
}

#[derive(Debug)]
struct Peers(Vec<SocketAddrV4>);

impl<'de> Deserialize<'de> for Peers {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct PeersVisitor;

        impl<'de> serde::de::Visitor<'de> for PeersVisitor {
            type Value = Peers;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("string or list of peer dictionaries")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v.len() % 6 != 0 {
                    return Err(E::custom(format!(
                        "peer string length must be multiple of 6, got {}",
                        v.len()
                    )));
                }

                let peers = v
                    .chunks_exact(6)
                    .filter_map(|chunk| {
                        let ip = std::net::Ipv4Addr::new(
                            chunk[0],
                            chunk[1],
                            chunk[2],
                            chunk[3],
                        );
                        let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                        Some(SocketAddrV4::new(ip, port))
                    })
                    .collect();

                Ok(Peers(peers))
            }

            // Optional: Also handle dictionary format if needed
            fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                #[derive(Deserialize)]
                struct PeerDict {
                    ip: String,
                    port: u16,
                    #[serde(default)]
                    peer_id: Option<String>,
                }

                let mut peers = Vec::new();
                while let Some(peer) = seq.next_element::<PeerDict>()? {
                    if let Ok(ip) = peer.ip.parse() {
                        peers.push(SocketAddrV4::new(ip, peer.port));
                    }
                }
                Ok(Peers(peers))
            }
        }

        deserializer.deserialize_any(PeersVisitor)
    }
}


#[derive(Debug, Deserialize)]
struct PeerDict {
    peer_id: Option<String>,
    ip: String,
    port: u16,
}

pub struct Tracker {
    announce_url: String,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
    port: u16,
    uploaded: u64,
    downloaded: u64,
    left: u64,
}

impl Tracker {
    pub fn new(torrent: &Torrent) -> Result<Self> {
        println!("Announce URL: {}", torrent.announce);
        Ok(Self {
            announce_url: torrent.announce.clone(),
            info_hash: torrent.info_hash(),
            peer_id: generate_peer_id(),
            port: 6881,
            uploaded: 0,
            downloaded: 0,
            left: torrent.total_length() as u64,
        })
    }

    pub async fn get_peers(&self) -> Result<Vec<SocketAddrV4>> {
        let request = TrackerRequest {
            info_hash: &self.info_hash,
            peer_id: &self.peer_id,
            port: self.port,
            uploaded: self.uploaded,
            downloaded: self.downloaded,
            left: self.left,
            compact: 1,
            event: "started",
        };

        let url = self.build_tracker_url(&request)?;
        println!("Requesting peers from tracker: {}", url);

        let response = reqwest::get(url).await?;
        println!("Response status: {}", response.status());
        
        let bytes = response.bytes().await?;
        let response: TrackerResponse = serde_bencode::from_bytes(&bytes)
            .map_err(|e| BitTorrentError::Tracker(format!("Failed to decode response: {}", e)))?;

        let peers = response.peers.0;
        println!("Successfully decoded {} peers", peers.len());
        
        if peers.is_empty() {
            return Err(BitTorrentError::Tracker("No peers available".into()));
        }

        Ok(peers)
    }

    fn build_tracker_url(&self, request: &TrackerRequest) -> Result<Url> {
        // Start with the base URL
        let base_url = Url::parse(&self.announce_url)
            .map_err(|e| BitTorrentError::Tracker(e.to_string()))?;
            
        let mut query = String::new();
        
        query.push_str("?info_hash=");
        for &byte in request.info_hash {
            query.push_str(&format!("%{:02x}", byte));
        }
        
        query.push_str("&peer_id=");
        for &byte in request.peer_id {
            query.push_str(&format!("%{:02x}", byte));
        }
        
        query.push_str(&format!("&port={}", request.port));
        query.push_str(&format!("&uploaded={}", request.uploaded));
        query.push_str(&format!("&downloaded={}", request.downloaded));
        query.push_str(&format!("&left={}", request.left));
        query.push_str(&format!("&compact={}", request.compact));
        query.push_str(&format!("&event={}", request.event));
        
        Ok(Url::parse(&format!("{}{}", base_url, query))
            .map_err(|e| BitTorrentError::Tracker(e.to_string()))?)
    }
}

fn urlencode(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| format!("%{:02x}", byte)).collect()
}

