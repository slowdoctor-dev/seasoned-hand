//! Canonical hashing helper.
//!
//! Collapses the identical `format!("{:x}", Sha256::digest(..))` copies
//! that lived in `deliverable` (content hashing) and `org::invitation`
//! (login-token hashing) into one definition, so the hash encoding is
//! audited and changed in a single place.

use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
