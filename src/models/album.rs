// Album model

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    pub id: i64,
    pub title: String,
    pub artist_id: i64,
    pub artist_name: String,
    pub year: Option<u16>,
    pub rating: u8,  // 0-5
    pub artwork_path: Option<PathBuf>,
    pub online_artwork_path: Option<PathBuf>,
    pub description: Option<String>,
    // MusicBrainz metadata (populated by artwork scraper)
    pub musicbrainz_id: Option<String>,
    pub label: Option<String>,
    pub country: Option<String>,
    pub barcode: Option<String>,
    pub album_type: Option<String>,
    pub release_status: Option<String>,
}
