// Track model

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::PathBuf;
use std::time::Duration;

/// A validated star rating in the range 0–5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rating(pub u8);

/// Error returned when a u8 value is outside the valid rating range (0–5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRating(pub u8);

impl std::fmt::Display for InvalidRating {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid rating: {} (must be 0–5)", self.0)
    }
}

impl std::error::Error for InvalidRating {}

impl rusqlite::types::FromSql for Rating {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        let v = u8::column_result(value)?;
        Rating::try_from(v).map_err(|e| rusqlite::types::FromSqlError::Other(Box::new(e)))
    }
}

impl rusqlite::types::ToSql for Rating {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl TryFrom<u8> for Rating {
    type Error = InvalidRating;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value <= 5 {
            Ok(Rating(value))
        } else {
            Err(InvalidRating(value))
        }
    }
}

impl From<Rating> for u8 {
    fn from(r: Rating) -> u8 {
        r.0
    }
}

impl Serialize for Rating {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Rating {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = u8::deserialize(deserializer)?;
        Rating::try_from(value).map_err(|e| serde::de::Error::custom(e.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
#[serde(rename_all = "snake_case")]
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
    pub rating: Rating,
    pub fingerprint: Option<String>,
    pub is_duplicate: bool,
    pub duplicate_of: Option<i64>,
    pub last_played_at: Option<String>, // ISO 8601 datetime
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

/// Builder for [`Track`], making it ergonomic to construct tracks with many optional fields.
///
/// Required fields are provided via [`TrackBuilder::new`]; optional fields are set via
/// setter methods that return `&mut Self` for chaining. Call [`TrackBuilder::build`] to
/// get the finished [`Track`].
pub struct TrackBuilder {
    id: i64,
    title: String,
    artist_id: i64,
    source_id: i64,
    file_path: PathBuf,
    file_type: AudioFormat,
    file_size: u64,
    duration: Duration,
    album_id: Option<i64>,
    track_number: Option<u32>,
    disc_number: u32,
    sample_rate: Option<u32>,
    bit_depth: Option<u8>,
    rating: Rating,
    fingerprint: Option<String>,
    is_duplicate: bool,
    duplicate_of: Option<i64>,
    last_played_at: Option<String>,
    play_count: u32,
}

impl TrackBuilder {
    pub fn new(
        title: String,
        artist_id: i64,
        source_id: i64,
        file_path: PathBuf,
        file_type: AudioFormat,
        file_size: u64,
        duration: Duration,
    ) -> Self {
        Self {
            id: 0,
            title,
            artist_id,
            source_id,
            file_path,
            file_type,
            file_size,
            duration,
            album_id: None,
            track_number: None,
            disc_number: 0,
            sample_rate: None,
            bit_depth: None,
            rating: Rating(0),
            fingerprint: None,
            is_duplicate: false,
            duplicate_of: None,
            last_played_at: None,
            play_count: 0,
        }
    }

    pub fn id(mut self, id: i64) -> Self {
        self.id = id;
        self
    }
    pub fn album_id(mut self, album_id: Option<i64>) -> Self {
        self.album_id = album_id;
        self
    }
    pub fn track_number(mut self, track_number: Option<u32>) -> Self {
        self.track_number = track_number;
        self
    }
    pub fn disc_number(mut self, disc_number: u32) -> Self {
        self.disc_number = disc_number;
        self
    }
    pub fn sample_rate(mut self, sample_rate: Option<u32>) -> Self {
        self.sample_rate = sample_rate;
        self
    }
    pub fn bit_depth(mut self, bit_depth: Option<u8>) -> Self {
        self.bit_depth = bit_depth;
        self
    }
    pub fn rating(mut self, rating: Rating) -> Self {
        self.rating = rating;
        self
    }
    pub fn fingerprint(mut self, fingerprint: Option<String>) -> Self {
        self.fingerprint = fingerprint;
        self
    }
    pub fn is_duplicate(mut self, is_duplicate: bool) -> Self {
        self.is_duplicate = is_duplicate;
        self
    }
    pub fn duplicate_of(mut self, duplicate_of: Option<i64>) -> Self {
        self.duplicate_of = duplicate_of;
        self
    }
    pub fn last_played_at(mut self, last_played_at: Option<String>) -> Self {
        self.last_played_at = last_played_at;
        self
    }
    pub fn play_count(mut self, play_count: u32) -> Self {
        self.play_count = play_count;
        self
    }

    pub fn build(self) -> Track {
        Track {
            id: self.id,
            title: self.title,
            album_id: self.album_id,
            artist_id: self.artist_id,
            source_id: self.source_id,
            file_path: self.file_path,
            duration: self.duration,
            track_number: self.track_number,
            disc_number: self.disc_number,
            sample_rate: self.sample_rate,
            bit_depth: self.bit_depth,
            file_type: self.file_type,
            file_size: self.file_size,
            rating: self.rating,
            fingerprint: self.fingerprint,
            is_duplicate: self.is_duplicate,
            duplicate_of: self.duplicate_of,
            last_played_at: self.last_played_at,
            play_count: self.play_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_format_from_str() {
        assert_eq!(AudioFormat::from_str("flac"), Some(AudioFormat::Flac));
        assert_eq!(AudioFormat::from_str("FLAC"), Some(AudioFormat::Flac));
        assert_eq!(AudioFormat::from_str("mp3"), Some(AudioFormat::Mp3));
        assert_eq!(AudioFormat::from_str("aac"), Some(AudioFormat::Aac));
        assert_eq!(AudioFormat::from_str("alac"), Some(AudioFormat::Alac));
        assert_eq!(AudioFormat::from_str("m4a"), Some(AudioFormat::Alac));
    }

    #[test]
    fn test_audio_format_as_str_roundtrip() {
        for format in [
            AudioFormat::Flac,
            AudioFormat::Mp3,
            AudioFormat::Aac,
            AudioFormat::Alac,
        ] {
            let s = format.as_str();
            let back = AudioFormat::from_str(s).unwrap();
            assert_eq!(back, format);
        }
    }

    #[test]
    fn test_audio_format_unknown_extension() {
        assert_eq!(AudioFormat::from_str("wav"), None);
        assert_eq!(AudioFormat::from_str("ogg"), None);
        assert_eq!(AudioFormat::from_str(""), None);
    }

    // =========================================================================
    // Rating::try_from — T007
    // =========================================================================

    #[test]
    fn test_rating_try_from_zero_is_ok() {
        assert_eq!(Rating::try_from(0), Ok(Rating(0)));
    }

    #[test]
    fn test_rating_try_from_five_is_ok() {
        assert_eq!(Rating::try_from(5), Ok(Rating(5)));
    }

    #[test]
    fn test_rating_try_from_six_is_err() {
        assert_eq!(Rating::try_from(6), Err(InvalidRating(6)));
    }

    #[test]
    fn test_rating_try_from_255_is_err() {
        assert_eq!(Rating::try_from(255), Err(InvalidRating(255)));
    }

    #[test]
    fn test_rating_serde_round_trip_valid() {
        let r = Rating::try_from(3).unwrap();
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, "3");
        let back: Rating = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn test_rating_serde_round_trip_zero() {
        let r = Rating::try_from(0).unwrap();
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, "0");
        let back: Rating = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn test_rating_serde_deserialize_invalid_rejects() {
        let result = serde_json::from_str::<Rating>("6");
        assert!(result.is_err(), "deserializing 6 as Rating must fail");
    }

    #[test]
    fn test_rating_serde_deserialize_255_rejects() {
        let result = serde_json::from_str::<Rating>("255");
        assert!(result.is_err(), "deserializing 255 as Rating must fail");
    }
}
