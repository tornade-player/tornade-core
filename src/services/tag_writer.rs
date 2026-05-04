//! Audio tag writing service using the `lofty` crate.

use crate::services::error::LibraryError;
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, ItemValue, TagExt, TagItem};
use std::path::Path;

/// Fields that can be updated on a single track.
///
/// Passed to [`TagWriterService::write_track_tags`] to atomically overwrite
/// all editable tag fields in one lofty write operation.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TrackTagUpdate {
    pub title: String,
    pub artist_name: String,
    pub album_title: Option<String>,
    pub album_artist_name: Option<String>,
    pub year: Option<u16>,
    pub genre_names: Vec<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
}

/// Writes audio tags back to audio files on disk using [`lofty`].
///
/// The service is stateless and cheap to construct. All methods open,
/// mutate and save the target file in place.
pub struct TagWriterService;

impl TagWriterService {
    /// Create a new `TagWriterService`.
    pub fn new() -> Self {
        TagWriterService
    }

    /// Overwrite all editable track-level tags in an audio file.
    ///
    /// The method reads the current tags, applies every field from `update`,
    /// and persists the result with [`lofty`]'s `save_to_path`.
    ///
    /// # Errors
    ///
    /// Returns [`LibraryError::Metadata`] if the file cannot be opened,
    /// read, or written.
    pub fn write_track_tags(
        &self,
        path: &Path,
        update: &TrackTagUpdate,
    ) -> Result<(), LibraryError> {
        let mut tagged_file = Probe::open(path)
            .map_err(|e| LibraryError::Metadata(format!("Failed to open file: {e}")))?
            .read()
            .map_err(|e| LibraryError::Metadata(format!("Failed to read file: {e}")))?;

        // Borrow primary tag type first (immutable), then borrow mutably below.
        let has_primary = tagged_file.primary_tag().is_some();
        let tag = if has_primary {
            tagged_file.primary_tag_mut()
        } else {
            tagged_file.first_tag_mut()
        }
        .ok_or_else(|| LibraryError::Metadata("No writable tag found in file".to_string()))?;

        // Mandatory fields
        tag.set_title(update.title.clone());
        tag.set_artist(update.artist_name.clone());

        // Optional fields: set when Some, remove when None
        match &update.album_title {
            Some(v) => tag.set_album(v.clone()),
            None => tag.remove_album(),
        }

        match &update.album_artist_name {
            Some(v) => {
                tag.insert(TagItem::new(
                    ItemKey::AlbumArtist,
                    ItemValue::Text(v.clone()),
                ));
            }
            None => {
                tag.retain(|i| i.key() != &ItemKey::AlbumArtist);
            }
        }

        match update.year {
            Some(y) => tag.set_year(u32::from(y)),
            None => tag.remove_year(),
        }

        // First genre only via the standard Accessor; additional genres are
        // intentionally dropped because lofty's Accessor maps to a single GENRE field.
        match update.genre_names.first() {
            Some(g) => tag.set_genre(g.clone()),
            None => tag.remove_genre(),
        }

        match update.track_number {
            Some(n) => tag.set_track(n),
            None => tag.remove_track(),
        }

        match update.disc_number {
            Some(n) => tag.set_disk(n),
            None => tag.remove_disk(),
        }

        tag.save_to_path(path, WriteOptions::default())
            .map_err(|e| LibraryError::Metadata(format!("Failed to save tags: {e}")))?;

        Ok(())
    }

    /// Overwrite ONLY the album-level tags (ALBUM, ALBUMARTIST) in an audio file.
    ///
    /// Track-specific tags (TITLE, ARTIST, TRACKNUMBER, DISCNUMBER, etc.) are
    /// left untouched. This is used when propagating album-level edits to every
    /// track in an album.
    ///
    /// # Errors
    ///
    /// Returns [`LibraryError::Metadata`] if the file cannot be opened,
    /// read, or written.
    pub fn write_album_level_tags(
        &self,
        path: &Path,
        album_title: &str,
        album_artist: &str,
    ) -> Result<(), LibraryError> {
        let mut tagged_file = Probe::open(path)
            .map_err(|e| LibraryError::Metadata(format!("Failed to open file: {e}")))?
            .read()
            .map_err(|e| LibraryError::Metadata(format!("Failed to read file: {e}")))?;

        // Borrow primary tag type first (immutable), then borrow mutably below.
        let has_primary = tagged_file.primary_tag().is_some();
        let tag = if has_primary {
            tagged_file.primary_tag_mut()
        } else {
            tagged_file.first_tag_mut()
        }
        .ok_or_else(|| LibraryError::Metadata("No writable tag found in file".to_string()))?;

        // Only ALBUM and ALBUMARTIST are touched here.
        tag.set_album(album_title.to_owned());
        tag.insert(TagItem::new(
            ItemKey::AlbumArtist,
            ItemValue::Text(album_artist.to_owned()),
        ));

        tag.save_to_path(path, WriteOptions::default())
            .map_err(|e| LibraryError::Metadata(format!("Failed to save tags: {e}")))?;

        Ok(())
    }
}

impl Default for TagWriterService {
    fn default() -> Self {
        Self::new()
    }
}
