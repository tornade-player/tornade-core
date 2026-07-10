//! Reusable metadata-editing service.
//!
//! [`MetadataEditService`] writes a track's tags to BOTH the SQLite database
//! and the audio file on disk in a single call. It exists so that the terminal
//! UI (which links `tornade-core` as a Rust library) can perform tag edits
//! without going through the Swift FFI layer.
//!
//! Ordering guarantee: the database is updated first, then the audio file. If
//! the file write fails, the database has already been updated (mirroring the
//! existing FFI behaviour where DB and file writes are separate calls).

use crate::db::{DbPool, queries};
use crate::services::error::LibraryError;
use crate::services::tag_writer::{TagWriterService, TrackTagUpdate};
use chrono::Datelike;

/// Result of attempting to update a single track's metadata.
///
/// Used by both [`MetadataEditService::update_track`] and
/// [`MetadataEditService::update_tracks`] so callers get a uniform,
/// serialisable status per track.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrackUpdateResult {
    /// The track that was targeted.
    pub track_id: i64,
    /// `true` if both the database and audio-file writes succeeded.
    pub ok: bool,
    /// Human-readable error message when `ok` is `false`, otherwise `None`.
    pub error: Option<String>,
}

/// Writes track metadata to both the database and the audio file.
///
/// Constructed from an `r2d2` [`DbPool`] and cheap to clone/share.
pub struct MetadataEditService {
    pool: DbPool,
}

impl MetadataEditService {
    /// Create a new `MetadataEditService` backed by the given connection pool.
    pub fn new(pool: DbPool) -> Self {
        MetadataEditService { pool }
    }

    /// Update a single track's metadata in the database and the audio file.
    ///
    /// Steps:
    /// 1. Validate `year` (if `Some`, it must be within `1900..=current_year + 1`).
    /// 2. Update the database via [`queries::update_track_metadata`].
    /// 3. Read the freshly-stored values back with
    ///    [`queries::build_track_tag_update_from_db`] and write them to the
    ///    audio file with [`TagWriterService::write_track_tags`].
    ///
    /// # Errors
    ///
    /// Returns [`LibraryError::Metadata`] for an out-of-range year, a
    /// [`LibraryError::Pool`] error if a connection cannot be obtained, or any
    /// database/file error surfaced by the underlying operations.
    pub fn update_track(
        &self,
        track_id: i64,
        update: &TrackTagUpdate,
    ) -> Result<TrackUpdateResult, LibraryError> {
        validate_year(update.year)?;

        let conn = self.pool.get()?;

        // Database first.
        queries::update_track_metadata(&conn, track_id, update)?;

        // Audio file second: re-read the stored values so the file reflects the
        // database exactly (album/artist ids may have been resolved on write).
        let (file_path, stored) = queries::build_track_tag_update_from_db(&conn, track_id)?;
        let writer = TagWriterService::new();
        writer.write_track_tags(&file_path, &stored)?;

        Ok(TrackUpdateResult {
            track_id,
            ok: true,
            error: None,
        })
    }

    /// Load a track's CURRENT tag values from the database, for prefilling an
    /// editor.
    ///
    /// This is a thin wrapper over
    /// [`queries::build_track_tag_update_from_db`] that returns only the
    /// [`TrackTagUpdate`] (dropping the file path), so callers can obtain the
    /// current values without needing direct access to the connection pool.
    ///
    /// # Errors
    ///
    /// Returns a [`LibraryError::Pool`] error if a connection cannot be
    /// obtained, or any database error surfaced by the underlying query
    /// (e.g. an unknown `track_id`).
    pub fn current_track_update(&self, track_id: i64) -> Result<TrackTagUpdate, LibraryError> {
        let conn = self.pool.get()?;
        let (_path, update) = queries::build_track_tag_update_from_db(&conn, track_id)?;
        Ok(update)
    }

    /// Apply album-level metadata to multiple tracks, one at a time.
    ///
    /// For each track only the ALBUM-LEVEL fields are taken from `update`
    /// (`album_title`, `album_artist_name`, `year`, `genre_names`). Every
    /// track's own `title`, `artist_name`, `track_number` and `disc_number` are
    /// preserved by first loading its current values via
    /// [`queries::build_track_tag_update_from_db`] and only overriding the
    /// album-level fields. This is what an "edit album" flow wants: propagate
    /// the shared album fields without clobbering each track's identity.
    ///
    /// This method NEVER aborts on the first failure: it returns exactly one
    /// [`TrackUpdateResult`] per input id, in the same order, with `ok = false`
    /// and a populated `error` for any track that failed (e.g. an unknown id or
    /// a missing audio file).
    pub fn update_tracks(
        &self,
        track_ids: &[i64],
        update: &TrackTagUpdate,
    ) -> Vec<TrackUpdateResult> {
        track_ids
            .iter()
            .map(
                |&track_id| match self.update_track_album_fields(track_id, update) {
                    Ok(()) => TrackUpdateResult {
                        track_id,
                        ok: true,
                        error: None,
                    },
                    Err(e) => TrackUpdateResult {
                        track_id,
                        ok: false,
                        error: Some(e.to_string()),
                    },
                },
            )
            .collect()
    }

    /// Apply only the album-level fields of `update` to a single track,
    /// preserving that track's own identity fields. Writes DB then file.
    fn update_track_album_fields(
        &self,
        track_id: i64,
        update: &TrackTagUpdate,
    ) -> Result<(), LibraryError> {
        validate_year(update.year)?;

        let conn = self.pool.get()?;

        // Load the track's current values, then override ONLY album-level fields.
        let (_path, mut merged) = queries::build_track_tag_update_from_db(&conn, track_id)?;
        merged.album_title = update.album_title.clone();
        merged.album_artist_name = update.album_artist_name.clone();
        merged.year = update.year;
        merged.genre_names = update.genre_names.clone();

        // Database first.
        queries::update_track_metadata(&conn, track_id, &merged)?;

        // Audio file second, from the stored values.
        let (file_path, stored) = queries::build_track_tag_update_from_db(&conn, track_id)?;
        let writer = TagWriterService::new();
        writer.write_track_tags(&file_path, &stored)?;

        Ok(())
    }
}

/// Current year according to the system clock.
fn current_year() -> i32 {
    chrono::Local::now().year()
}

/// Validate an optional release year.
///
/// `None` is always accepted (year not set). A `Some` value must fall within
/// `1900..=current_year + 1` (the upper bound allows for pre-releases dated a
/// year ahead).
fn validate_year(year: Option<u16>) -> Result<(), LibraryError> {
    if let Some(y) = year {
        let y = i32::from(y);
        let max = current_year() + 1;
        if y < 1900 || y > max {
            return Err(LibraryError::Metadata(format!(
                "Invalid year: {y} (must be between 1900 and {max})"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries;
    use crate::test_helpers::TestEnv;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// Absolute path to the shared `tests/fixtures/minimal.flac` fixture.
    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.flac")
    }

    /// Read a track's stored title straight from the database.
    fn read_title(env: &TestEnv, track_id: i64) -> String {
        let conn = env.pool.get().unwrap();
        conn.query_row(
            "SELECT title FROM tracks WHERE id = ?1",
            rusqlite::params![track_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
    }

    /// Insert a track whose `file_path` points at a real, writable copy of the
    /// FLAC fixture in `dir`. Returns the new track id.
    fn seed_track_with_file(
        env: &TestEnv,
        dir: &Path,
        title: &str,
        track_number: Option<u32>,
    ) -> i64 {
        use crate::models::AudioFormat;
        use crate::models::source::SourceType;

        let conn = env.pool.get().unwrap();
        let source_id = queries::insert_source(
            &conn,
            "Src",
            SourceType::Disk,
            Some(&PathBuf::from("/music")),
        )
        .unwrap();
        let artist_id = queries::insert_artist(&conn, "Orig Artist", None).unwrap();
        let album_id = queries::insert_album(&conn, "Orig Album", artist_id, Some(2000)).unwrap();

        let file_name = format!("{title}.flac");
        let dst = dir.join(&file_name);
        std::fs::copy(fixture_path(), &dst).expect("copy fixture");

        queries::insert_track(
            &conn,
            title,
            Some(album_id),
            artist_id,
            source_id,
            &dst,
            120_000,
            track_number,
            Some(44100),
            Some(16),
            AudioFormat::Flac,
            10_000_000,
        )
        .unwrap()
    }

    fn base_update() -> TrackTagUpdate {
        TrackTagUpdate {
            title: "New Title".to_string(),
            artist_name: "New Artist".to_string(),
            album_title: Some("New Album".to_string()),
            album_artist_name: Some("New Album Artist".to_string()),
            year: Some(2020),
            genre_names: vec!["Jazz".to_string()],
            track_number: Some(1),
            disc_number: None,
        }
    }

    // (a) Year validation: out-of-range rejected, unset allowed. ------------

    #[test]
    fn test_year_validation_rejects_out_of_range() {
        assert!(validate_year(Some(1899)).is_err());
        assert!(validate_year(Some(1800)).is_err());

        let far_future = (current_year() + 5) as u16;
        assert!(validate_year(Some(far_future)).is_err());
    }

    #[test]
    fn test_year_validation_allows_none_and_in_range() {
        assert!(validate_year(None).is_ok());
        assert!(validate_year(Some(1900)).is_ok());
        assert!(validate_year(Some(2000)).is_ok());
        assert!(validate_year(Some(current_year() as u16)).is_ok());
        // current_year + 1 is the inclusive upper bound.
        assert!(validate_year(Some((current_year() + 1) as u16)).is_ok());
    }

    #[test]
    fn test_update_track_rejects_bad_year() {
        let env = TestEnv::new();
        let (_s, _ar, _al, _g, track1, _track2) = env.seed_basic_library();

        let svc = MetadataEditService::new(env.pool.clone());
        let mut update = base_update();
        update.year = Some(1000);

        let err = svc.update_track(track1, &update).unwrap_err();
        assert!(matches!(err, LibraryError::Metadata(_)));
    }

    // (b) update_tracks returns one result per id and never aborts. ---------

    #[test]
    fn test_update_tracks_one_result_per_id_no_abort_on_bad_id() {
        let fixture = fixture_path();
        if !fixture.exists() {
            eprintln!("SKIP: fixture not found at {fixture:?}");
            return;
        }

        let env = TestEnv::new();
        let dir = TempDir::new().unwrap();
        let good = seed_track_with_file(&env, dir.path(), "Good", Some(1));
        let bad = 999_999; // non-existent track id

        let svc = MetadataEditService::new(env.pool.clone());
        let update = base_update();

        // Order: bad first to prove we do NOT abort on the first failure.
        let results = svc.update_tracks(&[bad, good], &update);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].track_id, bad);
        assert!(!results[0].ok);
        assert!(results[0].error.is_some());

        assert_eq!(results[1].track_id, good);
        assert!(
            results[1].ok,
            "good track should succeed: {:?}",
            results[1].error
        );
        assert!(results[1].error.is_none());
    }

    // (c) update_tracks preserves per-track title, changes album-level fields.

    #[test]
    fn test_update_tracks_preserves_title_changes_album_fields() {
        let fixture = fixture_path();
        if !fixture.exists() {
            eprintln!("SKIP: fixture not found at {fixture:?}");
            return;
        }

        let env = TestEnv::new();
        let dir = TempDir::new().unwrap();
        let t1 = seed_track_with_file(&env, dir.path(), "Song Alpha", Some(1));
        let t2 = seed_track_with_file(&env, dir.path(), "Song Beta", Some(2));

        let svc = MetadataEditService::new(env.pool.clone());

        // Album-level update: title/artist_name here are placeholders and MUST
        // be ignored by update_tracks; only album fields propagate.
        let mut update = base_update();
        update.title = "PLACEHOLDER".to_string();
        update.artist_name = "PLACEHOLDER".to_string();
        update.album_title = Some("Shared Album".to_string());
        update.album_artist_name = Some("Shared Album Artist".to_string());
        update.year = Some(2021);
        update.genre_names = vec!["Ambient".to_string()];

        let results = svc.update_tracks(&[t1, t2], &update);
        assert!(
            results.iter().all(|r| r.ok),
            "all should succeed: {results:?}"
        );

        // Per-track titles preserved.
        assert_eq!(read_title(&env, t1), "Song Alpha");
        assert_eq!(read_title(&env, t2), "Song Beta");

        // Album-level fields applied (verified via the DB read-back helper).
        let conn = env.pool.get().unwrap();
        let (_p1, u1) = queries::build_track_tag_update_from_db(&conn, t1).unwrap();
        assert_eq!(u1.album_title.as_deref(), Some("Shared Album"));
        assert_eq!(u1.album_artist_name.as_deref(), Some("Shared Album Artist"));
        assert_eq!(u1.year, Some(2021));
        assert_eq!(u1.genre_names, vec!["Ambient".to_string()]);
        assert_eq!(u1.title, "Song Alpha");
    }
}
