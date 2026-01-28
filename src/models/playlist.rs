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
pub enum RepeatMode {
    Off,
    All,
    One,
}
