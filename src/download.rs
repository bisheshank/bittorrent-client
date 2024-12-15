use crate::{
    error::{BitTorrentError, Result},
    peer::Peer,
    piece::PieceManager,
    torrent::Torrent,
    tracker::Tracker,
    utils::{bit_set, generate_peer_id},
    MAX_PEERS,
};
use futures_util::{stream::FuturesUnordered, StreamExt};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::{mpsc, Mutex};
use tokio::time::{timeout, Duration};

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

impl Download {
    pub async fn new(torrent: Torrent) -> Result<Self> {
        println!("Initializing download for: {}", torrent.info.name);
        let tracker = Tracker::new(&torrent)?;
        println!("Connecting to tracker at: {}", torrent.announce);
        let peer_list = tracker.get_peers().await?;
        println!("Found {} potential peers", peer_list.len());

        let info_hash = torrent.info_hash();
        let mut peers = Vec::new();
        let mut failed_attempts = 0;
        const MAX_FAILED_ATTEMPTS: usize = 200;
        const CONCURRENT_CONNECTS: usize = 50;
        const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

        // Create a pool of connection futures
        let mut connection_pool = FuturesUnordered::new();

        // Initialize with first batch of connection attempts
        for addr in peer_list.iter().take(CONCURRENT_CONNECTS) {
            connection_pool.push(timeout(
                CONNECT_TIMEOUT,
                Peer::connect(*addr, info_hash, generate_peer_id()),
            ));
        }

        let mut peer_index = CONCURRENT_CONNECTS;

        while let Some(result) = connection_pool.next().await {
            match result {
                Ok(Ok(peer)) => {
                    println!("Successfully connected to peer: {}", peer.addr());
                    peers.push(peer);

                    if peers.len() >= MAX_PEERS {
                        break;
                    }
                }
                _ => {
                    failed_attempts += 1;
                    if failed_attempts >= MAX_FAILED_ATTEMPTS {
                        println!("Reached maximum failed connection attempts");
                        break;
                    }
                }
            }

            // Add new connection attempt if we have more peers to try
            if peer_index < peer_list.len() {
                connection_pool.push(timeout(
                    CONNECT_TIMEOUT,
                    Peer::connect(peer_list[peer_index], info_hash, generate_peer_id()),
                ));
                peer_index += 1;
            }

            // Break if we have no more pending connections and enough peers
            if connection_pool.is_empty() || peers.len() >= MAX_PEERS {
                break;
            }
        }

        if peers.is_empty() {
            return Err(BitTorrentError::Peer(
                "Failed to connect to any peers".into(),
            ));
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
        let piece_manager_clone = Arc::clone(&piece_manager);

        for (peer_id, mut peer) in peers.into_iter().enumerate() {
            let tx = tx.clone();
            let piece_manager = Arc::clone(&piece_manager);

            tokio::spawn(async move {
                let mut consecutive_failures = 0;
                const MAX_CONSECUTIVE_FAILURES: usize = 3;
                let mut failed_pieces = HashSet::new();

                while consecutive_failures < MAX_CONSECUTIVE_FAILURES {
                    // Get next piece under lock
                    let piece = {
                        let mut pm = piece_manager.lock().await;
                        // Don't request pieces that have failed for this peer
                        pm.next_piece_excluding(peer_id, &failed_pieces)
                    };

                    match piece {
                        Some(piece) => {
                            println!("Peer {} requesting piece {}", peer_id, piece.index());
                            match peer.request_piece(&piece).await {
                                Ok(data) => {
                                    let verified = {
                                        let pm = piece_manager.lock().await;
                                        pm.verify_piece(piece.index(), &data)
                                    };

                                    if verified {
                                        consecutive_failures = 0;
                                        let msg = TaskMessage::PieceCompleted {
                                            index: piece.index(),
                                            data,
                                        };
                                        if tx.send(msg).await.is_err() {
                                            break;
                                        }
                                    } else {
                                        consecutive_failures += 1;
                                        failed_pieces.insert(piece.index());
                                        let msg = TaskMessage::PieceFailed {
                                            index: piece.index(),
                                            error: BitTorrentError::Piece(
                                                "Verification failed".into(),
                                            ),
                                        };
                                        if tx.send(msg).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                Err(e) => {
                                    consecutive_failures += 1;
                                    failed_pieces.insert(piece.index());

                                    if e.is_connection_error() {
                                        let msg = TaskMessage::PeerDisconnected { peer_id };
                                        let _ = tx.send(msg).await;
                                        break;
                                    } else {
                                        println!(
                                            "Peer {} failed to download piece {}: {:?}",
                                            peer_id,
                                            piece.index(),
                                            e
                                        );
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
                            // No more pieces available for this peer
                            break;
                        }
                    }
                }

                // Clean up peer when done
                let mut pm = piece_manager.lock().await;
                pm.remove_peer(peer_id);
                println!("Peer {} task completed", peer_id);
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
        )
        .await;

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

            println!(
                "Entering endgame mode for {} remaining pieces",
                failed_pieces.len()
            );
            self.handle_endgame(&failed_pieces).await?;
        }

        let is_complete = failed_pieces.is_empty() && completed_pieces == total_pieces;

        if is_complete {
            Ok(self.downloaded.clone())
        } else {
            Err(BitTorrentError::Download(
                "Failed to download all pieces".into(),
            ))
        }
    }

    async fn handle_endgame(&mut self, failed_pieces: &[usize]) -> Result<()> {
        for &piece_index in failed_pieces {
            let piece_info = self
                .piece_manager
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
                            println!(
                                "Endgame: Failed to download piece {} from peer: {:?}",
                                piece_index, e
                            );
                            continue;
                        }
                    }
                }
            }

            if !success {
                return Err(BitTorrentError::Piece(format!(
                    "Failed to download piece {} in endgame mode",
                    piece_index
                )));
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
