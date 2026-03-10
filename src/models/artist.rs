//! Artist domain model.

use serde::{Deserialize, Serialize};

/// A recording artist or band as stored in the library database.
///
/// Artists are created from the `ARTIST` / `ALBUMARTIST` tags during scanning.
/// Biographical fields (`bio`, `country`, `formed_year`, …) are populated by
/// [`crate::services::ArtworkService`] via TheAudioDB and MusicBrainz lookups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    /// SQLite primary key (`artists.id`).
    pub id: i64,
    /// Artist name as it appears in the audio tags (case-preserved).
    pub name: String,
    /// Sortable variant of the name (e.g. `"Beatles, The"`), used for alphabetic ordering.
    pub name_sort: Option<String>,
    /// Long-form biography fetched from an external source.
    pub bio: Option<String>,
    /// Country of origin (free-text, as returned by TheAudioDB).
    pub country: Option<String>,
    /// Primary genre label from TheAudioDB.
    pub genre: Option<String>,
    /// Musical style descriptor from TheAudioDB.
    pub style: Option<String>,
    /// Mood descriptor from TheAudioDB.
    pub mood: Option<String>,
    /// Year the band/group was formed. `None` for solo artists or if unknown.
    pub formed_year: Option<i64>,
    /// Birth year, for solo artists.
    pub born_year: Option<i64>,
    /// Death year, for deceased solo artists.
    pub died_year: Option<i64>,
    /// Non-`None` when a band has officially disbanded; value is a free-text date or year.
    pub disbanded: Option<String>,
    /// MusicBrainz Artist ID (`mbid`).
    pub musicbrainz_id: Option<String>,
    /// TheAudioDB numeric artist ID (stored as a string to match the API response).
    pub theaudiodb_id: Option<String>,
}
