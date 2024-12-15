// tests/torrent_test.rs
use bittorrent;
mod common;

#[tokio::test]
async fn test_parse_single_file_torrent() {
    common::create_test_torrent();
    
    let torrent = Torrent::from_file(common::test_file("debian.torrent"))
        .await
        .expect("Failed to parse torrent file");
    
    assert_eq!(torrent.announce, "http://bttracker.debian.org:6969/announce");
    assert_eq!(torrent.info.name, "debian-12.1.0-amd64-netinst.iso");
    assert_eq!(torrent.info.piece_length, 262144); // 256 KB
    
    // Check total length
    match torrent.info.kind {
        FileKind::Single { length, .. } => {
            assert_eq!(length, 673_185_792); // ~642 MB
        }
        _ => panic!("Expected single file torrent"),
    }
    
    // Verify the first piece hash
    assert_eq!(
        hex::encode(&torrent.info.pieces.0[0]),
        "f2d173b6b9be8455f15864fd94da0bbdec917835" // example hash
    );
    
    // Check piece count matches file size
    let expected_pieces = (673_185_792 + 262144 - 1) / 262144;
    assert_eq!(torrent.info.pieces.0.len(), expected_pieces);
}

#[test]
fn test_info_hash_calculation() {
    let torrent_bytes = include_bytes!("data/debian.torrent");
    let torrent = Torrent::from_bytes(torrent_bytes)
        .expect("Failed to parse torrent from bytes");
    
    let info_hash = torrent.info_hash();
    assert_eq!(
        hex::encode(info_hash),
        "6b92e37e877d36c36e0ba62f44a688cea6987181" // example hash
    );
}

#[test]
fn test_piece_length_calculation() {
    let torrent_bytes = include_bytes!("data/debian.torrent");
    let torrent = Torrent::from_bytes(torrent_bytes)
        .expect("Failed to parse torrent from bytes");
    
    // Test normal piece length
    assert_eq!(torrent.piece_length(0), 262144);
    
    // Test last piece length
    let last_piece = torrent.piece_count() - 1;
    let expected_last_length = 673_185_792 % 262144;
    assert_eq!(torrent.piece_length(last_piece), expected_last_length);
}
