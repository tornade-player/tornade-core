// Shared test utilities for tornade-core

use crate::db::{self, DbPool};
use crate::db::queries;
use crate::models::AudioFormat;
use crate::models::source::SourceType;
use crate::utils::paths::AppPaths;
use std::path::PathBuf;
use tempfile::TempDir;

/// Test environment with in-memory database and temp directories
pub struct TestEnv {
    pub _tmp: TempDir,
    pub pool: DbPool,
    pub app_paths: AppPaths,
}

impl TestEnv {
    /// Create a new test environment with initialized database
    pub fn new() -> Self {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let db_path = tmp.path().join("test.db");
        let pool = db::create_pool(db_path).expect("Failed to create pool");
        db::initialize_database(&pool).expect("Failed to init database");

        // Build AppPaths pointing to the temp dir
        let base = tmp.path().join(".config").join("tornade");
        std::fs::create_dir_all(base.join("cache").join("artwork")).unwrap();
        std::fs::create_dir_all(base.join("assets").join("albums")).unwrap();
        std::fs::create_dir_all(base.join("assets").join("artists")).unwrap();
        std::fs::create_dir_all(base.join("reports")).unwrap();

        let app_paths = AppPaths {
            config_dir: base.clone(),
            data_dir: base.clone(),
            cache_dir: base.join("cache"),
        };

        TestEnv { _tmp: tmp, pool, app_paths }
    }

    /// Seed basic library data: 1 source, 1 artist, 1 album, 1 genre, 2 tracks
    /// Returns (source_id, artist_id, album_id, genre_id, track1_id, track2_id)
    pub fn seed_basic_library(&self) -> (i64, i64, i64, i64, i64, i64) {
        let conn = self.pool.get().unwrap();

        let source_id = queries::insert_source(&conn, "Test Library", SourceType::Disk, Some(&PathBuf::from("/music"))).unwrap();
        let artist_id = queries::insert_artist(&conn, "Test Artist", Some("Artist, Test")).unwrap();
        let album_id = queries::insert_album(&conn, "Test Album", artist_id, Some(2024)).unwrap();
        let genre_id = queries::insert_genre(&conn, "Rock").unwrap();

        let track1_id = queries::insert_track(
            &conn, "Track One", Some(album_id), artist_id, source_id,
            &PathBuf::from("/music/track1.flac"), 240_000, Some(1),
            Some(44100), Some(16), AudioFormat::Flac, 30_000_000,
        ).unwrap();

        let track2_id = queries::insert_track(
            &conn, "Track Two", Some(album_id), artist_id, source_id,
            &PathBuf::from("/music/track2.flac"), 180_000, Some(2),
            Some(44100), Some(16), AudioFormat::Flac, 25_000_000,
        ).unwrap();

        queries::link_track_genre(&conn, track1_id, genre_id).unwrap();
        queries::link_track_genre(&conn, track2_id, genre_id).unwrap();

        (source_id, artist_id, album_id, genre_id, track1_id, track2_id)
    }
}
