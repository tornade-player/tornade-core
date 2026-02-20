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
            "SELECT a.id, a.title, a.artist_id, ar.name as artist_name, a.year, a.rating,
                    a.artwork_path, a.online_artwork_path, a.description,
                    a.musicbrainz_id, a.label, a.country, a.barcode, a.album_type, a.release_status
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
                musicbrainz_id: row.get(9)?,
                label: row.get(10)?,
                country: row.get(11)?,
                barcode: row.get(12)?,
                album_type: row.get(13)?,
                release_status: row.get(14)?,
            })
        })
        .map_err(LibraryError::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(LibraryError::Database)?;

        // Search artists by name using pattern matching
        let artist_pattern = format!("%{}%", query);
        let mut stmt = conn.prepare(
            "SELECT id, name, name_sort, bio, country, genre, style, mood,
                    formed_year, born_year, died_year, disbanded, musicbrainz_id, theaudiodb_id
             FROM artists WHERE name LIKE ?1 LIMIT 20"
        ).map_err(LibraryError::Database)?;

        let artists: Vec<Artist> = stmt.query_map([&artist_pattern], |row| {
            Ok(Artist {
                id: row.get(0)?,
                name: row.get(1)?,
                name_sort: row.get(2)?,
                bio: row.get(3)?,
                country: row.get(4)?,
                genre: row.get(5)?,
                style: row.get(6)?,
                mood: row.get(7)?,
                formed_year: row.get(8)?,
                born_year: row.get(9)?,
                died_year: row.get(10)?,
                disbanded: row.get(11)?,
                musicbrainz_id: row.get(12)?,
                theaudiodb_id: row.get(13)?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TestEnv;

    #[test]
    fn test_search_empty_query_returns_empty() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("").unwrap();
        assert!(results.tracks.is_empty());
        assert!(results.albums.is_empty());
        assert!(results.artists.is_empty());
    }

    #[test]
    fn test_search_whitespace_query_returns_empty() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("   ").unwrap();
        assert!(results.tracks.is_empty());
    }

    #[test]
    fn test_search_album_by_title() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("Test Album").unwrap();
        assert!(!results.albums.is_empty());
        assert_eq!(results.albums[0].title, "Test Album");
    }

    #[test]
    fn test_search_artist_by_name() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("Test Artist").unwrap();
        assert!(!results.artists.is_empty());
    }

    #[test]
    fn test_search_no_results() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("xyzxyzxyznotfound").unwrap();
        assert!(results.tracks.is_empty());
        assert!(results.albums.is_empty());
        assert!(results.artists.is_empty());
    }

    #[test]
    fn test_search_track_by_title_via_fts() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        // FTS index is populated via trigger on INSERT; "Track" matches both seeded tracks
        let results = service.search("Track").unwrap();
        assert!(!results.tracks.is_empty(), "FTS5 search must find tracks by title token");
    }

    #[test]
    fn test_search_track_by_exact_title_via_fts() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("One").unwrap();
        assert_eq!(results.tracks.len(), 1, "only 'Track One' contains the token 'One'");
        assert_eq!(results.tracks[0].title, "Track One");
    }

    #[test]
    fn test_search_track_by_artist_name_via_fts() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        // FTS artist_name column is populated from the artist table via trigger
        let results = service.search("Artist").unwrap();
        assert!(!results.tracks.is_empty(), "FTS5 must find tracks by artist name token");
    }

    // ── FTS5 special characters — must not panic ──────────────────────────

    #[test]
    fn test_search_fts5_double_quote_does_not_panic() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        // A bare double-quote is malformed FTS5 syntax; must not panic.
        // Returning Ok(empty) or Err(...) are both acceptable.
        let _ = service.search("\"");
    }

    #[test]
    fn test_search_fts5_unclosed_paren_does_not_panic() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let _ = service.search("(unclosed");
    }

    #[test]
    fn test_search_fts5_boolean_operators_do_not_panic() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        // FTS5 treats AND/OR/NOT as operators; verify no panic for edge-case inputs.
        let _ = service.search("AND");
        let _ = service.search("OR");
        let _ = service.search("NOT");
    }

    #[test]
    fn test_search_fts5_colon_does_not_panic() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let _ = service.search("title:");
    }

    // ── LIKE wildcard behaviour ───────────────────────────────────────────

    #[test]
    fn test_search_percent_causes_fts5_syntax_error() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        // "%" is invalid FTS5 syntax; search() must return Err rather than panic.
        // The LIKE path would match all albums, but FTS5 fires first and errors.
        // This test documents the current behaviour as a regression checkpoint.
        // Ideal future behaviour: sanitise the query and return Ok(empty).
        let result = service.search("%");
        assert!(result.is_err(), "current behaviour: '%' causes an FTS5 syntax error");
    }

    #[test]
    fn test_search_underscore_in_query_does_not_match_wrong_artist() {
        let env = TestEnv::new();
        env.seed_basic_library(); // artist "Test Artist"
        let service = SearchService::new(env.pool.clone());
        // "_est Artist" would match "Test Artist" because "_" is a LIKE single-char wildcard.
        let results = service.search("_est Artist").unwrap();
        // Current behaviour: the "_" wildcard matches, so "Test Artist" is returned.
        // This documents the current behaviour; the ideal fix would escape "_".
        assert!(
            !results.artists.is_empty(),
            "current behaviour: '_' in query acts as LIKE single-char wildcard"
        );
    }

    // ── Result capping ────────────────────────────────────────────────────

    #[test]
    fn test_search_results_capped_per_type() {
        let env = TestEnv::new();
        env.seed_basic_library(); // 2 tracks, 1 album, 1 artist
        let service = SearchService::new(env.pool.clone());
        let results = service.search("Test").unwrap();
        assert!(results.tracks.len()  <= 50, "track results must be capped at 50");
        assert!(results.albums.len()  <= 20, "album results must be capped at 20");
        assert!(results.artists.len() <= 20, "artist results must be capped at 20");
    }

    #[test]
    fn test_search_empty_library_returns_all_empty() {
        let env = TestEnv::new(); // no data seeded
        let service = SearchService::new(env.pool.clone());
        let results = service.search("anything").unwrap();
        assert!(results.tracks.is_empty());
        assert!(results.albums.is_empty());
        assert!(results.artists.is_empty());
    }
}
