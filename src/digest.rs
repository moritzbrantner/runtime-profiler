use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

#[must_use]
pub fn sha256_bytes(bytes: &[u8]) -> String {
    lower_hex(&Sha256::digest(bytes))
}

/// Lowercase hex of a digest; sha2 0.11 output no longer implements `LowerHex`.
fn lower_hex(digest: &[u8]) -> String {
    use std::fmt::Write;
    digest
        .iter()
        .fold(String::with_capacity(digest.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let file = File::open(path)
        .with_context(|| format!("failed to open artifact for hashing: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];

    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("failed to hash artifact: {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let digest = hasher.finalize();
    Ok(lower_hex(&digest))
}
