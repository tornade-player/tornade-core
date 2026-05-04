//! Duplicate track detection service.

use crate::db::DbPool;
use crate::models::Track;
use crate::services::error::LibraryError;
use log::info;
use std::collections::HashMap;

type Result<T> = std::result::Result<T, LibraryError>;

/// Represents a group of duplicate tracks
#[derive(Debug, Clone)]
pub struct DuplicateGroup {
    pub key: String,
    pub tracks: Vec<Track>,
}

/// Service for detecting duplicate tracks in the library
pub struct DuplicateService {
    pool: DbPool,
}

impl DuplicateService {
    /// Create a new `DuplicateService` backed by the given connection pool.
    pub fn new(pool: DbPool) -> Self {
        DuplicateService { pool }
    }

    /// Find duplicate tracks based on metadata (T125, T126)
    /// Groups tracks by (title + artist + duration) to identify potential duplicates
    pub fn find_duplicates(&self) -> Result<Vec<DuplicateGroup>> {
        let conn = self.pool.get()?;

        // Get all tracks
        let mut stmt = conn.prepare(
            "SELECT id, title, album_id, artist_id, source_id, file_path,
                    duration, track_number, disc_number, sample_rate, bit_depth,
                    file_type, file_size, rating, fingerprint, is_duplicate,
                    duplicate_of, last_played_at, play_count
             FROM tracks
             ORDER BY title, artist_id",
        )?;

        let tracks: Vec<Track> = stmt
            .query_map([], |row| {
                Ok(Track {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    album_id: row.get(2)?,
                    artist_id: row.get(3)?,
                    source_id: row.get(4)?,
                    file_path: std::path::PathBuf::from(row.get::<_, String>(5)?),
                    duration: std::time::Duration::from_millis(row.get::<_, i64>(6)? as u64),
                    track_number: row.get(7)?,
                    disc_number: row.get(8)?,
                    sample_rate: row.get(9)?,
                    bit_depth: row.get(10)?,
                    file_type: crate::models::AudioFormat::from_str(&row.get::<_, String>(11)?)
                        .unwrap(),
                    file_size: row.get::<_, i64>(12)? as u64,
                    rating: row.get(13)?,
                    fingerprint: row.get(14)?,
                    is_duplicate: row.get::<_, i32>(15)? != 0,
                    duplicate_of: row.get(16)?,
                    last_played_at: row.get(17)?,
                    play_count: row.get::<_, i32>(18)? as u32,
                    artist_names: vec![],
                    year: None,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        // Group tracks by (normalized_title, artist_id, duration_seconds)
        let mut groups: HashMap<String, Vec<Track>> = HashMap::new();

        for track in tracks {
            // Create a key for duplicate detection
            // Normalize title (lowercase, trim) + artist + duration (rounded to seconds)
            let normalized_title = track.title.to_lowercase().trim().to_string();
            let duration_secs = track.duration.as_secs();
            let key = format!("{}:{}:{}", normalized_title, track.artist_id, duration_secs);

            groups.entry(key).or_default().push(track);
        }

        // Filter to only groups with 2+ tracks (actual duplicates)
        let duplicates: Vec<DuplicateGroup> = groups
            .into_iter()
            .filter(|(_, tracks)| tracks.len() > 1)
            .map(|(key, tracks)| DuplicateGroup { key, tracks })
            .collect();

        info!("Found {} duplicate groups", duplicates.len());

        Ok(duplicates)
    }

    /// Mark a track as a duplicate (T127)
    /// This can be used to hide duplicates from the main library view
    pub fn mark_duplicate(
        &self,
        track_id: i64,
        is_duplicate: bool,
        duplicate_of: Option<i64>,
    ) -> Result<()> {
        let conn = self.pool.get()?;

        conn.execute(
            "UPDATE tracks SET is_duplicate = ?1, duplicate_of = ?2 WHERE id = ?3",
            rusqlite::params![i32::from(is_duplicate), duplicate_of, track_id],
        )?;

        info!("Marked track {track_id} as duplicate: {is_duplicate}");

        Ok(())
    }

    /// Hide a duplicate track from library views (T127)
    /// Marks it as a duplicate of another track
    pub fn hide_duplicate(&self, track_id: i64, original_track_id: i64) -> Result<()> {
        self.mark_duplicate(track_id, true, Some(original_track_id))
    }

    /// Unhide a track marked as duplicate
    pub fn unhide_duplicate(&self, track_id: i64) -> Result<()> {
        self.mark_duplicate(track_id, false, None)
    }

    /// Get statistics about duplicates in the library
    pub fn get_duplicate_stats(&self) -> Result<(usize, usize)> {
        let duplicates = self.find_duplicates()?;
        let num_groups = duplicates.len();
        let num_tracks: usize = duplicates.iter().map(|g| g.tracks.len()).sum();

        Ok((num_groups, num_tracks))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use tempfile::TempDir;

    #[test]
    fn test_find_duplicates_empty() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let pool = db::create_pool(db_path.clone()).unwrap();
        db::initialize_database(&pool).unwrap();

        let service = DuplicateService::new(pool);
        let duplicates = service.find_duplicates().unwrap();

        assert_eq!(duplicates.len(), 0);
    }

    #[test]
    fn test_find_duplicates_with_matches() {
        use crate::db::queries;
        use crate::models::{AudioFormat, source::SourceType};
        use crate::test_helpers::TestEnv;

        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source_id = queries::insert_source(&conn, "Lib", SourceType::Disk, None).unwrap();
        let artist_id = queries::insert_artist(&conn, "Artist", None).unwrap();

        // Two tracks with same title + artist + duration => duplicate
        queries::insert_track(
            &conn,
            "Same Song",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/a/1.flac"),
            180_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "Same Song",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/b/2.flac"),
            180_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        drop(conn);

        let service = DuplicateService::new(env.pool.clone());
        let duplicates = service.find_duplicates().unwrap();
        assert_eq!(duplicates.len(), 1);
        assert_eq!(duplicates[0].tracks.len(), 2);
    }

    #[test]
    fn test_mark_duplicate() {
        use crate::test_helpers::TestEnv;
        let env = TestEnv::new();
        let (_, _, _, _, t1, t2) = env.seed_basic_library();

        let service = DuplicateService::new(env.pool.clone());
        service.mark_duplicate(t1, true, Some(t2)).unwrap();

        let conn = env.pool.get().unwrap();
        let (is_dup, dup_of): (i32, Option<i64>) = conn
            .query_row(
                "SELECT is_duplicate, duplicate_of FROM tracks WHERE id = ?1",
                [t1],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(is_dup, 1);
        assert_eq!(dup_of, Some(t2));
    }

    #[test]
    fn test_hide_and_unhide_duplicate() {
        use crate::test_helpers::TestEnv;
        let env = TestEnv::new();
        let (_, _, _, _, t1, t2) = env.seed_basic_library();

        let service = DuplicateService::new(env.pool.clone());
        service.hide_duplicate(t1, t2).unwrap();

        let conn = env.pool.get().unwrap();
        let is_dup: i32 = conn
            .query_row("SELECT is_duplicate FROM tracks WHERE id = ?1", [t1], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(is_dup, 1);

        service.unhide_duplicate(t1).unwrap();
        let is_dup: i32 = conn
            .query_row("SELECT is_duplicate FROM tracks WHERE id = ?1", [t1], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(is_dup, 0);
    }

    #[test]
    fn test_get_duplicate_stats_empty() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let pool = db::create_pool(db_path.clone()).unwrap();
        db::initialize_database(&pool).unwrap();

        let service = DuplicateService::new(pool);
        let (groups, tracks) = service.get_duplicate_stats().unwrap();
        assert_eq!(groups, 0);
        assert_eq!(tracks, 0);
    }

    #[test]
    fn test_get_duplicate_stats_with_one_group() {
        use crate::db::queries;
        use crate::models::{AudioFormat, source::SourceType};
        use crate::test_helpers::TestEnv;

        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source_id = queries::insert_source(&conn, "Lib", SourceType::Disk, None).unwrap();
        let artist_id = queries::insert_artist(&conn, "Artist", None).unwrap();

        queries::insert_track(
            &conn,
            "Same Song",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/a/1.flac"),
            180_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "Same Song",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/b/2.flac"),
            180_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        drop(conn);

        let service = DuplicateService::new(env.pool.clone());
        let (groups, tracks) = service.get_duplicate_stats().unwrap();
        assert_eq!(groups, 1);
        assert_eq!(tracks, 2);
    }

    #[test]
    fn test_get_duplicate_stats_with_multiple_groups() {
        use crate::db::queries;
        use crate::models::{AudioFormat, source::SourceType};
        use crate::test_helpers::TestEnv;

        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source_id = queries::insert_source(&conn, "Lib", SourceType::Disk, None).unwrap();
        let artist_id = queries::insert_artist(&conn, "Artist", None).unwrap();

        // Group 1: "Song A" duplicated twice
        queries::insert_track(
            &conn,
            "Song A",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/a/1.flac"),
            120_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "Song A",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/b/1.flac"),
            120_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();

        // Group 2: "Song B" duplicated three times
        queries::insert_track(
            &conn,
            "Song B",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/a/2.flac"),
            240_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "Song B",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/b/2.flac"),
            240_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "Song B",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/c/2.flac"),
            240_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        drop(conn);

        let service = DuplicateService::new(env.pool.clone());
        let (groups, tracks) = service.get_duplicate_stats().unwrap();
        assert_eq!(groups, 2);
        assert_eq!(tracks, 5); // 2 + 3
    }

    #[test]
    fn test_find_duplicates_same_title_different_artist_not_duplicate() {
        use crate::db::queries;
        use crate::models::{AudioFormat, source::SourceType};
        use crate::test_helpers::TestEnv;

        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source_id = queries::insert_source(&conn, "Lib", SourceType::Disk, None).unwrap();
        let artist1 = queries::insert_artist(&conn, "Artist A", None).unwrap();
        let artist2 = queries::insert_artist(&conn, "Artist B", None).unwrap();

        // Same title + duration but different artist → not duplicates
        queries::insert_track(
            &conn,
            "Same Title",
            None,
            artist1,
            source_id,
            &std::path::PathBuf::from("/a/1.flac"),
            180_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "Same Title",
            None,
            artist2,
            source_id,
            &std::path::PathBuf::from("/b/1.flac"),
            180_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        drop(conn);

        let service = DuplicateService::new(env.pool.clone());
        let duplicates = service.find_duplicates().unwrap();
        assert!(
            duplicates.is_empty(),
            "same title with different artists must not be flagged as duplicates"
        );
    }

    #[test]
    fn test_find_duplicates_same_title_different_duration_not_duplicate() {
        use crate::db::queries;
        use crate::models::{AudioFormat, source::SourceType};
        use crate::test_helpers::TestEnv;

        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source_id = queries::insert_source(&conn, "Lib", SourceType::Disk, None).unwrap();
        let artist_id = queries::insert_artist(&conn, "Artist", None).unwrap();

        // Same title + artist but different duration → not duplicates
        queries::insert_track(
            &conn,
            "Same Song",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/a/1.flac"),
            180_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "Same Song",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/b/1.flac"),
            240_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        drop(conn);

        let service = DuplicateService::new(env.pool.clone());
        let duplicates = service.find_duplicates().unwrap();
        assert!(
            duplicates.is_empty(),
            "same title with different durations must not be duplicates"
        );
    }

    #[test]
    fn test_find_duplicates_title_case_insensitive() {
        use crate::db::queries;
        use crate::models::{AudioFormat, source::SourceType};
        use crate::test_helpers::TestEnv;

        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source_id = queries::insert_source(&conn, "Lib", SourceType::Disk, None).unwrap();
        let artist_id = queries::insert_artist(&conn, "Artist", None).unwrap();

        // Title differs only in case → should be detected as duplicate
        queries::insert_track(
            &conn,
            "My Song",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/a/1.flac"),
            180_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "MY SONG",
            None,
            artist_id,
            source_id,
            &std::path::PathBuf::from("/b/1.flac"),
            180_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1000,
        )
        .unwrap();
        drop(conn);

        let service = DuplicateService::new(env.pool.clone());
        let duplicates = service.find_duplicates().unwrap();
        assert_eq!(
            duplicates.len(),
            1,
            "title comparison must be case-insensitive"
        );
    }
}
