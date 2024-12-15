use crate::{
    error::{BitTorrentError, Result},
    peer::Peer,
    piece::PieceManager,
    torrent::Torrent,
    tracker::Tracker,
    utils::{bit_set, generate_peer_id},
    MAX_PEERS,
};
use tokio::time::{Duration, timeout};
use futures_util::{stream::FuturesUnordered, StreamExt};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

#[derive(Debug)]
enum TaskMessage {
    PieceCompleted {
        index: usize,
        data: Vec<u8>,
    },
    PieceFailed {
        index: usize,
        error: BitTorrentError,
    },
    PeerDisconnected {
        peer_id: usize,
    },
}

pub struct Download {
    torrent: Torrent,
    peers: Vec<Peer>,
    piece_manager: PieceManager,
    downloaded: Vec<u8>,
}

// src/download.rs

impl Download {
    pub async fn new(torrent: Torrent) -> Result<Self> {
        println!("Initializing download for: {}", torrent.info.name);
        let tracker = Tracker::new(&torrent)?;
        println!("Connecting to tracker at: {}", torrent.announce);
        let peer_list = tracker.get_peers().await?;
        println!("Found {} potential peers", peer_list.len());
        let info_hash = torrent.info_hash();
        let mut peers = Vec::new();

        // Try to connect to peers concurrently with a timeout
        let connect_timeout = Duration::from_secs(30);
        let mut peer_futures = FuturesUnordered::new();
        
        // Start with a smaller batch of connection attempts
        for addr in peer_list.iter().take(25) {
            peer_futures.push(Peer::connect(*addr, info_hash, generate_peer_id()));
        }

        let mut connected_peers = 0;
        let start_time = std::time::Instant::now();

        while let Some(result) = timeout(connect_timeout, peer_futures.next()).await? {
            match result {
                Ok(peer) => {
                    peers.push(peer);
                    connected_peers += 1;
                    if connected_peers >= MAX_PEERS {
                        break;
                    }
                }
                Err(e) => {
                    println!("Failed to connect to peer: {}", e);
                }
            }

            // If we're running low on connection attempts and haven't reached our target,
            // add more peers to try
            if peer_futures.len() < 5 && connected_peers < MAX_PEERS {
                let next_peers = peer_list.iter()
                    .skip(25 + connected_peers)
                    .take(5);
                
                for addr in next_peers {
                    peer_futures.push(Peer::connect(*addr, info_hash, generate_peer_id()));
                }
            }

            // Break if we've been trying too long
            if start_time.elapsed() > Duration::from_secs(60) {
                println!("Connection phase timeout reached");
                break;
            }
        }

        if peers.is_empty() {
            return Err(BitTorrentError::Peer("No peers available".into()));
        }

        println!("Successfully connected to {} peers", peers.len());

        // Initialize piece manager and download buffer
        let piece_manager = PieceManager::new(
            torrent.info.piece_length,
            torrent.info.pieces.0.clone(),
            torrent.total_length(),
        );

        let downloaded = vec![0; torrent.total_length()];

        Ok(Self {
            torrent,
            peers,
            piece_manager,
            downloaded,
        })
    }

    async fn start_download_tasks(
        peers: Vec<Peer>,
        piece_manager: Arc<Mutex<PieceManager>>,
        task_concurrency: usize,
    ) -> mpsc::Receiver<TaskMessage> {
        let (tx, rx) = mpsc::channel(task_concurrency * 2);

        for (peer_id, mut peer) in peers.into_iter().enumerate() {
            let tx = tx.clone();
            let piece_manager = Arc::clone(&piece_manager);

            tokio::spawn(async move {
                let mut active = true;
                
                while active {
                    // Get next piece under lock
                    let piece = {
                        let mut pm = piece_manager.lock().await;
                        pm.next_piece(peer_id)
                    };

                    match piece {
                        Some(piece) => {
                            match peer.request_piece(&piece).await {
                                Ok(data) => {
                                    let verified = {
                                        let pm = piece_manager.lock().await;
                                        pm.verify_piece(piece.index(), &data)
                                    };

                                    if verified {
                                        let msg = TaskMessage::PieceCompleted {
                                            index: piece.index(),
                                            data,
                                        };
                                        if tx.send(msg).await.is_err() {
                                            break;
                                        }
                                    } else {
                                        let msg = TaskMessage::PieceFailed {
                                            index: piece.index(),
                                            error: BitTorrentError::Piece("Verification failed".into()),
                                        };
                                        if tx.send(msg).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                Err(e) => {
                                    if e.is_connection_error() {
                                        let msg = TaskMessage::PeerDisconnected { peer_id };
                                        let _ = tx.send(msg).await;
                                        active = false;
                                    } else {
                                        let msg = TaskMessage::PieceFailed {
                                            index: piece.index(),
                                            error: e,
                                        };
                                        if tx.send(msg).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        None => {
                            active = false;
                        }
                    }
                }

                // Clean up peer when done
                let mut pm = piece_manager.lock().await;
                pm.remove_peer(peer_id);
            });
        }

        rx
    }

    pub async fn download_all(&mut self) -> Result<Vec<u8>> {
        println!("\nStarting download of: {}", self.torrent.info.name);
        println!("Total size: {} bytes", self.torrent.total_length());
        println!("Number of pieces: {}", self.torrent.info.pieces.0.len());

        let piece_manager = Arc::new(Mutex::new(std::mem::take(&mut self.piece_manager)));
        
        // Initialize piece manager with peers
        {
            let mut pm = piece_manager.lock().await;
            for (peer_id, peer) in self.peers.iter().enumerate() {
                pm.register_peer(peer_id);
                if let Some(bitfield) = peer.get_bitfield() {
                    for piece_index in 0..self.torrent.info.pieces.0.len() {
                        if bit_set(bitfield, piece_index) {
                            pm.add_peer_piece(peer_id, piece_index);
                        }
                    }
                }
            }
        }

        let mut rx = Self::start_download_tasks(
            std::mem::take(&mut self.peers),
            Arc::clone(&piece_manager),
            5,
        ).await;

        let total_pieces = self.torrent.info.pieces.0.len();
        let mut completed_pieces = 0;
        let mut failed_pieces = Vec::new();

        while let Some(message) = rx.recv().await {
            match message {
                TaskMessage::PieceCompleted { index, data } => {
                    self.store_piece(index, data);
                    {
                        let mut pm = piece_manager.lock().await;
                        pm.mark_completed(index);
                    }
                    completed_pieces += 1;
                    println!(
                        "Downloaded piece {}/{} ({:.1}%)",
                        completed_pieces,
                        total_pieces,
                        (completed_pieces as f64 / total_pieces as f64) * 100.0
                    );

                    if completed_pieces == total_pieces {
                        break;
                    }
                }
                TaskMessage::PieceFailed { index, error } => {
                    println!("Failed to download piece {}: {:?}", index, error);
                    failed_pieces.push(index);
                }
                TaskMessage::PeerDisconnected { peer_id } => {
                    println!("Peer {} disconnected", peer_id);
                }
            }
        }

        // Handle endgame mode if needed
        if !failed_pieces.is_empty() {
            self.piece_manager = Arc::try_unwrap(piece_manager)
                .expect("All tasks should be done")
                .into_inner();
            
            println!("Entering endgame mode for {} remaining pieces", failed_pieces.len());
            self.handle_endgame(&failed_pieces).await?;
        }

        let is_complete = failed_pieces.is_empty() && completed_pieces == total_pieces;
        
        if is_complete {
            Ok(self.downloaded.clone())
        } else {
            Err(BitTorrentError::Download("Failed to download all pieces".into()))
        }
    }

    async fn handle_endgame(&mut self, failed_pieces: &[usize]) -> Result<()> {
        for &piece_index in failed_pieces {
            let piece_info = self.piece_manager
                .get_piece(piece_index)
                .ok_or_else(|| BitTorrentError::Piece("Piece info not found".into()))?;

            let mut success = false;
            for peer in &mut self.peers {
                if peer.has_piece(piece_index) {
                    match peer.request_piece(&piece_info).await {
                        Ok(data) => {
                            if self.piece_manager.verify_piece(piece_index, &data) {
                                self.store_piece(piece_index, data);
                                self.piece_manager.mark_completed(piece_index);
                                success = true;
                                break;
                            }
                        }
                        Err(e) => {
                            println!("Endgame: Failed to download piece {} from peer: {:?}", piece_index, e);
                            continue;
                        }
                    }
                }
            }

            if !success {
                return Err(BitTorrentError::Piece(
                    format!("Failed to download piece {} in endgame mode", piece_index)
                ));
            }
        }

        Ok(())
    }

    fn store_piece(&mut self, index: usize, data: Vec<u8>) {
        let start = index * self.torrent.info.piece_length;
        if start + data.len() > self.downloaded.len() {
            self.downloaded.resize(start + data.len(), 0);
        }
        self.downloaded[start..start + data.len()].copy_from_slice(&data);
    }

    pub fn progress(&self) -> f64 {
        self.piece_manager.progress()
    }
}
