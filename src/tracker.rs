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
    #[serde(rename = "failure reason")]
    failure_reason: Option<String>,
    #[serde(default)]
    peers: PeerList,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PeerList {
    Binary(Vec<u8>),
    Dict(Vec<PeerDict>),
}

impl Default for PeerList {
    fn default() -> Self {
        PeerList::Binary(Vec::new())
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
        println!("Contacting tracker: {}", self.announce_url);
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
        println!("Full tracker URL: {}", url);

        // Create a client with debug headers
        let client = reqwest::Client::builder()
            .user_agent("BitTorrent/1.0")
            .build()?;

        // Make the request and print detailed response info
        let response = client.get(url).send().await?;

        println!("Response status: {}", response.status());
        println!("Response headers: {:#?}", response.headers());

        // Get the response body
        let body = response.bytes().await?;
        println!("Response length: {}", body.len());

        if body.is_empty() {
            return Err(BitTorrentError::Tracker(
                "Empty response from tracker".into(),
            ));
        }

        let response: TrackerResponse = match serde_bencode::from_bytes::<TrackerResponse>(&body) {
            Ok(resp) => {
                println!("Successfully decoded tracker response");
                println!("Interval: {} seconds", resp.interval);
                if let Some(min_interval) = resp.min_interval {
                    println!("Min interval: {} seconds", min_interval);
                }
                if let Some(complete) = resp.complete {
                    println!("Complete (seeders): {}", complete);
                }
                if let Some(incomplete) = resp.incomplete {
                    println!("Incomplete (leechers): {}", incomplete);
                }
                resp
            }
            Err(e) => {
                println!("Failed to decode response: {:?}", e);
                println!("Raw response: {:?}", String::from_utf8_lossy(&body));
                return Err(BitTorrentError::Tracker(e.to_string()));
            }
        };

        match response.peers {
            PeerList::Binary(bytes) => {
                if bytes.len() % 6 != 0 {
                    return Err(BitTorrentError::Tracker("Invalid peers data".into()));
                }

                Ok(bytes
                    .chunks_exact(6)
                    .filter_map(|chunk| {
                        let ip = std::net::Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]);
                        let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                        Some(SocketAddrV4::new(ip, port))
                    })
                    .collect())
            }
            PeerList::Dict(peers) => Ok(peers
                .into_iter()
                .filter_map(|peer| {
                    let ip = peer.ip.parse().ok()?;
                    Some(SocketAddrV4::new(ip, peer.port))
                })
                .collect()),
        }
    }

    fn build_tracker_url(&self, request: &TrackerRequest) -> Result<Url> {
        let base_url = format!("{}?", self.announce_url);

        // Manually build query string to avoid double encoding
        let query = format!(
            "info_hash={}&peer_id={}&port={}&uploaded={}&downloaded={}&left={}&compact={}&event={}",
            urlencode(request.info_hash),
            urlencode(request.peer_id),
            request.port,
            request.uploaded,
            request.downloaded,
            request.left,
            request.compact,
            request.event
        );

        let url_str = base_url + &query;

        println!("Direct URL: {}", url_str);

        // Parse the complete URL
        Ok(Url::parse(&url_str).map_err(|e| BitTorrentError::Tracker(e.to_string()))?)
    }
}

fn urlencode(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| format!("%{:02x}", byte)).collect()
}
