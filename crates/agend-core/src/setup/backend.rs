//! Identity of an imported, not-yet-admitted native backend executable.
//! A claimed version never substitutes for a successful native canary.

use alloc::string::String;
use serde::{Deserialize, Serialize};
pub mod canary;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedBackend {
    pub format: u32,
    pub backend: String,
    pub version: String,
    pub sha256: String,
    pub bytes: u64,
}

pub fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 80
        && version
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_+".contains(&b))
}
