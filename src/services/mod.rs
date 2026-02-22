// Business logic and services

pub mod artwork;
pub mod duplicate;
pub mod error;
pub mod events;
pub mod library;
pub mod metadata;
pub mod player;
pub mod playlist;
pub mod reports;
pub mod search;

pub use artwork::{ArtworkFetchProgress, ArtworkService};
pub use duplicate::{DuplicateGroup, DuplicateService};
pub use error::{LibraryError, PlayerError, PlaylistError};
pub use events::{
    EventListener, LibraryEvent, PlaybackState, PlayerEvent, PlaylistEvent, ScanError,
    ScanProgress, ScanResult,
};
pub use library::LibraryService;
pub use metadata::{MetadataService, TrackMetadata};
pub use player::PlayerService;
pub use playlist::PlaylistService;
pub use reports::{ArtworkReport, ScanReport};
pub use search::{SearchResults, SearchService};
// RepeatMode is re-exported from crate::models
