// Metadata extraction service using lofty

use crate::services::error::LibraryError;
use crate::utils::AppPaths;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct TrackMetadata {
    pub title: String,
    pub artist: String,
    /// The ALBUMARTIST tag, if present. Used to group tracks into albums
    /// regardless of per-track featured artists.
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<u16>,
    pub duration: Duration,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub has_artwork: bool,
}

#[derive(Clone)]
pub struct MetadataService {
    app_paths: AppPaths,
}

impl MetadataService {
    pub fn new(app_paths: AppPaths) -> Self {
        MetadataService { app_paths }
    }

    /// Read metadata from an audio file
    pub fn read_metadata(&self, path: &Path) -> Result<TrackMetadata, LibraryError> {
        let tagged_file = Probe::open(path)
            .map_err(|e| LibraryError::Metadata(format!("Failed to open file: {e}")))?
            .read()
            .map_err(|e| LibraryError::Metadata(format!("Failed to read file: {e}")))?;

        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());

        let properties = tagged_file.properties();

        let default_title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string();

        // Extract basic metadata, falling back to defaults when no tags are present
        let (
            title,
            artist,
            album_artist,
            album,
            genre,
            track_number,
            disc_number,
            year,
            has_artwork,
        ) = if let Some(tag) = tag {
            let title = tag.title().map_or(default_title, |s| s.to_string());
            let artist = tag
                .artist()
                .map_or_else(|| "Unknown Artist".to_string(), |s| s.to_string());
            let album_artist = tag
                .get_string(&ItemKey::AlbumArtist)
                .map(std::string::ToString::to_string);
            let album = tag.album().map(|s| s.to_string());
            let genre = tag.genre().map(|s| s.to_string());
            let track_number = tag.track();
            let disc_number = tag.disk();
            let year = tag.year().and_then(|y| y.try_into().ok());
            let has_artwork = !tag.pictures().is_empty();
            (
                title,
                artist,
                album_artist,
                album,
                genre,
                track_number,
                disc_number,
                year,
                has_artwork,
            )
        } else {
            (
                default_title,
                "Unknown Artist".to_string(),
                None,
                None,
                None,
                None,
                None,
                None,
                false,
            )
        };

        let duration = properties.duration();
        let sample_rate = properties.sample_rate();
        let bit_depth = properties.bit_depth();

        Ok(TrackMetadata {
            title,
            artist,
            album_artist,
            album,
            genre,
            track_number,
            disc_number,
            year,
            duration,
            sample_rate,
            bit_depth,
            has_artwork,
        })
    }

    /// Extract and cache album artwork
    pub fn extract_artwork(
        &self,
        path: &Path,
        artwork_hash: &str,
    ) -> Result<PathBuf, LibraryError> {
        let cache_dir = self.app_paths.artwork_cache_dir();
        let cached_path = cache_dir.join(format!("{artwork_hash}.jpg"));

        // Return cached path if it exists
        if cached_path.exists() {
            return Ok(cached_path);
        }

        // Read file and extract artwork
        let tagged_file = Probe::open(path)
            .map_err(|e| LibraryError::Metadata(format!("Failed to open file: {e}")))?
            .read()
            .map_err(|e| LibraryError::Metadata(format!("Failed to read file: {e}")))?;

        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
            .ok_or_else(|| LibraryError::Metadata("No tags found".to_string()))?;

        // Find front cover or any picture
        let picture = tag
            .pictures()
            .iter()
            .find(|p| p.pic_type() == PictureType::CoverFront)
            .or_else(|| tag.pictures().first())
            .ok_or_else(|| LibraryError::Metadata("No artwork found".to_string()))?;

        // Save artwork to cache
        fs::write(&cached_path, picture.data()).map_err(LibraryError::Io)?;

        Ok(cached_path)
    }

    /// Generate hash for artwork (simple implementation using file path + size)
    pub fn generate_artwork_hash(path: &Path) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        path.to_string_lossy().hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// Get file extension/format
    pub fn get_file_format(path: &Path) -> Option<String> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_lowercase)
    }

    /// Generate thumbnail from artwork (T131)
    /// Creates a smaller version of artwork for faster loading in lists
    pub fn generate_thumbnail(
        &self,
        source_path: &Path,
        artwork_hash: &str,
        size: u32,
    ) -> Result<PathBuf, LibraryError> {
        let cache_dir = self.app_paths.artwork_cache_dir();
        let thumbnail_path = cache_dir.join(format!("{artwork_hash}_{size}x{size}.jpg"));

        // Return cached thumbnail if it exists
        if thumbnail_path.exists() {
            return Ok(thumbnail_path);
        }

        // First extract full artwork if needed
        let artwork_path = self.extract_artwork(source_path, artwork_hash)?;

        // Load and resize image
        let img = image::open(&artwork_path)
            .map_err(|e| LibraryError::Metadata(format!("Failed to load image: {e}")))?;

        // Create thumbnail using a fast filter
        let thumbnail = img.thumbnail(size, size);

        // Save thumbnail
        thumbnail
            .save(&thumbnail_path)
            .map_err(|e| LibraryError::Metadata(format!("Failed to save thumbnail: {e}")))?;

        Ok(thumbnail_path)
    }

    /// Generate multiple thumbnail sizes at once (T131)
    /// Commonly used sizes: 64x64 (list), 256x256 (grid), 512x512 (detail)
    pub fn generate_thumbnails(
        &self,
        source_path: &Path,
        artwork_hash: &str,
        sizes: &[u32],
    ) -> Result<Vec<PathBuf>, LibraryError> {
        let mut thumbnails = Vec::new();

        for &size in sizes {
            match self.generate_thumbnail(source_path, artwork_hash, size) {
                Ok(path) => thumbnails.push(path),
                Err(e) => {
                    // Log but continue with other sizes
                    log::warn!("Failed to generate {size}x{size} thumbnail: {e}");
                }
            }
        }

        if thumbnails.is_empty() {
            return Err(LibraryError::Metadata(
                "Failed to generate any thumbnails".to_string(),
            ));
        }

        Ok(thumbnails)
    }

    /// Get or generate cached thumbnail (T131)
    pub fn get_thumbnail(
        &self,
        source_path: &Path,
        artwork_hash: &str,
        size: u32,
    ) -> Option<PathBuf> {
        let cache_dir = self.app_paths.artwork_cache_dir();
        let thumbnail_path = cache_dir.join(format!("{artwork_hash}_{size}x{size}.jpg"));

        if thumbnail_path.exists() {
            Some(thumbnail_path)
        } else {
            // Try to generate it
            self.generate_thumbnail(source_path, artwork_hash, size)
                .ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TestEnv;

    const MINIMAL_FLAC: &[u8] = include_bytes!("../../tests/fixtures/minimal.flac");

    // Minimal FLAC with STREAMINFO only — no VORBIS_COMMENT block.
    // Used to exercise the "no tags" fallback path in read_metadata.
    const NO_TAGS_FLAC: &[u8] = include_bytes!("../../tests/fixtures/no-tags.flac");

    fn make_service() -> (TestEnv, MetadataService) {
        let env = TestEnv::new();
        let svc = MetadataService::new(env.app_paths.clone());
        (env, svc)
    }

    #[test]
    fn test_file_format() {
        let path = Path::new("/music/test.flac");
        assert_eq!(
            MetadataService::get_file_format(path),
            Some("flac".to_string())
        );
    }

    #[test]
    fn test_file_format_case_insensitive() {
        assert_eq!(
            MetadataService::get_file_format(Path::new("/x.FLAC")),
            Some("flac".to_string())
        );
        assert_eq!(
            MetadataService::get_file_format(Path::new("/x.MP3")),
            Some("mp3".to_string())
        );
    }

    #[test]
    fn test_file_format_no_extension_returns_none() {
        let path = Path::new("/music/tracknoextension");
        assert_eq!(MetadataService::get_file_format(path), None);
    }

    #[test]
    fn test_artwork_hash() {
        let path = Path::new("/music/album/track.flac");
        let hash = MetadataService::generate_artwork_hash(path);
        assert!(!hash.is_empty());
    }

    #[test]
    fn test_artwork_hash_is_deterministic() {
        let path = Path::new("/music/album/track.flac");
        let hash1 = MetadataService::generate_artwork_hash(path);
        let hash2 = MetadataService::generate_artwork_hash(path);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_artwork_hash_differs_for_different_paths() {
        let h1 = MetadataService::generate_artwork_hash(Path::new("/music/a/track.flac"));
        let h2 = MetadataService::generate_artwork_hash(Path::new("/music/b/track.flac"));
        assert_ne!(h1, h2);
    }

    // ── read_metadata ─────────────────────────────────────────────────────────

    #[test]
    fn test_read_metadata_from_minimal_flac() {
        let tmp = tempfile::TempDir::new().unwrap();
        let flac_path = tmp.path().join("test.flac");
        std::fs::write(&flac_path, MINIMAL_FLAC).unwrap();

        let (_env, svc) = make_service();
        let meta = svc.read_metadata(&flac_path).unwrap();

        assert!(!meta.title.is_empty(), "title must be non-empty");
        assert!(!meta.artist.is_empty(), "artist must be non-empty");
        assert!(meta.duration.as_millis() > 0, "duration must be positive");
    }

    #[test]
    fn test_read_metadata_nonexistent_file_returns_error() {
        let (_env, svc) = make_service();
        let result = svc.read_metadata(Path::new("/does/not/exist.flac"));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_metadata_corrupted_file_returns_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bad_path = tmp.path().join("bad.flac");
        std::fs::write(&bad_path, b"NOT A VALID FLAC FILE").unwrap();

        let (_env, svc) = make_service();
        let result = svc.read_metadata(&bad_path);
        assert!(result.is_err(), "corrupted file must return an error");
    }

    // ── read_metadata — no-tags fallback ──────────────────────────────────────

    #[test]
    fn test_read_metadata_no_tags_title_falls_back_to_file_stem() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("my_song.flac");
        std::fs::write(&path, NO_TAGS_FLAC).unwrap();

        let (_env, svc) = make_service();
        let meta = svc.read_metadata(&path).unwrap();

        assert_eq!(
            meta.title, "my_song",
            "title must fall back to file stem when no tags"
        );
    }

    #[test]
    fn test_read_metadata_no_tags_artist_falls_back_to_unknown() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("track.flac");
        std::fs::write(&path, NO_TAGS_FLAC).unwrap();

        let (_env, svc) = make_service();
        let meta = svc.read_metadata(&path).unwrap();

        assert_eq!(meta.artist, "Unknown Artist");
    }

    #[test]
    fn test_read_metadata_no_tags_optional_fields_are_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("track.flac");
        std::fs::write(&path, NO_TAGS_FLAC).unwrap();

        let (_env, svc) = make_service();
        let meta = svc.read_metadata(&path).unwrap();

        assert!(meta.album.is_none(), "album must be None when no tags");
        assert!(
            meta.album_artist.is_none(),
            "album_artist must be None when no tags"
        );
        assert!(meta.genre.is_none(), "genre must be None when no tags");
        assert!(meta.track_number.is_none());
        assert!(meta.disc_number.is_none());
        assert!(meta.year.is_none());
        assert!(!meta.has_artwork);
    }

    #[test]
    fn test_read_metadata_tagged_file_uses_embedded_values() {
        // minimal.flac contains TITLE=Test Track, ARTIST=Test Artist, ALBUM=Test Album
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("tagged.flac");
        std::fs::write(&path, MINIMAL_FLAC).unwrap();

        let (_env, svc) = make_service();
        let meta = svc.read_metadata(&path).unwrap();

        assert_eq!(meta.title, "Test Track");
        assert_eq!(meta.artist, "Test Artist");
        assert_eq!(meta.album.as_deref(), Some("Test Album"));
    }

    // ── get_thumbnail ─────────────────────────────────────────────────────────

    #[test]
    fn test_get_thumbnail_returns_none_when_no_artwork_exists() {
        let tmp = tempfile::TempDir::new().unwrap();
        let flac_path = tmp.path().join("noart.flac");
        std::fs::write(&flac_path, MINIMAL_FLAC).unwrap();

        let (_env, svc) = make_service();
        let hash = MetadataService::generate_artwork_hash(&flac_path);
        // minimal.flac has no embedded artwork, so thumbnail generation must fail silently
        let result = svc.get_thumbnail(&flac_path, &hash, 64);
        assert!(result.is_none());
    }
}
