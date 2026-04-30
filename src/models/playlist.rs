//! Playlist and playback-queue domain models.

use serde::{Deserialize, Serialize};

/// A track entry within a playlist, carrying its position timestamp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistTrack {
    /// Foreign key into the `tracks` table.
    pub track_id: i64,
    /// When this track was added to the playlist (ISO 8601, UTC).
    pub added_at: String,
}

/// A named, ordered collection of tracks saved by the user.
///
/// Playlists are persisted in the `playlists` / `playlist_tracks` tables and
/// managed by [`crate::services::PlaylistService`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    /// SQLite primary key (`playlists.id`).
    pub id: i64,
    /// User-visible playlist name.
    pub name: String,
    /// Optional free-text description.
    pub description: Option<String>,
    /// Ordered list of playlist-track entries (track ID + added_at timestamp).
    pub tracks: Vec<PlaylistTrack>,
    /// Creation timestamp (ISO 8601, UTC).
    pub created_at: String,
    /// Last modification timestamp (ISO 8601, UTC).
    pub updated_at: String,
}

/// The transient playback queue managed by [`crate::services::PlayerService`].
///
/// Unlike a [`Playlist`], the queue is not persisted — it exists only for the
/// lifetime of the current playback session. Shuffle support is implemented by
/// maintaining a pre-computed `shuffle_order` index alongside the original track
/// order so that both can be traversed without modifying `tracks`.
#[derive(Debug, Clone, Default)]
pub struct Queue {
    /// Ordered track IDs in the queue (original, non-shuffled order).
    pub tracks: Vec<i64>,
    /// Index into either `tracks` (shuffle off) or `shuffle_order` (shuffle on).
    pub current_index: usize,
    /// Whether shuffle mode is active.
    pub shuffle_enabled: bool,
    /// Current repeat behaviour.
    pub repeat_mode: RepeatMode,
    /// Pre-computed permutation of `0..tracks.len()` used when `shuffle_enabled` is true.
    pub shuffle_order: Vec<usize>,
}

impl Queue {
    /// Create an empty queue with shuffle off and repeat mode set to [`RepeatMode::Off`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if the queue contains no tracks.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Returns the number of tracks in the queue.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Returns the track ID of the currently active position, respecting shuffle order.
    ///
    /// Returns `None` when the queue is empty or `current_index` is out of bounds.
    pub fn current_track(&self) -> Option<i64> {
        if self.is_empty() {
            return None;
        }

        let index = if self.shuffle_enabled {
            self.shuffle_order.get(self.current_index).copied()?
        } else {
            self.current_index
        };

        self.tracks.get(index).copied()
    }
}

/// Controls what happens when the queue reaches the last track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    /// Playback stops after the last track.
    #[default]
    Off,
    /// The entire queue restarts from the beginning after the last track.
    All,
    /// The current track repeats indefinitely.
    One,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_new_empty() {
        let queue = Queue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.current_track(), None);
        assert!(!queue.shuffle_enabled);
        assert_eq!(queue.repeat_mode, RepeatMode::Off);
    }

    #[test]
    fn test_queue_current_track() {
        let mut queue = Queue::new();
        queue.tracks = vec![10, 20, 30];
        queue.current_index = 1;

        assert_eq!(queue.current_track(), Some(20));
        assert_eq!(queue.len(), 3);
        assert!(!queue.is_empty());
    }

    #[test]
    fn test_queue_current_track_with_shuffle() {
        let mut queue = Queue::new();
        queue.tracks = vec![10, 20, 30];
        queue.shuffle_enabled = true;
        queue.shuffle_order = vec![2, 0, 1]; // shuffled order
        queue.current_index = 0;

        // current_index=0 -> shuffle_order[0]=2 -> tracks[2]=30
        assert_eq!(queue.current_track(), Some(30));
    }

    #[test]
    fn test_queue_current_track_out_of_bounds() {
        let mut queue = Queue::new();
        queue.tracks = vec![10, 20];
        queue.current_index = 5; // out of bounds

        assert_eq!(queue.current_track(), None);
    }
}
