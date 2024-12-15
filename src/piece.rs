use std::collections::{HashMap, HashSet};
use std::cmp::Ordering;
use sha1::{Digest, Sha1};

#[derive(Debug, Clone)]
pub struct PieceInfo {
    index: usize,
    hash: [u8; 20],
    length: usize,
    priority: i32,
}

impl PieceInfo {
    pub fn new(index: usize, hash: [u8; 20], length: usize) -> Self {
        Self {
            index,
            hash,
            length,
            priority: 0,
        }
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn length(&self) -> usize {
        self.length
    }

    pub fn priority(&self) -> i32 {
        self.priority
    }

    pub fn set_priority(&mut self, priority: i32) {
        self.priority = priority;
    }
}

// Keep the existing Ord implementation style for PieceInfo
impl Ord for PieceInfo {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
            .then_with(|| self.index.cmp(&other.index))
    }
}

impl PartialOrd for PieceInfo {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for PieceInfo {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl Eq for PieceInfo {}

#[derive(Debug, Clone, Default)]
pub struct PieceManager {
    piece_length: usize,
    num_pieces: usize,
    pieces: Vec<PieceInfo>,
    // Track which peers have which pieces
    peer_pieces: HashMap<usize, HashSet<usize>>,
    // Track downloaded pieces
    completed_pieces: HashSet<usize>,
}

impl PieceManager {
    pub fn new(piece_length: usize, hashes: Vec<[u8; 20]>, total_length: usize) -> Self {
        let num_pieces = hashes.len();
        let pieces = hashes
            .into_iter()
            .enumerate()
            .map(|(i, hash)| {
                let length = if i == num_pieces - 1 {
                    let remaining = total_length % piece_length;
                    if remaining == 0 {
                        piece_length
                    } else {
                        remaining
                    }
                } else {
                    piece_length
                };

                PieceInfo::new(i, hash, length)
            })
            .collect();

        Self {
            piece_length,
            num_pieces,
            pieces,
            peer_pieces: HashMap::new(),
            completed_pieces: HashSet::new(),
        }
    }

    pub fn register_peer(&mut self, peer_id: usize) {
        self.peer_pieces.insert(peer_id, HashSet::new());
    }

    pub fn add_peer_piece(&mut self, peer_id: usize, piece_index: usize) {
        if let Some(pieces) = self.peer_pieces.get_mut(&peer_id) {
            pieces.insert(piece_index);
        }
    }

    pub fn remove_peer(&mut self, peer_id: usize) {
        self.peer_pieces.remove(&peer_id);
    }

    pub fn next_piece(&self, peer_id: usize) -> Option<PieceInfo> {
        let peer_pieces = self.peer_pieces.get(&peer_id)?;
        
        // Find rarest piece this peer has that we haven't downloaded
        let mut piece_counts = HashMap::new();
        for pieces in self.peer_pieces.values() {
            for &piece_index in pieces {
                if !self.completed_pieces.contains(&piece_index) {
                    *piece_counts.entry(piece_index).or_insert(0) += 1;
                }
            }
        }

        peer_pieces
            .iter()
            .filter(|&&index| !self.completed_pieces.contains(&index))
            .min_by_key(|&&index| piece_counts.get(&index).unwrap_or(&0))
            .map(|&index| self.pieces[index].clone())
    }

    pub fn verify_piece(&self, index: usize, data: &[u8]) -> bool {
        let piece = &self.pieces[index];
        let mut hasher = Sha1::new();
        hasher.update(data);
        let hash: [u8; 20] = hasher
            .finalize()
            .try_into()
            .expect("SHA1 output is always 20 bytes");
        hash == piece.hash
    }

    pub fn mark_completed(&mut self, index: usize) {
        self.completed_pieces.insert(index);
    }

    pub fn is_complete(&self) -> bool {
        self.completed_pieces.len() == self.num_pieces
    }

    pub fn progress(&self) -> f64 {
        self.completed_pieces.len() as f64 / self.num_pieces as f64
    }

    pub fn get_piece(&self, index: usize) -> Option<&PieceInfo> {
        self.pieces.get(index)
    }

    pub fn has_piece(&self, peer_id: usize, piece_index: usize) -> bool {
        self.peer_pieces
            .get(&peer_id)
            .map(|pieces| pieces.contains(&piece_index))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_piece_manager() {
        let hashes = vec![[0; 20], [1; 20], [2; 20]];
        let mut manager = PieceManager::new(1024, hashes, 2500);
        
        manager.register_peer(1);
        manager.add_peer_piece(1, 0);
        manager.add_peer_piece(1, 1);

        manager.register_peer(2);
        manager.add_peer_piece(2, 1);
        manager.add_peer_piece(2, 2);

        // Piece 1 should be most common (both peers have it)
        let next = manager.next_piece(1).unwrap();
        assert_eq!(next.index(), 0);

        manager.mark_completed(0);
        
        // Now should select piece 1 since 0 is completed
        let next = manager.next_piece(1).unwrap();
        assert_eq!(next.index(), 1);

        assert!(!manager.is_complete());
        manager.mark_completed(1);
        manager.mark_completed(2);
        assert!(manager.is_complete());
    }

    #[test]
    fn test_piece_verification() {
        let mut hasher = Sha1::new();
        hasher.update(b"test data");
        let hash = hasher.finalize().try_into().unwrap();

        let hashes = vec![hash];
        let manager = PieceManager::new(1024, hashes, 1024);

        assert!(manager.verify_piece(0, b"test data"));
        assert!(!manager.verify_piece(0, b"wrong data"));
    }
}
