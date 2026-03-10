//! Genre domain model.

use serde::{Deserialize, Serialize};

/// A music genre tag.
///
/// Genres are extracted from the `GENRE` tag of audio files during scanning and
/// stored in a normalised `genres` table. A track can belong to multiple genres
/// via the `track_genres` join table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Genre {
    /// SQLite primary key (`genres.id`).
    pub id: i64,
    /// Genre name as it appears in the audio tags (e.g. `"Jazz"`, `"Hip-Hop"`).
    pub name: String,
}
