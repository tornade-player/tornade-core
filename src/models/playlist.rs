// Playlist and Queue models

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub tracks: Vec<i64>,  // Track IDs in order
    pub created_at: String,  // ISO 8601
    pub updated_at: String,  // ISO 8601
}

#[derive(Debug, Clone)]
pub struct Queue {
    pub tracks: Vec<i64>,        // Track IDs
    pub current_index: usize,
    pub shuffle_enabled: bool,
    pub repeat_mode: RepeatMode,
    pub shuffle_order: Vec<usize>,  // Shuffled indices
}

impl Queue {
    pub fn new() -> Self {
        Queue {
            tracks: Vec::new(),
            current_index: 0,
            shuffle_enabled: false,
            repeat_mode: RepeatMode::Off,
            shuffle_order: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    Off,
    All,
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
