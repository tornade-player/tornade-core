// Business logic and services

pub mod error;
pub mod events;
pub mod metadata;
pub mod library;
pub mod player;
pub mod playlist;
pub mod duplicate;
pub mod search;

pub use error::{LibraryError, PlayerError, PlaylistError};
pub use events::{
    LibraryEvent, PlayerEvent, PlaylistEvent,
    PlaybackState, RepeatMode, ScanProgress, ScanResult, ScanError,
    EventListener,
};
pub use metadata::{MetadataService, TrackMetadata};
pub use library::LibraryService;
pub use player::PlayerService;
pub use playlist::PlaylistService;
pub use duplicate::{DuplicateService, DuplicateGroup};
pub use search::{SearchService, SearchResults};
