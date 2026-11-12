//! Minimal shared utilities retained for the India reference modules.

/// Lowercase hexadecimal SHA-256 used by the source archive and snapshots.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    hex_lower(&digest)
}

/// Encode arbitrary bytes as lower-case, zero-padded hexadecimal.
#[must_use]
pub fn hex_lower(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_lower_is_zero_padded_and_lowercase() {
        assert_eq!(hex_lower(&[0, 10, 255]), "000aff");
    }

    #[test]
    fn sha256_hex_matches_published_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
