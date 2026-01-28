// Album model

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    pub id: i64,
    pub title: String,
    pub artist_id: i64,
    pub year: Option<u16>,
    pub rating: u8,  // 0-5
    pub artwork_path: Option<PathBuf>,
    pub description: Option<String>,
}
