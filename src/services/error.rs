// Service layer error types

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Metadata error: {0}")]
    Metadata(String),

    #[error("Track not found: {0}")]
    TrackNotFound(i64),

    #[error("Album not found: {0}")]
    AlbumNotFound(i64),

    #[error("Source not found: {0}")]
    SourceNotFound(i64),

    #[error("Invalid rating: {0} (must be 0-5)")]
    InvalidRating(u8),

    #[error("Scan cancelled")]
    ScanCancelled,
}

#[derive(Debug, Error)]
pub enum PlayerError {
    #[error("Audio error: {0}")]
    Audio(String),

    #[error("Track not found: {0}")]
    TrackNotFound(i64),

    #[error("Empty queue")]
    EmptyQueue,

    #[error("Invalid position")]
    InvalidPosition,
}

#[derive(Debug, Error)]
pub enum PlaylistError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Playlist not found: {0}")]
    PlaylistNotFound(i64),

    #[error("Invalid M3U file: {0}")]
    InvalidM3u(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
