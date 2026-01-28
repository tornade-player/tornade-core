// Track model

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioFormat {
    Flac,
    Mp3,
    Aac,
    Alac,
}

impl AudioFormat {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "flac" => Some(AudioFormat::Flac),
            "mp3" => Some(AudioFormat::Mp3),
            "aac" => Some(AudioFormat::Aac),
            "alac" | "m4a" => Some(AudioFormat::Alac),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AudioFormat::Flac => "flac",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Aac => "aac",
            AudioFormat::Alac => "alac",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: i64,
    pub title: String,
    pub album_id: Option<i64>,
    pub artist_id: i64,
    pub source_id: i64,
    pub file_path: PathBuf,
    #[serde(with = "duration_serde")]
    pub duration: Duration,
    pub track_number: Option<u32>,
    pub disc_number: u32,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub file_type: AudioFormat,
    pub file_size: u64,
    pub rating: u8,  // 0-5
    pub fingerprint: Option<String>,
    pub is_duplicate: bool,
    pub duplicate_of: Option<i64>,
    pub last_played_at: Option<String>,  // ISO 8601 datetime
    pub play_count: u32,
}

// Custom serde for Duration (stored as milliseconds in DB)
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}
