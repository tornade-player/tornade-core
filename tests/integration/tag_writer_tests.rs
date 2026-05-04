//! Integration tests for [`TagWriterService`].
//!
//! Each test copies the shared `tests/fixtures/minimal.flac` fixture into a
//! temporary directory so the original is never mutated.

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::Accessor;
use tornade_core::services::{TagWriterService, TrackTagUpdate};

/// Absolute path to `tests/fixtures/minimal.flac` resolved relative to this
/// source file at compile time, so the test works regardless of the cwd.
fn fixture_path() -> PathBuf {
    // CARGO_MANIFEST_DIR is set by Cargo at compile time and always points to
    // the crate root (where Cargo.toml lives).
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join("tests/fixtures/minimal.flac")
}

/// Copy the fixture to a temp directory and return the (TempDir, file path).
/// The `TempDir` must be kept alive for the duration of the test.
fn copy_fixture_to_temp() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("failed to create temp dir");
    let src = fixture_path();
    let dst = dir.path().join("test.flac");
    fs::copy(&src, &dst).expect("failed to copy fixture");
    (dir, dst)
}

// ---------------------------------------------------------------------------
// Test 1: write track-level tags and read them back
// ---------------------------------------------------------------------------

#[test]
fn test_write_and_read_back_track_tags() {
    let src = fixture_path();
    if !src.exists() {
        // Fixture absent in this checkout — skip gracefully.
        eprintln!("SKIP: fixture not found at {src:?}");
        return;
    }

    let (_tmp, path) = copy_fixture_to_temp();

    let service = TagWriterService::new();
    let update = TrackTagUpdate {
        title: "Integration Test Title".to_string(),
        artist_name: "Test Artist".to_string(),
        album_title: Some("Test Album".to_string()),
        album_artist_name: Some("Test Album Artist".to_string()),
        year: Some(2024),
        genre_names: vec!["Electronic".to_string()],
        track_number: Some(3),
        disc_number: Some(1),
    };

    service
        .write_track_tags(&path, &update)
        .expect("write_track_tags should succeed");

    // Read back using lofty directly.
    let tagged_file = Probe::open(&path)
        .expect("probe open")
        .read()
        .expect("probe read");

    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())
        .expect("tag must exist after write");

    assert_eq!(tag.title().as_deref(), Some("Integration Test Title"));
    assert_eq!(tag.artist().as_deref(), Some("Test Artist"));
    assert_eq!(tag.album().as_deref(), Some("Test Album"));
    assert_eq!(tag.year(), Some(2024));
    assert_eq!(tag.genre().as_deref(), Some("Electronic"));
    assert_eq!(tag.track(), Some(3));
    assert_eq!(tag.disk(), Some(1));
}

// ---------------------------------------------------------------------------
// Test 2: write_album_level_tags only touches ALBUM and ALBUMARTIST
// ---------------------------------------------------------------------------

#[test]
fn test_write_album_level_tags_only() {
    let src = fixture_path();
    if !src.exists() {
        eprintln!("SKIP: fixture not found at {src:?}");
        return;
    }

    let (_tmp, path) = copy_fixture_to_temp();

    let service = TagWriterService::new();

    // First, set up known track-level tags so we can assert they are unchanged.
    let setup = TrackTagUpdate {
        title: "Original Title".to_string(),
        artist_name: "Original Artist".to_string(),
        album_title: Some("Original Album".to_string()),
        album_artist_name: Some("Original Album Artist".to_string()),
        year: Some(2020),
        genre_names: vec![],
        track_number: Some(1),
        disc_number: Some(1),
    };
    service
        .write_track_tags(&path, &setup)
        .expect("setup write should succeed");

    // Now perform an album-level-only update.
    service
        .write_album_level_tags(&path, "New Album Name", "New Album Artist")
        .expect("write_album_level_tags should succeed");

    // Read back and check.
    let tagged_file = Probe::open(&path)
        .expect("probe open")
        .read()
        .expect("probe read");

    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())
        .expect("tag must exist");

    // Album-level fields must be updated.
    assert_eq!(tag.album().as_deref(), Some("New Album Name"));

    // Track-level fields must remain from the original write.
    assert_eq!(tag.title().as_deref(), Some("Original Title"));
    assert_eq!(tag.artist().as_deref(), Some("Original Artist"));
    assert_eq!(tag.track(), Some(1));
    assert_eq!(tag.disk(), Some(1));
}

// ---------------------------------------------------------------------------
// Test 3: error on non-existent path
// ---------------------------------------------------------------------------

#[test]
fn test_write_tags_to_nonexistent_file_returns_error() {
    let service = TagWriterService::new();
    let update = TrackTagUpdate {
        title: "T".to_string(),
        artist_name: "A".to_string(),
        album_title: None,
        album_artist_name: None,
        year: None,
        genre_names: vec![],
        track_number: None,
        disc_number: None,
    };

    let result = service.write_track_tags(
        std::path::Path::new("/nonexistent/path/track.flac"),
        &update,
    );

    assert!(result.is_err(), "expected error for nonexistent file");
}
