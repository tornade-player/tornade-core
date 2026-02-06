// Full-text search service using FTS5

use crate::db::DbPool;
use crate::db::queries;
use crate::models::{Track, Album, Artist};
use crate::services::error::LibraryError;
use std::path::PathBuf;

pub struct SearchService {
    pool: DbPool,
}

pub struct SearchResults {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
}

impl SearchService {
    pub fn new(pool: DbPool) -> Self {
        SearchService { pool }
    }

    /// Search across tracks, albums, and artists using FTS5
    pub fn search(&self, query: &str) -> Result<SearchResults, LibraryError> {
        if query.trim().is_empty() {
            return Ok(SearchResults {
                tracks: Vec::new(),
                albums: Vec::new(),
                artists: Vec::new(),
            });
        }

        let conn = self.pool.get()
            .map_err(|e| LibraryError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?;

        // Search tracks using FTS5
        let mut stmt = conn.prepare(
            "SELECT t.id FROM tracks t
             JOIN tracks_fts fts ON fts.rowid = t.id
             WHERE tracks_fts MATCH ?1
             ORDER BY rank
             LIMIT 50"
        ).map_err(LibraryError::Database)?;

        let track_ids: Vec<i64> = stmt.query_map([query], |row| row.get(0))
            .map_err(LibraryError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(LibraryError::Database)?;

        // Get full track details using existing query function
        let mut tracks = Vec::new();
        for track_id in track_ids {
            if let Ok(Some(track)) = queries::get_track(&conn, track_id) {
                tracks.push(track);
            }
        }

        // Search albums by title using pattern matching
        let album_pattern = format!("%{}%", query);
        let mut stmt = conn.prepare(
            "SELECT a.id, a.title, a.artist_id, ar.name as artist_name, a.year, a.rating, a.artwork_path, a.online_artwork_path, a.description
             FROM albums a
             JOIN artists ar ON ar.id = a.artist_id
             WHERE a.title LIKE ?1 LIMIT 20"
        ).map_err(LibraryError::Database)?;

        let albums: Vec<Album> = stmt.query_map([&album_pattern], |row| {
            Ok(Album {
                id: row.get(0)?,
                title: row.get(1)?,
                artist_id: row.get(2)?,
                artist_name: row.get(3)?,
                year: row.get(4)?,
                rating: row.get(5)?,
                artwork_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
                online_artwork_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
                description: row.get(8)?,
            })
        })
        .map_err(LibraryError::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(LibraryError::Database)?;

        // Search artists by name using pattern matching
        let artist_pattern = format!("%{}%", query);
        let mut stmt = conn.prepare(
            "SELECT * FROM artists WHERE name LIKE ?1 LIMIT 20"
        ).map_err(LibraryError::Database)?;

        let artists: Vec<Artist> = stmt.query_map([&artist_pattern], |row| {
            Ok(Artist {
                id: row.get(0)?,
                name: row.get(1)?,
                bio: row.get(2)?,
                name_sort: row.get(3)?,
            })
        })
        .map_err(LibraryError::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(LibraryError::Database)?;

        Ok(SearchResults {
            tracks,
            albums,
            artists,
        })
    }
}
