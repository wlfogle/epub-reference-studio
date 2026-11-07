pub mod align;
pub mod book;
pub mod ollama;
pub mod online;
pub mod sigil;
pub mod workspace;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub title: String,
    pub authors: Vec<String>,
    pub language: String,
    pub identifiers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub kind: String,
    pub location: String,
    pub context: String,
    pub explanation: String,
    #[serde(default)]
    pub start: Option<usize>,
    #[serde(default)]
    pub end: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edit {
    pub id: String,
    pub start: usize,
    pub end: usize,
    pub before: String,
    pub after: String,
    pub context: String,
    pub location: String,
    pub kind: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Comparison {
    pub edits: Vec<Edit>,
    pub anchor_count: usize,
    pub anchored_fraction: f64,
    pub skipped_regions: usize,
    pub warning: String,
}
