//! Token format `agt_` + 32 bytes base64url and SHA-256 in hex.

use sha2::{Digest, Sha256};

const PREFIX: &str = "agt_";
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
/// 32 bytes in base64url without padding.
const BODY_LEN: usize = 43;

fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// New token from a cryptographically strong source (`rand`, ChaCha seeded by the OS).
pub fn generate() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    format!("{PREFIX}{}", encode(&bytes))
}

/// Exactly the generator's format: `agt_` and 43 base64url characters without padding. The last character carries 4 significant bits
/// and two zero bits, so its index is a multiple of 4: the generator cannot produce other values.
pub fn is_valid_format(token: &str) -> bool {
    let Some(body) = token.strip_prefix(PREFIX) else {
        return false;
    };
    if body.len() != BODY_LEN || !body.bytes().all(|b| ALPHABET.contains(&b)) {
        return false;
    }
    body.bytes()
        .last()
        .and_then(|b| ALPHABET.iter().position(|a| *a == b))
        .is_some_and(|i| i % 4 == 0)
}

/// SHA-256 in hex, 64 lowercase characters.
pub fn sha256_hex(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
