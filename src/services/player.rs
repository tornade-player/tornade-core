// Audio playback service using rodio

use crate::db::DbPool;
use crate::models::{Track, Queue, RepeatMode};
use crate::services::error::PlayerError;
use crate::utils::app_state::{self, PersistedState};
use crate::services::events::PlaybackState;
use log::{info, warn};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::fs::File;
use std::io::BufReader;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, PlayerError>;

pub struct PlayerService {
    pool: DbPool,
    state: Arc<Mutex<PlayerState>>,
    audio: Arc<Mutex<Option<(OutputStream, OutputStreamHandle)>>>,
    sink: Arc<Mutex<Option<Sink>>>,
}

struct PlayerState {
    current_track: Option<Track>,
    queue: Queue,
    playback_state: PlaybackState,
    volume: f32,
    playback_start_time: Option<Instant>,
    paused_at: Option<Duration>,
    skipped_track_ids: HashSet<i64>,
}

impl PlayerService {
    pub fn new(pool: DbPool) -> Result<Self> {
        // Restore queue state from previous session
        let persisted = app_state::load_state(&pool).unwrap_or_default();

        let queue = Queue {
            tracks: persisted.queue,
            current_index: persisted.queue_index,
            shuffle_enabled: persisted.shuffle_enabled,
            repeat_mode: persisted.repeat_mode,
            shuffle_order: persisted.shuffle_order,
        };

        info!("Restored queue with {} tracks (index {})", queue.tracks.len(), queue.current_index);

        Ok(PlayerService {
            pool,
            state: Arc::new(Mutex::new(PlayerState {
                current_track: None,
                queue,
                playback_state: PlaybackState::Stopped,
                volume: persisted.volume,
                playback_start_time: None,
                paused_at: None,
                skipped_track_ids: HashSet::new(),
            })),
            audio: Arc::new(Mutex::new(None)),
            sink: Arc::new(Mutex::new(None)),
        })
    }

    /// Initialize audio stream (lazy initialization)
    fn ensure_audio_stream(&self) -> Result<OutputStreamHandle> {
        let mut audio = self.audio.lock().unwrap();

        if audio.is_none() {
            let (stream, handle) = OutputStream::try_default()
                .map_err(|e| PlayerError::Audio(format!("Failed to create audio stream: {}", e)))?;
            *audio = Some((stream, handle.clone()));
            Ok(handle)
        } else {
            Ok(audio.as_ref().unwrap().1.clone())
        }
    }

    // ========================================================================
    // Playback Control
    // ========================================================================

    /// Start playing a track
    pub fn play(&self, track_id: i64) -> Result<()> {
        info!("Playing track: {}", track_id);

        // Ensure audio stream is initialized
        let stream_handle = self.ensure_audio_stream()?;

        // Get track from database
        let conn = self.pool.get().map_err(|e| {
            PlayerError::Audio(format!("Database connection error: {}", e))
        })?;

        let track = crate::db::queries::get_track(&conn, track_id)
            .map_err(|e| PlayerError::Audio(format!("Database error: {}", e)))?
            .ok_or(PlayerError::TrackNotFound(track_id))?;

        // Create new sink
        let sink = Sink::try_new(&stream_handle)
            .map_err(|e| PlayerError::Audio(format!("Failed to create sink: {}", e)))?;

        // Open and decode file
        let file = File::open(&track.file_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PlayerError::FileNotFound(track.file_path.to_string_lossy().into_owned())
            } else {
                PlayerError::Audio(format!("Failed to open file: {}", e))
            }
        })?;

        let source = Decoder::new(BufReader::new(file))
            .map_err(|e| PlayerError::Audio(format!("Failed to decode audio: {}", e)))?;

        // Set volume
        let volume = self.state.lock().unwrap().volume;
        sink.set_volume(volume);

        // Play
        sink.append(source);
        sink.play();

        // Update state
        {
            let mut state = self.state.lock().unwrap();
            state.current_track = Some(track);
            state.playback_state = PlaybackState::Playing;
            state.playback_start_time = Some(Instant::now());
            state.paused_at = None;

            // Update current_index to match the track being played
            if let Some(position) = state.queue.tracks.iter().position(|&id| id == track_id) {
                // If shuffle is enabled, find position in shuffle_order
                if state.queue.shuffle_enabled {
                    // Find the index in shuffle_order that points to this track position
                    if let Some(shuffle_idx) = state.queue.shuffle_order.iter().position(|&idx| idx == position) {
                        state.queue.current_index = shuffle_idx;
                    }
                } else {
                    state.queue.current_index = position;
                }
            }
        }

        // Store sink
        {
            let mut sink_lock = self.sink.lock().unwrap();
            *sink_lock = Some(sink);
        }

        Ok(())
    }

    /// Pause playback
    pub fn pause(&self) -> Result<()> {
        let sink_lock = self.sink.lock().unwrap();
        if let Some(ref sink) = *sink_lock {
            sink.pause();

            // Save current position when pausing
            let mut state = self.state.lock().unwrap();
            if let Some(start_time) = state.playback_start_time {
                state.paused_at = Some(start_time.elapsed());
            }
            state.playback_state = PlaybackState::Paused;

            info!("Playback paused");
            Ok(())
        } else {
            Err(PlayerError::EmptyQueue)
        }
    }

    /// Resume playback
    pub fn resume(&self) -> Result<()> {
        let sink_lock = self.sink.lock().unwrap();
        if let Some(ref sink) = *sink_lock {
            sink.play();

            // Adjust start time to account for paused duration
            let mut state = self.state.lock().unwrap();
            if let Some(paused_at) = state.paused_at {
                state.playback_start_time = Some(Instant::now() - paused_at);
                state.paused_at = None;
            }
            state.playback_state = PlaybackState::Playing;

            info!("Playback resumed");
            Ok(())
        } else {
            Err(PlayerError::EmptyQueue)
        }
    }

    /// Stop playback
    pub fn stop(&self) -> Result<()> {
        {
            let mut sink_lock = self.sink.lock().unwrap();
            if let Some(sink) = sink_lock.take() {
                sink.stop();
            }
        }

        {
            let mut state = self.state.lock().unwrap();
            state.current_track = None;
            state.playback_state = PlaybackState::Stopped;
            state.playback_start_time = None;
            state.paused_at = None;
        }

        info!("Playback stopped");
        Ok(())
    }

    /// Seek to position in the current track
    ///
    /// Implementation note: Since rodio's Decoder doesn't support seeking,
    /// we simulate it by adjusting the playback start time.
    /// This works well for tracking position but won't skip actual audio data.
    /// For true seeking (skipping to position in file), we'd need a custom
    /// symphonia-based Source with Seek trait implementation.
    pub fn seek(&self, position: Duration) -> Result<()> {
        // Get current track info before locking state
        let (track, was_playing, volume) = {
            let state = self.state.lock().unwrap();

            // Verify we have a current track
            let track = state.current_track.as_ref()
                .ok_or(PlayerError::EmptyQueue)?
                .clone();

            let was_playing = matches!(state.playback_state, PlaybackState::Playing);
            let volume = state.volume;

            (track, was_playing, volume)
        };

        // Validate and clamp position
        let clamped_position = if position > track.duration {
            track.duration
        } else {
            position
        };

        // If not playing, just update the paused position
        if !was_playing {
            let mut state = self.state.lock().unwrap();
            state.paused_at = Some(clamped_position);
            info!("Seeked to {:?} while paused", clamped_position);
            return Ok(());
        }

        // For playing state, we need to restart playback from the new position
        // Get stream handle
        let stream_handle = self.ensure_audio_stream()?;

        // Open and decode file
        let file = File::open(&track.file_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PlayerError::FileNotFound(track.file_path.to_string_lossy().into_owned())
            } else {
                PlayerError::Audio(format!("Failed to open file: {}", e))
            }
        })?;

        let source = Decoder::new(BufReader::new(file))
            .map_err(|e| PlayerError::Audio(format!("Failed to decode audio: {}", e)))?;

        // Skip to the desired position using rodio's skip_duration
        use rodio::Source;
        let source_at_position = source.skip_duration(clamped_position);

        // Create new sink
        let sink = Sink::try_new(&stream_handle)
            .map_err(|e| PlayerError::Audio(format!("Failed to create sink: {}", e)))?;

        sink.set_volume(volume);
        sink.append(source_at_position);
        sink.play();

        // Update state
        {
            let mut state = self.state.lock().unwrap();
            state.playback_start_time = Some(Instant::now() - clamped_position);
            state.playback_state = PlaybackState::Playing;

            // Replace sink
            *self.sink.lock().unwrap() = Some(sink);
        }

        info!("Seeked to {:?}", clamped_position);
        Ok(())
    }

    /// Skip to next track, auto-skipping up to 3 consecutive missing files
    pub fn next(&self) -> Result<()> {
        let mut consecutive_misses = 0;

        loop {
            let track_id = {
                let mut state = self.state.lock().unwrap();

                if state.queue.is_empty() {
                    return Err(PlayerError::EmptyQueue);
                }

                // Advance one step
                state.queue.current_index += 1;

                // Handle end of queue
                if state.queue.current_index >= state.queue.len() {
                    match state.queue.repeat_mode {
                        RepeatMode::All => state.queue.current_index = 0,
                        RepeatMode::One => state.queue.current_index -= 1,
                        RepeatMode::Off => {
                            drop(state);
                            return self.stop();
                        }
                    }
                }

                state.queue.current_track().ok_or(PlayerError::EmptyQueue)?
            };

            match self.play(track_id) {
                Ok(()) => {
                    self.save_queue_state();
                    return Ok(());
                }
                Err(PlayerError::FileNotFound(path)) if consecutive_misses < 3 => {
                    warn!("Track {} file not found in next(), skipping", track_id);
                    self.state.lock().unwrap().skipped_track_ids.insert(track_id);
                    consecutive_misses += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Skip to previous track (or restart if < 3s)
    pub fn previous(&self) -> Result<()> {
        // Check if we should restart current track or go to previous
        let should_restart = {
            let sink_lock = self.sink.lock().unwrap();
            if let Some(ref _sink) = *sink_lock {
                // If we've been playing for less than 3 seconds, go to previous
                // Otherwise restart current track
                // Note: rodio doesn't expose position easily, so we'll just go to previous
                false
            } else {
                false
            }
        };

        if should_restart {
            // Restart current track
            let track_id = self.state.lock().unwrap()
                .current_track
                .as_ref()
                .map(|t| t.id)
                .ok_or(PlayerError::EmptyQueue)?;
            return self.play(track_id);
        }

        let mut state = self.state.lock().unwrap();

        if state.queue.is_empty() {
            return Err(PlayerError::EmptyQueue);
        }

        if state.queue.current_index > 0 {
            state.queue.current_index -= 1;
        } else {
            // At start of queue
            if state.queue.repeat_mode == RepeatMode::All {
                state.queue.current_index = state.queue.len() - 1;
            } else {
                return Err(PlayerError::InvalidPosition);
            }
        }

        let track_id = state.queue.current_track()
            .ok_or(PlayerError::EmptyQueue)?;

        drop(state);

        self.play(track_id)?;
        self.save_queue_state();
        Ok(())
    }

    /// Jump to a specific index in the queue, skipping up to 3 consecutive missing files
    pub fn jump_to_index(&self, index: usize) -> Result<()> {
        let queue_len = {
            let state = self.state.lock().unwrap();
            if state.queue.is_empty() {
                return Err(PlayerError::EmptyQueue);
            }
            state.queue.len()
        };

        if index >= queue_len {
            return Err(PlayerError::InvalidPosition);
        }

        let mut current = index;
        let mut last_missing: Option<String> = None;

        while current < queue_len {
            let track_id = {
                let mut state = self.state.lock().unwrap();
                state.queue.current_index = current;
                state.queue.current_track().ok_or(PlayerError::EmptyQueue)?
            };

            match self.play(track_id) {
                Ok(()) => {
                    self.save_queue_state();
                    return Ok(());
                }
                Err(PlayerError::FileNotFound(path)) => {
                    warn!("Track {} file not found ({}), skipping", track_id, path);
                    self.state.lock().unwrap().skipped_track_ids.insert(track_id);
                    // Stop after 3 consecutive missing files — likely a disconnected volume
                    if current >= index + 3 {
                        return Err(PlayerError::FileNotFound(path));
                    }
                    last_missing = Some(path);
                    current += 1;
                }
                Err(e) => return Err(e),
            }
        }

        Err(PlayerError::FileNotFound(last_missing.unwrap_or_default()))
    }

    // ========================================================================
    // Queue Management
    // ========================================================================

    /// Persist current queue state to the database (best-effort, logs on failure)
    fn save_queue_state(&self) {
        let persisted = {
            let state = self.state.lock().unwrap();
            PersistedState {
                current_track_id: state.current_track.as_ref().map(|t| t.id),
                playback_position: 0.0,
                queue: state.queue.tracks.clone(),
                queue_index: state.queue.current_index,
                shuffle_order: state.queue.shuffle_order.clone(),
                volume: state.volume,
                shuffle_enabled: state.queue.shuffle_enabled,
                repeat_mode: state.queue.repeat_mode,
            }
        };
        if let Err(e) = app_state::save_state(&self.pool, &persisted) {
            warn!("Failed to persist queue state: {}", e);
        }
    }

    /// Set the playback queue
    pub fn set_queue(&self, track_ids: Vec<i64>) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.queue.tracks = track_ids;
        state.queue.current_index = 0;
        state.skipped_track_ids.clear();

        // Regenerate shuffle order if shuffle is enabled
        if state.queue.shuffle_enabled {
            use rand::seq::SliceRandom;
            use rand::thread_rng;
            let mut indices: Vec<usize> = (0..state.queue.len()).collect();
            indices.shuffle(&mut thread_rng());
            state.queue.shuffle_order = indices;
        }

        info!("Queue set with {} tracks", state.queue.tracks.len());
        drop(state);
        self.save_queue_state();
        Ok(())
    }

    /// Add tracks to queue
    pub fn add_to_queue(&self, track_ids: Vec<i64>) -> Result<()> {
        let count = track_ids.len();
        {
            let mut state = self.state.lock().unwrap();
            state.queue.tracks.extend(track_ids);

            // Regenerate shuffle order if shuffle is enabled
            if state.queue.shuffle_enabled {
                use rand::seq::SliceRandom;
                use rand::thread_rng;
                let mut indices: Vec<usize> = (0..state.queue.len()).collect();
                indices.shuffle(&mut thread_rng());
                state.queue.shuffle_order = indices;
            }
        }

        info!("Added {} tracks to queue", count);
        self.save_queue_state();
        Ok(())
    }

    /// Remove track from queue at position
    pub fn remove_from_queue(&self, position: usize) -> Result<()> {
        let mut state = self.state.lock().unwrap();

        if position >= state.queue.tracks.len() {
            return Err(PlayerError::InvalidPosition);
        }

        state.queue.tracks.remove(position);

        // Adjust current index if needed
        if state.queue.current_index >= position && state.queue.current_index > 0 {
            state.queue.current_index -= 1;
        }

        // Regenerate shuffle order if shuffle is enabled
        if state.queue.shuffle_enabled {
            use rand::seq::SliceRandom;
            use rand::thread_rng;
            let mut indices: Vec<usize> = (0..state.queue.len()).collect();
            indices.shuffle(&mut thread_rng());
            state.queue.shuffle_order = indices;
        }

        info!("Removed track at position {}", position);
        drop(state);
        self.save_queue_state();
        Ok(())
    }

    /// Move track in queue
    pub fn move_in_queue(&self, from: usize, to: usize) -> Result<()> {
        {
            let mut state = self.state.lock().unwrap();

            if from >= state.queue.tracks.len() || to >= state.queue.tracks.len() {
                return Err(PlayerError::InvalidPosition);
            }

            let track_id = state.queue.tracks.remove(from);
            state.queue.tracks.insert(to, track_id);

            // Adjust current index if needed
            if state.queue.current_index == from {
                state.queue.current_index = to;
            } else if from < state.queue.current_index && to >= state.queue.current_index {
                state.queue.current_index -= 1;
            } else if from > state.queue.current_index && to <= state.queue.current_index {
                state.queue.current_index += 1;
            }

            // Regenerate shuffle order if shuffle is enabled
            if state.queue.shuffle_enabled {
                use rand::seq::SliceRandom;
                use rand::thread_rng;
                let mut indices: Vec<usize> = (0..state.queue.len()).collect();
                indices.shuffle(&mut thread_rng());
                state.queue.shuffle_order = indices;
            }
        }

        info!("Moved track from position {} to {}", from, to);
        self.save_queue_state();
        Ok(())
    }

    /// Clear queue
    pub fn clear_queue(&self) -> Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            state.queue.tracks.clear();
            state.queue.shuffle_order.clear();
            state.queue.current_index = 0;
            state.skipped_track_ids.clear();
        }
        info!("Queue cleared");
        self.save_queue_state();
        Ok(())
    }

    /// Get current queue
    pub fn get_queue(&self) -> Vec<i64> {
        self.state.lock().unwrap().queue.tracks.clone()
    }

    /// Get current queue index
    pub fn get_queue_index(&self) -> usize {
        self.state.lock().unwrap().queue.current_index
    }

    /// Get track IDs that failed to play due to missing files
    pub fn get_skipped_track_ids(&self) -> Vec<i64> {
        self.state.lock().unwrap().skipped_track_ids.iter().copied().collect()
    }

    // ========================================================================
    // Playback State
    // ========================================================================

    pub fn get_state(&self) -> PlaybackState {
        self.state.lock().unwrap().playback_state
    }

    pub fn get_current_track(&self) -> Option<Track> {
        self.state.lock().unwrap().current_track.clone()
    }

    /// Get current playback position in seconds
    pub fn get_position(&self) -> f64 {
        let state = self.state.lock().unwrap();

        match state.playback_state {
            PlaybackState::Playing => {
                if let Some(start_time) = state.playback_start_time {
                    start_time.elapsed().as_secs_f64()
                } else {
                    0.0
                }
            }
            PlaybackState::Paused => {
                state.paused_at.map(|d| d.as_secs_f64()).unwrap_or(0.0)
            }
            PlaybackState::Stopped => 0.0,
        }
    }

    // ========================================================================
    // Modes
    // ========================================================================

    pub fn set_shuffle(&self, enabled: bool) -> Result<()> {
        let mut state = self.state.lock().unwrap();

        // Remember which track is currently playing by finding its position in the queue
        let current_track_position = if enabled {
            // When enabling shuffle, current_index is an actual track position
            // We need to find where this position will be in the new shuffle_order
            Some(state.queue.current_index)
        } else {
            // When disabling shuffle, current_index is a shuffle_order position
            // We need to convert it to the actual track position
            if !state.queue.shuffle_order.is_empty() && state.queue.current_index < state.queue.shuffle_order.len() {
                Some(state.queue.shuffle_order[state.queue.current_index])
            } else {
                None
            }
        };

        state.queue.shuffle_enabled = enabled;

        if enabled {
            // Generate shuffle order
            use rand::seq::SliceRandom;
            use rand::thread_rng;

            let mut indices: Vec<usize> = (0..state.queue.len()).collect();
            eprintln!("🎲 SHUFFLE: Before shuffle - indices={:?}", indices);
            indices.shuffle(&mut thread_rng());
            eprintln!("🎲 SHUFFLE: After shuffle - indices={:?}", indices);
            state.queue.shuffle_order = indices;

            // Update current_index to point to the same track in the new shuffle order
            if let Some(track_pos) = current_track_position {
                if let Some(shuffle_idx) = state.queue.shuffle_order.iter().position(|&idx| idx == track_pos) {
                    eprintln!("🎲 SHUFFLE: track_pos={}, shuffle_idx={}", track_pos, shuffle_idx);
                    state.queue.current_index = shuffle_idx;
                }
            }
        } else {
            // When disabling shuffle, set current_index to the actual track position
            if let Some(track_pos) = current_track_position {
                state.queue.current_index = track_pos;
            }
        }

        eprintln!("🎲 SHUFFLE: {} (current_index={}, shuffle_order={:?})",
            if enabled { "enabled" } else { "disabled" },
            state.queue.current_index,
            state.queue.shuffle_order
        );
        drop(state);
        self.save_queue_state();
        Ok(())
    }

    pub fn set_repeat(&self, mode: RepeatMode) -> Result<()> {
        self.state.lock().unwrap().queue.repeat_mode = mode;
        info!("Repeat mode set to {:?}", mode);
        self.save_queue_state();
        Ok(())
    }

    pub fn is_shuffle_enabled(&self) -> bool {
        self.state.lock().unwrap().queue.shuffle_enabled
    }

    pub fn get_shuffle_order(&self) -> Vec<usize> {
        self.state.lock().unwrap().queue.shuffle_order.clone()
    }

    pub fn get_repeat_mode(&self) -> RepeatMode {
        self.state.lock().unwrap().queue.repeat_mode
    }

    // ========================================================================
    // Volume
    // ========================================================================

    pub fn set_volume(&self, volume: f32) -> Result<()> {
        let volume = volume.clamp(0.0, 1.0);

        {
            let sink_lock = self.sink.lock().unwrap();
            if let Some(ref sink) = *sink_lock {
                sink.set_volume(volume);
            }
        }

        self.state.lock().unwrap().volume = volume;
        info!("Volume set to {}", volume);
        Ok(())
    }

    pub fn get_volume(&self) -> f32 {
        self.state.lock().unwrap().volume
    }
}
