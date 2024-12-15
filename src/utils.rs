// src/utils.rs
use rand::{thread_rng, Rng};
use sha1::{Digest, Sha1};

pub fn generate_peer_id() -> [u8; 20] {
    let mut rng = thread_rng();
    let mut id = [0u8; 20];
    id[0..8].copy_from_slice(crate::PEER_ID_PREFIX);
    rng.fill(&mut id[8..]);
    id
}

pub fn calculate_piece_hash(data: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hasher
        .finalize()
        .try_into()
        .expect("SHA1 output is always 20 bytes")
}

pub fn bit_set(bitfield: &[u8], index: usize) -> bool {
    let byte_index = index / 8;
    let bit_offset = 7 - (index % 8);
    byte_index < bitfield.len() && (bitfield[byte_index] & (1 << bit_offset)) != 0
}

pub fn set_bit(bitfield: &mut [u8], index: usize) {
    let byte_index = index / 8;
    let bit_offset = 7 - (index % 8);
    if byte_index < bitfield.len() {
        bitfield[byte_index] |= 1 << bit_offset;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitfield_operations() {
        let mut bitfield = vec![0u8; 2];
        assert!(!bit_set(&bitfield, 0));

        set_bit(&mut bitfield, 0);
        assert!(bit_set(&bitfield, 0));
        assert!(!bit_set(&bitfield, 1));

        set_bit(&mut bitfield, 15);
        assert!(bit_set(&bitfield, 15));
    }
}
