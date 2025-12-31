use std::path::Path;

use std::fs::File;
use std::io::BufReader;

pub fn compute_file_hash(path: &Path) -> anyhow::Result<blake3::Hash> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&mut reader)?;
    let hash = hasher.finalize();
    Ok(hash)
}
