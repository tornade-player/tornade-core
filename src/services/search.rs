//! Full-text search service combining FTS5, LIKE, and Levenshtein fuzzy matching.

use crate::db::DbPool;
use crate::db::queries;
use crate::models::{Album, Artist, Track};
use crate::services::error::LibraryError;
use std::collections::HashSet;
use std::path::PathBuf;

/// Performs full-text and fuzzy search across tracks, albums, and artists.
///
/// Three complementary strategies are combined and their results merged
/// (deduplication by entity ID):
///
/// 1. **FTS5 prefix** — fast, relevance-ordered, handles partial-word queries
///    (e.g. `"Coltr"` → `"Coltrane"`).
/// 2. **LIKE `%query%`** — substring fallback for entries missing from the FTS index.
/// 3. **Levenshtein** — typo tolerance for queries ≥ 4 characters
///    (e.g. `"trak"` → `"track"`).
pub struct SearchService {
    pool: DbPool,
}

/// Results returned by [`SearchService::search`], grouped by entity type.
pub struct SearchResults {
    /// Matching tracks, ordered by relevance then title.
    pub tracks: Vec<Track>,
    /// Matching albums, ordered by relevance then title.
    pub albums: Vec<Album>,
    /// Matching artists, ordered by relevance then name.
    pub artists: Vec<Artist>,
}

// ── Query sanitizers ──────────────────────────────────────────────────────────

/// Sanitize a raw query for FTS5 MATCH syntax.
/// Strips FTS5 operator characters, then appends `*` to each token for prefix
/// matching ("Coltr" → "Coltr*"). Returns empty string if nothing remains.
fn sanitize_fts_query(raw: &str) -> String {
    let cleaned: String =
        raw.chars()
            .map(|c| match c {
                '"' | '(' | ')' | ':' | '^' | '-' | '*' | '\\' | '+' | '~' | '%' | '/' | '?'
                | '.' => ' ',
                _ => c,
            })
            .collect();
    cleaned
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| format!("{t}*"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Sanitize a raw query for use as a LIKE literal (no ESCAPE clause needed).
/// Removes `%`, `_`, and `\` which are special in SQL LIKE, so the result can
/// safely be wrapped in `'%' || ?1 || '%'` without an ESCAPE clause.
fn sanitize_like(raw: &str) -> String {
    raw.chars()
        .filter(|&c| c != '%' && c != '_' && c != '\\')
        .collect()
}

// ── Fuzzy matching (Rust-side) ────────────────────────────────────────────────

/// Levenshtein edit distance, O(m) space.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut row: Vec<usize> = (0..=m).collect();
    for i in 1..=n {
        let mut prev = row[0];
        row[0] = i;
        for j in 1..=m {
            let curr = row[j];
            row[j] = if a[i - 1] == b[j - 1] {
                prev
            } else {
                1 + prev.min(row[j]).min(row[j - 1])
            };
            prev = curr;
        }
    }
    row[m]
}

/// Maximum edit distance allowed for a query of a given character length.
fn edit_threshold(query_chars: usize) -> usize {
    match query_chars {
        0..=3 => 0, // very short — no Levenshtein (too many false positives)
        4..=6 => 1, // up to 1 typo
        _ => 2,     // up to 2 typos
    }
}

/// Return `true` if `query` fuzzily matches `candidate`.
///
/// Checks (in order of cost):
/// 1. `candidate` contains `query` as a case-insensitive substring.
/// 2. Every query word is a prefix of some candidate word.
/// 3. Edit distance ≤ threshold (queries ≥ 4 chars only).
pub fn fuzzy_matches(query: &str, candidate: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    let c = candidate.to_lowercase();

    // 1. Substring
    if c.contains(&q) {
        return true;
    }

    // 2. All query words appear as prefixes of some candidate word
    let q_words: Vec<&str> = q.split_whitespace().collect();
    let c_words: Vec<&str> = c.split_whitespace().collect();
    if !q_words.is_empty()
        && q_words
            .iter()
            .all(|qw| c_words.iter().any(|cw| cw.starts_with(qw)))
    {
        return true;
    }

    // 3. Levenshtein
    let threshold = edit_threshold(q.chars().count());
    if threshold > 0 {
        if levenshtein(&q, &c) <= threshold {
            return true;
        }
        if c_words.iter().any(|cw| levenshtein(&q, cw) <= threshold) {
            return true;
        }
    }

    false
}

// ── Search service ────────────────────────────────────────────────────────────

impl SearchService {
    /// Create a new `SearchService` backed by the given connection pool.
    pub fn new(pool: DbPool) -> Self {
        SearchService { pool }
    }

    /// Search across tracks, albums, and artists using three complementary methods
    /// that always run and whose results are merged (deduplication by ID):
    ///
    /// 1. **FTS5 prefix** (`term*`) — fast, relevance-ordered, case/diacritic-insensitive.
    ///    Handles partial-word typing ("Coltr" → "Coltrane").
    /// 2. **LIKE `%query%`** — catches tracks that are absent from the FTS index
    ///    (e.g. FTS out of sync). Always runs alongside FTS5.
    /// 3. **Levenshtein** — adds typo tolerance ("trak" → "track").
    ///    Runs for queries ≥ 4 chars, only on candidates not already found.
    pub fn search(&self, query: &str) -> Result<SearchResults, LibraryError> {
        if query.trim().is_empty() {
            return Ok(SearchResults {
                tracks: Vec::new(),
                albums: Vec::new(),
                artists: Vec::new(),
            });
        }

        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        })?;

        let fts_query = sanitize_fts_query(query);
        let like_q = sanitize_like(query.trim());
        let q_lower = query.trim().to_lowercase();
        let query_chars = q_lower.chars().count();

        // ── Tracks ────────────────────────────────────────────────────────────
        let mut track_seen: HashSet<i64> = HashSet::new();
        let mut track_ids: Vec<i64> = Vec::new();

        // 1. FTS5 prefix
        if !fts_query.is_empty() {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT t.id FROM tracks t
                 JOIN tracks_fts fts ON fts.rowid = t.id
                 WHERE tracks_fts MATCH ?1
                 ORDER BY rank LIMIT 50",
            ) {
                if let Ok(ids) = stmt
                    .query_map([&fts_query], |row| row.get(0))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<i64>>>())
                {
                    for id in ids {
                        if track_seen.insert(id) {
                            track_ids.push(id);
                        }
                    }
                }
            }
        }

        // 2. LIKE substring — always, deduplicates with FTS5 results
        if !like_q.is_empty() {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT t.id FROM tracks t
                 JOIN artists ar ON ar.id = t.artist_id
                 WHERE t.title LIKE '%' || ?1 || '%'
                    OR ar.name  LIKE '%' || ?1 || '%'
                 LIMIT 50",
            ) {
                if let Ok(ids) = stmt
                    .query_map([&like_q], |row| row.get(0))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<i64>>>())
                {
                    for id in ids {
                        if track_seen.insert(id) {
                            track_ids.push(id);
                        }
                    }
                }
            }
        }

        // 3. Levenshtein — only for queries ≥ 4 chars, adds candidates not yet found
        if query_chars >= 4 {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT t.id, t.title, ar.name
                 FROM tracks t
                 JOIN artists ar ON ar.id = t.artist_id",
            ) {
                let new_ids: Vec<i64> = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })
                    .map_err(LibraryError::Database)?
                    .filter_map(|r| r.ok())
                    .filter(|(id, title, artist)| {
                        !track_seen.contains(id)
                            && (fuzzy_matches(&q_lower, title) || fuzzy_matches(&q_lower, artist))
                    })
                    .take(50)
                    .map(|(id, _, _)| id)
                    .collect();
                for id in new_ids {
                    if track_seen.insert(id) {
                        track_ids.push(id);
                    }
                }
            }
        }

        track_ids.truncate(50);

        let mut tracks = Vec::new();
        for id in track_ids {
            if let Ok(Some(track)) = queries::get_track(&conn, id) {
                tracks.push(track);
            }
        }

        // ── Albums ────────────────────────────────────────────────────────────
        let mut album_seen: HashSet<i64> = HashSet::new();
        let mut album_ids: Vec<i64> = Vec::new();

        // 1. FTS5 prefix
        if !fts_query.is_empty() {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT a.id FROM albums a
                 JOIN albums_fts fts ON fts.rowid = a.id
                 WHERE albums_fts MATCH ?1
                 ORDER BY rank LIMIT 20",
            ) {
                if let Ok(ids) = stmt
                    .query_map([&fts_query], |row| row.get(0))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<i64>>>())
                {
                    for id in ids {
                        if album_seen.insert(id) {
                            album_ids.push(id);
                        }
                    }
                }
            }
        }

        // 2. LIKE substring
        if !like_q.is_empty() {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT al.id FROM albums al
                 JOIN artists ar ON ar.id = al.artist_id
                 WHERE al.title LIKE '%' || ?1 || '%'
                    OR ar.name  LIKE '%' || ?1 || '%'
                 LIMIT 20",
            ) {
                if let Ok(ids) = stmt
                    .query_map([&like_q], |row| row.get(0))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<i64>>>())
                {
                    for id in ids {
                        if album_seen.insert(id) {
                            album_ids.push(id);
                        }
                    }
                }
            }
        }

        // 3. Levenshtein
        if query_chars >= 4 {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT al.id, al.title, ar.name
                 FROM albums al
                 JOIN artists ar ON ar.id = al.artist_id",
            ) {
                let new_ids: Vec<i64> = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })
                    .map_err(LibraryError::Database)?
                    .filter_map(|r| r.ok())
                    .filter(|(id, title, artist)| {
                        !album_seen.contains(id)
                            && (fuzzy_matches(&q_lower, title) || fuzzy_matches(&q_lower, artist))
                    })
                    .take(20)
                    .map(|(id, _, _)| id)
                    .collect();
                for id in new_ids {
                    if album_seen.insert(id) {
                        album_ids.push(id);
                    }
                }
            }
        }

        album_ids.truncate(20);

        let albums: Vec<Album> = album_ids
            .iter()
            .filter_map(|&id| {
                conn.query_row(
                    "SELECT a.id, a.title, a.artist_id, ar.name, a.year, a.rating,
                        a.artwork_path, a.online_artwork_path, a.description,
                        a.musicbrainz_id, a.label, a.country, a.barcode, a.album_type, a.release_status
                     FROM albums a
                     JOIN artists ar ON ar.id = a.artist_id
                     WHERE a.id = ?1",
                    rusqlite::params![id],
                    |row| {
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
                    },
                )
                .ok()
            })
            .collect();

        // ── Artists ───────────────────────────────────────────────────────────
        let mut artist_seen: HashSet<i64> = HashSet::new();
        let mut artist_ids: Vec<i64> = Vec::new();

        // 1. FTS5 prefix
        if !fts_query.is_empty() {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT ar.id FROM artists ar
                 JOIN artists_fts fts ON fts.rowid = ar.id
                 WHERE artists_fts MATCH ?1
                 ORDER BY rank LIMIT 20",
            ) {
                if let Ok(ids) = stmt
                    .query_map([&fts_query], |row| row.get(0))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<i64>>>())
                {
                    for id in ids {
                        if artist_seen.insert(id) {
                            artist_ids.push(id);
                        }
                    }
                }
            }
        }

        // 2. LIKE substring
        if !like_q.is_empty() {
            if let Ok(mut stmt) =
                conn.prepare("SELECT id FROM artists WHERE name LIKE '%' || ?1 || '%' LIMIT 20")
            {
                if let Ok(ids) = stmt
                    .query_map([&like_q], |row| row.get(0))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<i64>>>())
                {
                    for id in ids {
                        if artist_seen.insert(id) {
                            artist_ids.push(id);
                        }
                    }
                }
            }
        }

        // 3. Levenshtein
        if query_chars >= 4 {
            if let Ok(mut stmt) = conn.prepare("SELECT id, name FROM artists") {
                let new_ids: Vec<i64> = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(LibraryError::Database)?
                    .filter_map(|r| r.ok())
                    .filter(|(id, name)| !artist_seen.contains(id) && fuzzy_matches(&q_lower, name))
                    .take(20)
                    .map(|(id, _)| id)
                    .collect();
                for id in new_ids {
                    if artist_seen.insert(id) {
                        artist_ids.push(id);
                    }
                }
            }
        }

        artist_ids.truncate(20);

        let artists: Vec<Artist> = artist_ids
            .iter()
            .filter_map(|&id| {
                conn.query_row(
                    "SELECT id, name, name_sort, bio, country, genre, style, mood,
                        formed_year, born_year, died_year, disbanded, musicbrainz_id,
                        theaudiodb_id, photo_path
                     FROM artists WHERE id = ?1",
                    rusqlite::params![id],
                    |row| {
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
                            photo_path: row
                                .get::<_, Option<String>>(14)?
                                .map(std::path::PathBuf::from),
                        })
                    },
                )
                .ok()
            })
            .collect();

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

    // ── sanitize_fts_query ────────────────────────────────────────────────

    #[test]
    fn test_sanitize_fts_query_adds_prefix_star() {
        assert_eq!(sanitize_fts_query("Coltr"), "Coltr*");
    }

    #[test]
    fn test_sanitize_fts_query_multi_word() {
        assert_eq!(sanitize_fts_query("Kind Bl"), "Kind* Bl*");
    }

    #[test]
    fn test_sanitize_fts_query_strips_special_chars() {
        assert_eq!(sanitize_fts_query("(bad%)"), "bad*");
    }

    #[test]
    fn test_sanitize_fts_query_strips_double_quote() {
        assert_eq!(sanitize_fts_query("\"phrase\""), "phrase*");
    }

    #[test]
    fn test_sanitize_fts_query_empty() {
        assert_eq!(sanitize_fts_query(""), "");
    }

    #[test]
    fn test_sanitize_fts_query_only_special_chars() {
        assert_eq!(sanitize_fts_query("\"()%"), "");
    }

    // ── sanitize_like ─────────────────────────────────────────────────────

    #[test]
    fn test_sanitize_like_removes_percent() {
        assert_eq!(sanitize_like("100%"), "100");
    }

    #[test]
    fn test_sanitize_like_removes_underscore() {
        assert_eq!(sanitize_like("_test"), "test");
    }

    #[test]
    fn test_sanitize_like_removes_backslash() {
        assert_eq!(sanitize_like("a\\b"), "ab");
    }

    #[test]
    fn test_sanitize_like_passthrough_normal() {
        assert_eq!(sanitize_like("Incassable"), "Incassable");
    }

    // ── levenshtein ───────────────────────────────────────────────────────

    #[test]
    fn test_levenshtein_identical() {
        assert_eq!(levenshtein("abc", "abc"), 0);
    }

    #[test]
    fn test_levenshtein_empty_a() {
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn test_levenshtein_empty_b() {
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn test_levenshtein_one_insertion() {
        assert_eq!(levenshtein("coltrne", "coltrane"), 1);
    }

    #[test]
    fn test_levenshtein_one_substitution() {
        assert_eq!(levenshtein("trak", "track"), 1);
    }

    // ── fuzzy_matches ─────────────────────────────────────────────────────

    #[test]
    fn test_fuzzy_matches_substring() {
        assert!(fuzzy_matches("coltr", "John Coltrane"));
    }

    #[test]
    fn test_fuzzy_matches_word_prefix() {
        assert!(fuzzy_matches("Kind Bl", "Kind of Blue"));
    }

    #[test]
    fn test_fuzzy_matches_case_insensitive() {
        assert!(fuzzy_matches("COLTRANE", "John Coltrane"));
    }

    #[test]
    fn test_fuzzy_matches_one_typo() {
        assert!(fuzzy_matches("coltrne", "John Coltrane"));
    }

    #[test]
    fn test_fuzzy_matches_trak_finds_track() {
        // "trak" is 4 chars, threshold = 1, distance from "track" = 1
        assert!(fuzzy_matches("trak", "Track One"));
    }

    #[test]
    fn test_fuzzy_matches_no_match() {
        assert!(!fuzzy_matches("beethoven", "John Coltrane"));
    }

    #[test]
    fn test_fuzzy_matches_empty_query() {
        assert!(fuzzy_matches("", "anything"));
    }

    #[test]
    fn test_fuzzy_matches_three_char_no_levenshtein() {
        // 3-char query → threshold 0 → no Levenshtein, only substring/prefix
        assert!(!fuzzy_matches("xzq", "John Coltrane"));
    }

    // ── integration: search() ─────────────────────────────────────────────

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
        let results = service.search("Track").unwrap();
        assert!(
            !results.tracks.is_empty(),
            "must find tracks by title token"
        );
    }

    #[test]
    fn test_search_track_by_exact_title_via_fts() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("One").unwrap();
        assert_eq!(
            results.tracks.len(),
            1,
            "only 'Track One' contains token 'One'"
        );
        assert_eq!(results.tracks[0].title, "Track One");
    }

    #[test]
    fn test_search_track_by_artist_name_via_fts() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("Artist").unwrap();
        assert!(
            !results.tracks.is_empty(),
            "must find tracks by artist name token"
        );
    }

    #[test]
    fn test_search_track_by_prefix() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("Trac").unwrap();
        assert!(
            !results.tracks.is_empty(),
            "prefix 'Trac' must find 'Track One'"
        );
    }

    #[test]
    fn test_search_album_by_prefix() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("Test Al").unwrap();
        assert!(
            !results.albums.is_empty(),
            "prefix must find album by partial title"
        );
    }

    #[test]
    fn test_search_artist_by_prefix() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("Test Art").unwrap();
        assert!(
            !results.artists.is_empty(),
            "prefix must find artist by partial name"
        );
    }

    /// Regression: a track absent from tracks_fts must still appear via LIKE.
    /// Reproduces the "02 - Incassable" / "inc" scenario.
    #[test]
    fn test_search_track_found_via_like_when_fts_missing() {
        let env = TestEnv::new();
        env.seed_basic_library();

        // Insert a track WITHOUT firing the FTS trigger (simulates out-of-sync index).
        {
            let conn = env.pool.get().unwrap();
            conn.execute(
                "INSERT INTO tracks (title, artist_id, album_id, source_id, file_path,
                    duration, disc_number, file_type, file_size, rating, is_duplicate, play_count)
                 SELECT 'Incassable', artist_id, album_id, source_id,
                    '/music/incassable.flac', 200000, 1, 'flac', 10000000, 0, 0, 0
                 FROM tracks LIMIT 1",
                [],
            )
            .unwrap();
            // Deliberately skip the FTS insert to simulate missing FTS data.
        }

        let service = SearchService::new(env.pool.clone());
        // "inc" is a substring of "Incassable" — LIKE must find it.
        let results = service.search("inc").unwrap();
        assert!(
            results.tracks.iter().any(|t| t.title == "Incassable"),
            "LIKE must find 'Incassable' even when absent from tracks_fts"
        );
    }

    #[test]
    fn test_search_track_fuzzy_typo_fallback() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        // "Trak" is 1 edit from "Track" — Levenshtein must find it
        let results = service.search("Trak").unwrap();
        assert!(
            !results.tracks.is_empty(),
            "Levenshtein must find 'Track One' with 'Trak'"
        );
    }

    #[test]
    fn test_search_artist_fuzzy_typo_fallback() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        // "Test Artst" is 1 edit from "Test Artist"
        let results = service.search("Test Artst").unwrap();
        assert!(
            !results.artists.is_empty(),
            "Levenshtein must find 'Test Artist' with 'Test Artst'"
        );
    }

    // ── Special characters — must not panic ───────────────────────────────

    #[test]
    fn test_search_fts5_double_quote_does_not_panic() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        assert!(service.search("\"").is_ok());
    }

    #[test]
    fn test_search_fts5_unclosed_paren_does_not_panic() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        assert!(service.search("(unclosed").is_ok());
    }

    #[test]
    fn test_search_fts5_boolean_operators_do_not_panic() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
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

    #[test]
    fn test_search_percent_returns_ok_empty() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        // "%" stripped by sanitize_like → no LIKE query; sanitize_fts → empty; 1 char < 4 → no Levenshtein
        let result = service.search("%").unwrap();
        assert!(result.tracks.is_empty());
        assert!(result.albums.is_empty());
        assert!(result.artists.is_empty());
    }

    #[test]
    fn test_search_underscore_matches_via_levenshtein() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        // "_est Artist" stripped by sanitize_like to "est Artist" (LIKE finds nothing),
        // but Levenshtein("_est artist", "test artist") = 1 ≤ 2 → found
        let results = service.search("_est Artist").unwrap();
        assert!(!results.artists.is_empty());
    }

    // ── Result capping ────────────────────────────────────────────────────

    #[test]
    fn test_search_results_capped_per_type() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("Test").unwrap();
        assert!(results.tracks.len() <= 50);
        assert!(results.albums.len() <= 20);
        assert!(results.artists.len() <= 20);
    }

    #[test]
    fn test_search_empty_library_returns_all_empty() {
        let env = TestEnv::new();
        let service = SearchService::new(env.pool.clone());
        let results = service.search("anything").unwrap();
        assert!(results.tracks.is_empty());
        assert!(results.albums.is_empty());
        assert!(results.artists.is_empty());
    }
}
