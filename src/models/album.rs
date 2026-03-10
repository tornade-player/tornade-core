//! Album domain model.

use crate::models::track::Rating;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A music album as stored in the library database.
///
/// Albums are created automatically during library scanning based on the
/// `ALBUM` and `ALBUMARTIST` tags of the audio files in a directory.
/// MusicBrainz fields are populated later by [`crate::services::ArtworkService`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    /// SQLite primary key (`albums.id`).
    pub id: i64,
    /// Album title from the `ALBUM` tag.
    pub title: String,
    /// Foreign key to the primary artist (`artists.id`).
    pub artist_id: i64,
    /// Denormalised artist name (avoids a join in listing queries).
    pub artist_name: String,
    /// Release year from the `DATE` or `YEAR` tag, if present.
    pub year: Option<u16>,
    /// User-assigned star rating (0–5). Defaults to 0.
    pub rating: Rating,
    /// Path to the local artwork file extracted from the audio tags, if any.
    pub artwork_path: Option<PathBuf>,
    /// Path to artwork downloaded from MusicBrainz / Cover Art Archive, if any.
    pub online_artwork_path: Option<PathBuf>,
    /// Optional free-text description fetched from an external source.
    pub description: Option<String>,
    /// MusicBrainz Release ID (`mbid`), populated by the artwork scraper.
    pub musicbrainz_id: Option<String>,
    /// Record label name from MusicBrainz, if available.
    pub label: Option<String>,
    /// Release country code (ISO 3166-1 alpha-2) from MusicBrainz, if available.
    pub country: Option<String>,
    /// Barcode / UPC from MusicBrainz, if available.
    pub barcode: Option<String>,
    /// Release group type from MusicBrainz (e.g. `"Album"`, `"Single"`, `"EP"`).
    pub album_type: Option<String>,
    /// MusicBrainz release status (e.g. `"Official"`, `"Promotional"`).
    pub release_status: Option<String>,
}
