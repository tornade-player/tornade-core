// Service layer event types for UI subscription

use crate::models::{Track, Playlist};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub total_files: u32,
    pub processed_files: u32,
    pub current_file: Option<std::path::PathBuf>,
    pub tracks_added: u32,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub tracks_added: u32,
    pub tracks_updated: u32,
    pub tracks_skipped: u32,
    pub errors: Vec<ScanError>,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub struct ScanError {
    pub path: std::path::PathBuf,
    pub error: String,
}

#[derive(Debug, Clone)]
pub enum LibraryEvent {
    ScanStarted { source_id: i64 },
    ScanProgress { progress: ScanProgress },
    ScanCompleted { result: ScanResult },
    TrackAdded { track: Track },
    TrackUpdated { track: Track },
    TrackDeleted { track_id: i64 },
    AlbumRated { album_id: i64, rating: u8 },
    TrackRated { track_id: i64, rating: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    Off,
    All,
    One,
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    StateChanged { state: PlaybackState },
    TrackChanged { track: Option<Track> },
    PositionChanged { position: Duration },
    QueueChanged { queue: Vec<i64> },
    VolumeChanged { volume: f32 },
    ShuffleChanged { enabled: bool },
    RepeatChanged { mode: RepeatMode },
}

#[derive(Debug, Clone)]
pub enum PlaylistEvent {
    PlaylistCreated { playlist: Playlist },
    PlaylistUpdated { playlist: Playlist },
    PlaylistDeleted { playlist_id: i64 },
}

pub trait EventListener {
    fn on_library_event(&self, event: LibraryEvent);
    fn on_player_event(&self, event: PlayerEvent);
    fn on_playlist_event(&self, event: PlaylistEvent);
}
