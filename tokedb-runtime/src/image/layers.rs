use std::path::Path;

use crate::error::{Result, RuntimeError};
use crate::filesystem::sha256_file;

const DIGEST_ALGORITHM: &str = "sha256";

pub fn is_valid_digest(digest: &str) -> bool {
    match digest.split_once(':') {
        Some((algorithm, hex)) => {
            algorithm == DIGEST_ALGORITHM
                && hex.len() == 64
                && hex.chars().all(|c| c.is_ascii_hexdigit())
        }
        None => false,
    }
}

pub fn layer_hex(digest: &str) -> Result<&str> {
    match digest.split_once(':') {
        Some((algorithm, hex)) if algorithm == DIGEST_ALGORITHM && hex.len() == 64 => Ok(hex),
        _ => Err(RuntimeError::InvalidManifest {
            reason: format!("invalid digest `{digest}`"),
        }),
    }
}

pub fn sha256_digest(path: &Path) -> Result<String> {
    Ok(format!("{DIGEST_ALGORITHM}:{}", sha256_file(path)?))
}

pub fn digest_of_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{DIGEST_ALGORITHM}:{:x}", Sha256::digest(bytes))
}

pub fn verify_digest(path: &Path, expected: &str) -> Result<()> {
    let actual = sha256_digest(path)?;
    if actual != expected {
        return Err(RuntimeError::DigestMismatch {
            expected: expected.to_string(),
            actual,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC_HEX: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn is_valid_digest_accepts_sha256_hex() {
        let digest = format!("sha256:{ABC_HEX}");
        assert!(is_valid_digest(&digest));
        assert!(is_valid_digest(&format!("sha256:{}", "A".repeat(64))));
    }

    #[test]
    fn is_valid_digest_rejects_bad_input() {
        for bad in [
            "",
            "sha256:",
            "sha256:1234",
            "md5:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "sha256:xyz",
            "sha256:1234",
        ] {
            assert!(!is_valid_digest(bad), "{bad}");
        }
    }

    #[test]
    fn layer_hex_strips_prefix() {
        let digest = format!("sha256:{ABC_HEX}");
        assert_eq!(layer_hex(&digest).unwrap(), ABC_HEX);
        assert!(layer_hex("sha256:short").is_err());
    }

    #[test]
    fn sha256_digest_matches_known_value() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("data.bin");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(sha256_digest(&file).unwrap(), format!("sha256:{ABC_HEX}"));
    }

    #[test]
    fn digest_of_bytes_matches_known_value() {
        assert_eq!(digest_of_bytes(b"abc"), format!("sha256:{ABC_HEX}"));
    }

    #[test]
    fn verify_digest_passes_and_fails() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("data.bin");
        std::fs::write(&file, b"abc").unwrap();
        verify_digest(&file, &format!("sha256:{ABC_HEX}")).unwrap();
        let err = verify_digest(&file, &format!("sha256:{}", "0".repeat(64))).unwrap_err();
        assert!(matches!(err, RuntimeError::DigestMismatch { .. }));
    }
}
