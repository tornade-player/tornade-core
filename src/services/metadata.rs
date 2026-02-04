// Metadata extraction service using lofty

use crate::services::error::LibraryError;
use crate::utils::AppPaths;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::Accessor;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct TrackMetadata {
    pub title: String,
    pub artist: String,
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
            .map_err(|e| LibraryError::Metadata(format!("Failed to open file: {}", e)))?
            .read()
            .map_err(|e| LibraryError::Metadata(format!("Failed to read file: {}", e)))?;

        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
            .ok_or_else(|| LibraryError::Metadata("No tags found".to_string()))?;

        let properties = tagged_file.properties();

        // Extract basic metadata
        let title = tag
            .title()
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string()
            });

        let artist = tag
            .artist()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Unknown Artist".to_string());

        let album = tag.album().map(|s| s.to_string());
        let genre = tag.genre().map(|s| s.to_string());
        let track_number = tag.track().and_then(|n| n.try_into().ok());
        let disc_number = tag.disk().and_then(|n| n.try_into().ok());
        let year = tag.year().and_then(|y| y.try_into().ok());

        let duration = properties.duration();
        let sample_rate = properties.sample_rate();
        let bit_depth = properties.bit_depth();

        let has_artwork = tag.pictures().len() > 0;

        Ok(TrackMetadata {
            title,
            artist,
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
    pub fn extract_artwork(&self, path: &Path, artwork_hash: &str) -> Result<PathBuf, LibraryError> {
        let cache_dir = self.app_paths.artwork_cache_dir();
        let cached_path = cache_dir.join(format!("{}.jpg", artwork_hash));

        // Return cached path if it exists
        if cached_path.exists() {
            return Ok(cached_path);
        }

        // Read file and extract artwork
        let tagged_file = Probe::open(path)
            .map_err(|e| LibraryError::Metadata(format!("Failed to open file: {}", e)))?
            .read()
            .map_err(|e| LibraryError::Metadata(format!("Failed to read file: {}", e)))?;

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
        fs::write(&cached_path, picture.data())
            .map_err(|e| LibraryError::Io(e))?;

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
            .map(|s| s.to_lowercase())
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
        let thumbnail_path = cache_dir.join(format!("{}_{}x{}.jpg", artwork_hash, size, size));

        // Return cached thumbnail if it exists
        if thumbnail_path.exists() {
            return Ok(thumbnail_path);
        }

        // First extract full artwork if needed
        let artwork_path = self.extract_artwork(source_path, artwork_hash)?;

        // Load and resize image
        let img = image::open(&artwork_path)
            .map_err(|e| LibraryError::Metadata(format!("Failed to load image: {}", e)))?;

        // Create thumbnail using a fast filter
        let thumbnail = img.thumbnail(size, size);

        // Save thumbnail
        thumbnail
            .save(&thumbnail_path)
            .map_err(|e| LibraryError::Metadata(format!("Failed to save thumbnail: {}", e)))?;

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
                    log::warn!("Failed to generate {}x{} thumbnail: {}", size, size, e);
                }
            }
        }

        if thumbnails.is_empty() {
            return Err(LibraryError::Metadata(
                "Failed to generate any thumbnails".to_string()
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
        let thumbnail_path = cache_dir.join(format!("{}_{}x{}.jpg", artwork_hash, size, size));

        if thumbnail_path.exists() {
            Some(thumbnail_path)
        } else {
            // Try to generate it
            self.generate_thumbnail(source_path, artwork_hash, size).ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_format() {
        let path = Path::new("/music/test.flac");
        assert_eq!(MetadataService::get_file_format(path), Some("flac".to_string()));
    }

    #[test]
    fn test_artwork_hash() {
        let path = Path::new("/music/album/track.flac");
        let hash = MetadataService::generate_artwork_hash(path);
        assert!(!hash.is_empty());
    }
}
