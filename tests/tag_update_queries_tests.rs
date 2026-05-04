//! Integration tests for tag-editor database query functions:
//! `update_track_metadata`, `update_album_metadata`, and `get_distinct_*`.
//!
//! Uses an in-memory SQLite database with the full initialised schema for isolation.

use std::path::PathBuf;
use tempfile::TempDir;
use tornade_core::db::queries::{
    self, AlbumTagUpdate, get_distinct_album_titles, get_distinct_artist_names,
    get_distinct_genre_names, get_distinct_years, update_album_metadata, update_track_metadata,
};
use tornade_core::db::{self as db_mod, DbPool};
use tornade_core::models::AudioFormat;
use tornade_core::models::source::SourceType;
use tornade_core::services::TrackTagUpdate;

// ============================================================================
// Test setup helpers
// ============================================================================

/// Minimal test environment: a temp-dir-backed pool with the full schema applied.
struct TestEnv {
    _tmp: TempDir,
    pool: DbPool,
}

impl TestEnv {
    fn new() -> Self {
        let tmp = TempDir::new().expect("temp dir");
        let db_path = tmp.path().join("test.db");
        let pool = db_mod::create_pool(db_path).expect("pool");
        db_mod::initialize_database(&pool).expect("schema");
        TestEnv { _tmp: tmp, pool }
    }

    /// Seed: 1 source, 1 artist, 1 album, 1 genre ("Rock"), 2 tracks.
    /// Returns `(source_id, artist_id, album_id, genre_id, track1_id, track2_id)`.
    fn seed_basic_library(&self) -> (i64, i64, i64, i64, i64, i64) {
        let conn = self.pool.get().unwrap();
        let source_id = queries::insert_source(
            &conn,
            "Test Library",
            SourceType::Disk,
            Some(&PathBuf::from("/music")),
        )
        .unwrap();
        let artist_id = queries::insert_artist(&conn, "Test Artist", Some("Artist, Test")).unwrap();
        let album_id = queries::insert_album(&conn, "Test Album", artist_id, Some(2024)).unwrap();
        let genre_id = queries::insert_genre(&conn, "Rock").unwrap();
        let track1_id = queries::insert_track(
            &conn,
            "Track One",
            Some(album_id),
            artist_id,
            source_id,
            &PathBuf::from("/music/track1.flac"),
            240_000,
            Some(1),
            Some(44100),
            Some(16),
            AudioFormat::Flac,
            30_000_000,
        )
        .unwrap();
        let track2_id = queries::insert_track(
            &conn,
            "Track Two",
            Some(album_id),
            artist_id,
            source_id,
            &PathBuf::from("/music/track2.flac"),
            180_000,
            Some(2),
            Some(44100),
            Some(16),
            AudioFormat::Flac,
            25_000_000,
        )
        .unwrap();
        queries::link_track_genre(&conn, track1_id, genre_id).unwrap();
        queries::link_track_genre(&conn, track2_id, genre_id).unwrap();
        (
            source_id, artist_id, album_id, genre_id, track1_id, track2_id,
        )
    }
}

// ============================================================================
// update_track_metadata — artist upsert
// ============================================================================

#[test]
fn test_update_track_metadata_upserts_artist() {
    let env = TestEnv::new();
    let (_, _artist_id, album_id, _, track_id, _) = env.seed_basic_library();
    let conn = env.pool.get().unwrap();

    let update = TrackTagUpdate {
        title: "Renamed Track".to_string(),
        artist_name: "New Artist".to_string(),
        album_title: Some("Test Album".to_string()),
        album_artist_name: None,
        year: Some(2025),
        genre_names: vec!["Jazz".to_string()],
        track_number: Some(1),
        disc_number: None,
    };

    update_track_metadata(&conn, track_id, &update).unwrap();

    // Artist "New Artist" should exist in the DB now.
    let new_artist_id: i64 = conn
        .query_row(
            "SELECT id FROM artists WHERE name = 'New Artist'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(new_artist_id > 0);

    // Track's artist_id should point to the new artist.
    let track_artist_id: i64 = conn
        .query_row(
            "SELECT artist_id FROM tracks WHERE id = ?1",
            rusqlite::params![track_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(track_artist_id, new_artist_id);

    // Track title should be updated.
    let title: String = conn
        .query_row(
            "SELECT title FROM tracks WHERE id = ?1",
            rusqlite::params![track_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(title, "Renamed Track");

    // album_id should be set (album was upserted, not deleted because track moved to same-named album)
    let stored_album_id: Option<i64> = conn
        .query_row(
            "SELECT album_id FROM tracks WHERE id = ?1",
            rusqlite::params![track_id],
            |row| row.get(0),
        )
        .unwrap();
    // The album title "Test Album" already exists; it should be reused or a new one created
    // under the new artist. Either way, album_id is Some.
    assert!(stored_album_id.is_some());
    let _ = album_id; // original album may still exist (track2 is still on it)
}

#[test]
fn test_update_track_metadata_replaces_genres() {
    let env = TestEnv::new();
    let (_, _, _, _, track_id, _) = env.seed_basic_library();
    let conn = env.pool.get().unwrap();

    let update = TrackTagUpdate {
        title: "Track One".to_string(),
        artist_name: "Test Artist".to_string(),
        album_title: Some("Test Album".to_string()),
        album_artist_name: None,
        year: Some(2024),
        genre_names: vec!["Electronic".to_string(), "Ambient".to_string()],
        track_number: Some(1),
        disc_number: None,
    };

    update_track_metadata(&conn, track_id, &update).unwrap();

    // Count genres linked to the track.
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM track_genres WHERE track_id = ?1",
            rusqlite::params![track_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2, "Expected exactly 2 genres after replacement");

    // Old genre "Rock" should NOT be linked.
    let rock_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM track_genres tg
              JOIN genres g ON g.id = tg.genre_id
             WHERE tg.track_id = ?1 AND g.name = 'Rock'",
            rusqlite::params![track_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rock_count, 0, "Old genre 'Rock' should have been removed");
}

// ============================================================================
// update_track_metadata — auto-delete empty album
// ============================================================================

#[test]
fn test_update_track_metadata_auto_deletes_empty_album() {
    let env = TestEnv::new();
    let conn = env.pool.get().unwrap();

    // Create a source + artist + album with exactly ONE track.
    let source_id = queries::insert_source(
        &conn,
        "Solo Source",
        SourceType::Disk,
        Some(&PathBuf::from("/music")),
    )
    .unwrap();
    let artist_id = queries::insert_artist(&conn, "Solo Artist", None).unwrap();
    let old_album_id = queries::insert_album(&conn, "Old Album", artist_id, None).unwrap();
    let genre_id = queries::insert_genre(&conn, "Blues").unwrap();

    let track_id = queries::insert_track(
        &conn,
        "Solo Track",
        Some(old_album_id),
        artist_id,
        source_id,
        &PathBuf::from("/music/solo.flac"),
        200_000,
        Some(1),
        Some(44100),
        Some(16),
        AudioFormat::Flac,
        20_000_000,
    )
    .unwrap();
    queries::link_track_genre(&conn, track_id, genre_id).unwrap();

    // Verify old album exists.
    let before: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM albums WHERE id = ?1",
            rusqlite::params![old_album_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(before, 1, "Old album should exist before update");

    // Move track to a brand-new album.
    let update = TrackTagUpdate {
        title: "Solo Track".to_string(),
        artist_name: "Solo Artist".to_string(),
        album_title: Some("New Album".to_string()),
        album_artist_name: None,
        year: None,
        genre_names: vec!["Blues".to_string()],
        track_number: Some(1),
        disc_number: None,
    };
    update_track_metadata(&conn, track_id, &update).unwrap();

    // Old album must be gone (it had 0 tracks left).
    let after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM albums WHERE id = ?1",
            rusqlite::params![old_album_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, 0, "Empty old album should have been auto-deleted");

    // Track should now be on the new album.
    let new_album_title: String = conn
        .query_row(
            "SELECT a.title FROM tracks t JOIN albums a ON a.id = t.album_id WHERE t.id = ?1",
            rusqlite::params![track_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(new_album_title, "New Album");
}

#[test]
fn test_update_track_metadata_keeps_album_when_other_tracks_remain() {
    let env = TestEnv::new();
    let (_, _, album_id, _, track1_id, _) = env.seed_basic_library();
    let conn = env.pool.get().unwrap();

    // Move only track1 to a new album; track2 stays on the original album.
    let update = TrackTagUpdate {
        title: "Track One".to_string(),
        artist_name: "Test Artist".to_string(),
        album_title: Some("Different Album".to_string()),
        album_artist_name: None,
        year: None,
        genre_names: vec![],
        track_number: Some(1),
        disc_number: None,
    };
    update_track_metadata(&conn, track1_id, &update).unwrap();

    // Original album must still exist because track2 is on it.
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM albums WHERE id = ?1",
            rusqlite::params![album_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "Album should survive when track2 is still linked");
}

// ============================================================================
// update_album_metadata — genre replacement for all tracks
// ============================================================================

#[test]
fn test_update_album_metadata_replaces_genres() {
    let env = TestEnv::new();
    let (_, artist_id, album_id, _, track1_id, track2_id) = env.seed_basic_library();
    let conn = env.pool.get().unwrap();

    // Baseline: both tracks have the "Rock" genre.
    let rock_links_before: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM track_genres tg
              JOIN genres g ON g.id = tg.genre_id
             WHERE tg.track_id IN (?1, ?2) AND g.name = 'Rock'",
            rusqlite::params![track1_id, track2_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rock_links_before, 2);

    let update = AlbumTagUpdate {
        title: "Test Album".to_string(),
        artist_name: "Test Artist".to_string(),
        year: Some(2024),
        genre_names: vec!["Classical".to_string(), "Orchestral".to_string()],
    };
    let affected = update_album_metadata(&conn, album_id, &update).unwrap();
    assert_eq!(affected, 2, "Should report 2 affected tracks");

    // Rock must be gone from both tracks.
    let rock_links_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM track_genres tg
              JOIN genres g ON g.id = tg.genre_id
             WHERE tg.track_id IN (?1, ?2) AND g.name = 'Rock'",
            rusqlite::params![track1_id, track2_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        rock_links_after, 0,
        "Old genre 'Rock' must be removed from both tracks"
    );

    // Both new genres should be linked to both tracks.
    let new_genre_links: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM track_genres tg
              JOIN genres g ON g.id = tg.genre_id
             WHERE tg.track_id IN (?1, ?2)
               AND g.name IN ('Classical', 'Orchestral')",
            rusqlite::params![track1_id, track2_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        new_genre_links, 4,
        "Each of 2 genres should be linked to each of 2 tracks (2x2=4)"
    );

    // album title and artist should be updated.
    let (stored_title, stored_artist_id): (String, i64) = conn
        .query_row(
            "SELECT title, artist_id FROM albums WHERE id = ?1",
            rusqlite::params![album_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored_title, "Test Album");
    assert_eq!(stored_artist_id, artist_id);
}

#[test]
fn test_update_album_metadata_clears_genres_when_empty_list() {
    let env = TestEnv::new();
    let (_, _, album_id, _, track1_id, track2_id) = env.seed_basic_library();
    let conn = env.pool.get().unwrap();

    let update = AlbumTagUpdate {
        title: "Test Album".to_string(),
        artist_name: "Test Artist".to_string(),
        year: None,
        genre_names: vec![],
    };
    let affected = update_album_metadata(&conn, album_id, &update).unwrap();
    assert_eq!(affected, 2);

    let remaining_genres: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM track_genres WHERE track_id IN (?1, ?2)",
            rusqlite::params![track1_id, track2_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining_genres, 0, "All genre links should be removed");
}

#[test]
fn test_update_album_metadata_upserts_new_artist() {
    let env = TestEnv::new();
    let (_, _, album_id, _, _, _) = env.seed_basic_library();
    let conn = env.pool.get().unwrap();

    let update = AlbumTagUpdate {
        title: "Remastered Album".to_string(),
        artist_name: "Brand New Artist".to_string(),
        year: Some(2025),
        genre_names: vec![],
    };
    update_album_metadata(&conn, album_id, &update).unwrap();

    // The new artist should exist.
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artists WHERE name = 'Brand New Artist'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "Brand New Artist should have been upserted");

    // Album title should be updated.
    let stored_title: String = conn
        .query_row(
            "SELECT title FROM albums WHERE id = ?1",
            rusqlite::params![album_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_title, "Remastered Album");
}

// ============================================================================
// get_distinct_* queries
// ============================================================================

#[test]
fn test_get_distinct_artist_names_returns_sorted() {
    let env = TestEnv::new();
    let conn = env.pool.get().unwrap();

    queries::insert_artist(&conn, "Zappa", None).unwrap();
    queries::insert_artist(&conn, "Amon Tobin", None).unwrap();
    queries::insert_artist(&conn, "Miles Davis", None).unwrap();

    let names = get_distinct_artist_names(&conn).unwrap();
    assert_eq!(names.len(), 3);
    // Must be sorted alphabetically.
    assert_eq!(names[0], "Amon Tobin");
    assert_eq!(names[1], "Miles Davis");
    assert_eq!(names[2], "Zappa");
}

#[test]
fn test_get_distinct_artist_names_deduplicates() {
    let env = TestEnv::new();
    let conn = env.pool.get().unwrap();

    queries::insert_artist(&conn, "Same Artist", None).unwrap();
    // INSERT OR IGNORE — second call should not create a duplicate.
    queries::insert_artist(&conn, "Same Artist", None).unwrap();

    let names = get_distinct_artist_names(&conn).unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0], "Same Artist");
}

#[test]
fn test_get_distinct_genre_names_returns_sorted() {
    let env = TestEnv::new();
    let conn = env.pool.get().unwrap();

    queries::insert_genre(&conn, "Rock").unwrap();
    queries::insert_genre(&conn, "Blues").unwrap();
    queries::insert_genre(&conn, "Jazz").unwrap();

    let genres = get_distinct_genre_names(&conn).unwrap();
    assert_eq!(genres, vec!["Blues", "Jazz", "Rock"]);
}

#[test]
fn test_get_distinct_album_titles_returns_sorted() {
    let env = TestEnv::new();
    let conn = env.pool.get().unwrap();

    let artist_id = queries::insert_artist(&conn, "Artist", None).unwrap();
    queries::insert_album(&conn, "Ziggy Stardust", artist_id, None).unwrap();
    queries::insert_album(&conn, "Aladdin Sane", artist_id, None).unwrap();
    queries::insert_album(&conn, "Heroes", artist_id, None).unwrap();

    let titles = get_distinct_album_titles(&conn).unwrap();
    assert_eq!(titles, vec!["Aladdin Sane", "Heroes", "Ziggy Stardust"]);
}

#[test]
fn test_get_distinct_years_returns_sorted_numerically() {
    let env = TestEnv::new();
    let conn = env.pool.get().unwrap();

    let source_id = queries::insert_source(
        &conn,
        "Source",
        SourceType::Disk,
        Some(&PathBuf::from("/music")),
    )
    .unwrap();
    let artist_id = queries::insert_artist(&conn, "Artist", None).unwrap();

    // Insert tracks with different years using the year field on the track.
    for (i, year) in [2003i64, 1975, 2022].iter().enumerate() {
        let album_id =
            queries::insert_album(&conn, &format!("Album {i}"), artist_id, None).unwrap();
        let track_id = queries::insert_track(
            &conn,
            &format!("Track {i}"),
            Some(album_id),
            artist_id,
            source_id,
            &PathBuf::from(format!("/music/track{i}.flac")),
            180_000,
            Some(1),
            Some(44100),
            Some(16),
            AudioFormat::Flac,
            10_000_000,
        )
        .unwrap();
        // Set year via direct SQL since insert_track doesn't accept year.
        conn.execute(
            "UPDATE tracks SET year = ?1 WHERE id = ?2",
            rusqlite::params![year, track_id],
        )
        .unwrap();
    }

    let years = get_distinct_years(&conn).unwrap();
    // Should be sorted numerically: 1975, 2003, 2022
    assert_eq!(years, vec!["1975", "2003", "2022"]);
}

#[test]
fn test_get_distinct_years_excludes_null() {
    let env = TestEnv::new();
    let conn = env.pool.get().unwrap();

    let source_id = queries::insert_source(
        &conn,
        "Source",
        SourceType::Disk,
        Some(&PathBuf::from("/music")),
    )
    .unwrap();
    let artist_id = queries::insert_artist(&conn, "Artist", None).unwrap();
    let album_id = queries::insert_album(&conn, "Album", artist_id, None).unwrap();

    // Track with no year (year = NULL in DB by default).
    queries::insert_track(
        &conn,
        "No Year Track",
        Some(album_id),
        artist_id,
        source_id,
        &PathBuf::from("/music/noyear.flac"),
        120_000,
        None,
        Some(44100),
        Some(16),
        AudioFormat::Flac,
        5_000_000,
    )
    .unwrap();

    let years = get_distinct_years(&conn).unwrap();
    assert!(years.is_empty(), "NULL years must be excluded");
}
