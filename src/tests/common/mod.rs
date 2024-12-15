// tests/common/mod.rs
use std::path::PathBuf;
use std::fs;

pub fn test_file(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("data");
    path.push(name);
    path
}

pub fn create_test_torrent() {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("data");
    fs::create_dir_all(&path).unwrap();
    
    path.push("debian.torrent");
    fs::write(path, include_bytes!("../data/Knoppix.torrent")).unwrap();
}
