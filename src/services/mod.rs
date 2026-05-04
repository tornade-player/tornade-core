//! Business-logic services that sit between the SQLite database and the UI.
//!
//! Each service is responsible for a single concern:
//!
//! | Service | Responsibility |
//! |---------|---------------|
//! | [`LibraryService`] | Scanning directories, indexing audio files |
//! | [`PlayerService`] | Audio playback via `cpal` (CoreAudio on macOS) |
//! | [`PlaylistService`] | CRUD operations on persistent playlists |
//! | [`SearchService`] | FTS5 + Levenshtein full-text search |
//! | [`ArtworkService`] | Downloading artwork from MusicBrainz / Cover Art Archive |
//! | [`DuplicateService`] | Detecting duplicate tracks by metadata fingerprint |
//! | [`MetadataService`] | Reading audio tags with `lofty` |
//!
//! Services are designed to be cheap to clone (they wrap an `r2d2` pool or
//! `Arc`-backed state) and are safe to share across threads.

pub mod artwork;
pub mod audio_engine;
pub mod duplicate;
pub mod error;
pub mod events;
pub mod library;
pub mod metadata;
pub mod metadata_scrape;
pub mod player;
pub mod playlist;
pub mod reports;
pub mod search;
pub mod tag_writer;

pub use artwork::{ArtworkFetchProgress, ArtworkService};
pub use duplicate::{DuplicateGroup, DuplicateService};
pub use error::{LibraryError, PlayerError, PlaylistError};
pub use events::{
    EventListener, LibraryEvent, PlaybackState, PlayerEvent, PlaylistEvent, ScanError,
    ScanProgress, ScanResult,
};
pub use library::LibraryService;
pub use metadata::{MetadataService, TrackMetadata};
pub use metadata_scrape::{MetadataScrapeService, ScrapeCandidate};
pub use player::PlayerService;
pub use playlist::PlaylistService;
pub use reports::{ArtworkReport, ScanReport};
pub use search::{SearchResults, SearchService};
pub use tag_writer::{TagWriterService, TrackTagUpdate};
// RepeatMode is re-exported from crate::models
