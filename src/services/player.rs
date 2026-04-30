// Audio playback service using custom AudioEngine (cpal direct) for decoding
// and lock-free real-time rendering. rodio is used only for decoding.

use crate::db::DbPool;
use crate::models::{Queue, RepeatMode, Track};
use crate::services::audio_engine::AudioEngine;
use crate::services::error::PlayerError;
use crate::services::events::PlaybackState;
use crate::utils::app_state::{self, PersistedState};
use log::{info, warn};
use rodio::source::UniformSourceIterator;
use rodio::{Decoder, Source};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, PlayerError>;

pub struct PlayerService {
    pool: DbPool,
    state: Arc<Mutex<PlayerState>>,
    engine: Arc<Mutex<Option<AudioEngine>>>,
    loading: Arc<AtomicBool>,
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

        info!(
            "Restored queue with {} tracks (index {})",
            queue.tracks.len(),
            queue.current_index
        );

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
            engine: Arc::new(Mutex::new(None)),
            loading: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Initialize the audio engine (lazy initialization).
    fn ensure_engine(&self) -> Result<()> {
        let mut engine = self.engine.lock().unwrap();
        if engine.is_none() {
            *engine =
                Some(AudioEngine::new().map_err(|e| {
                    PlayerError::Audio(format!("Failed to create audio engine: {e}"))
                })?);
        }
        Ok(())
    }

    /// Lock the engine and run a closure with a reference to AudioControls.
    fn with_controls<T>(
        &self,
        f: impl FnOnce(&crate::services::audio_engine::AudioControls) -> T,
    ) -> Result<T> {
        let engine = self.engine.lock().unwrap();
        let eng = engine
            .as_ref()
            .ok_or_else(|| PlayerError::Audio("Audio engine not initialized".into()))?;
        Ok(f(eng.controls()))
    }

    // ========================================================================
    // Playback Control
    // ========================================================================

    /// Start playing a track.
    ///
    /// The file read happens in a background thread so NAS latency doesn't
    /// block the UI. The track state is updated immediately; `playback_start_time`
    /// is set when audio actually starts.
    pub fn play(&self, track_id: i64) -> Result<()> {
        info!("Playing track: {track_id}");

        self.ensure_engine()?;

        // Get track from database
        let conn = self
            .pool
            .get()
            .map_err(|e| PlayerError::Audio(format!("Database connection error: {e}")))?;

        let track = crate::db::queries::get_track(&conn, track_id)
            .map_err(|e| PlayerError::Audio(format!("Database error: {e}")))?
            .ok_or(PlayerError::TrackNotFound(track_id))?;

        // Quick existence check (stat syscall, no data read — fast even on NAS)
        if !track.file_path.exists() {
            return Err(PlayerError::FileNotFound(
                track.file_path.to_string_lossy().into_owned(),
            ));
        }

        // Grab what we need for the background thread before updating state
        let volume = self.state.lock().unwrap().volume;
        let (dev_ch, dev_sr) = self.with_controls(|c| c.device_config())?;
        let bg_handle = self.with_controls(|c| c.bg_handle())?;
        let bg_state = Arc::clone(&self.state);
        let bg_loading = Arc::clone(&self.loading);
        let file_path = track.file_path.clone();

        self.loading.store(true, Relaxed);

        // Update state immediately so the UI reflects the new track
        {
            let mut state = self.state.lock().unwrap();
            state.current_track = Some(track);
            state.playback_state = PlaybackState::Playing;
            // Will be set accurately when audio actually starts
            state.playback_start_time = Some(Instant::now());
            state.paused_at = None;

            state.skipped_track_ids.remove(&track_id);

            if state.queue.current_track() != Some(track_id)
                && let Some(position) = state.queue.tracks.iter().position(|&id| id == track_id)
            {
                if state.queue.shuffle_enabled {
                    if let Some(shuffle_idx) = state
                        .queue
                        .shuffle_order
                        .iter()
                        .position(|&idx| idx == position)
                    {
                        state.queue.current_index = shuffle_idx;
                    }
                } else {
                    state.queue.current_index = position;
                }
            }
        }

        // Background: read file from NAS, decode, send to audio callback
        std::thread::spawn(move || {
            let file_data = match std::fs::read(&file_path) {
                Ok(data) => data,
                Err(e) => {
                    warn!("Background file read failed: {e}");
                    bg_loading.store(false, Relaxed);
                    let mut state = bg_state.lock().unwrap();
                    state.playback_state = PlaybackState::Stopped;
                    state.current_track = None;
                    return;
                }
            };

            let decoder = match Decoder::new(std::io::Cursor::new(file_data)) {
                Ok(d) => d,
                Err(e) => {
                    warn!("Background decode failed: {e}");
                    bg_loading.store(false, Relaxed);
                    let mut state = bg_state.lock().unwrap();
                    state.playback_state = PlaybackState::Stopped;
                    state.current_track = None;
                    return;
                }
            };

            let source: Box<dyn Iterator<Item = f32> + Send> =
                Box::new(UniformSourceIterator::<_, f32>::new(
                    decoder.convert_samples::<f32>(),
                    dev_ch,
                    dev_sr,
                ));

            // Send source to the audio callback
            bg_handle.set_volume(volume);
            bg_handle.play_source(source);
            bg_loading.store(false, Relaxed);

            // Set playback_start_time NOW — when audio actually begins
            bg_state.lock().unwrap().playback_start_time = Some(Instant::now());
        });

        Ok(())
    }

    /// Pause playback
    pub fn pause(&self) -> Result<()> {
        self.with_controls(|c| c.set_paused(true))?;

        let mut state = self.state.lock().unwrap();
        if let Some(start_time) = state.playback_start_time {
            state.paused_at = Some(start_time.elapsed());
        }
        state.playback_state = PlaybackState::Paused;

        info!("Playback paused");
        Ok(())
    }

    /// Resume playback
    pub fn resume(&self) -> Result<()> {
        self.with_controls(|c| c.set_paused(false))?;

        let mut state = self.state.lock().unwrap();
        if let Some(paused_at) = state.paused_at {
            state.playback_start_time = Some(Instant::now() - paused_at);
            state.paused_at = None;
        }
        state.playback_state = PlaybackState::Playing;

        info!("Playback resumed");
        Ok(())
    }

    /// Stop playback
    pub fn stop(&self) -> Result<()> {
        // Tell the render callback to drop the current source
        let _ = self.with_controls(|c| c.stop());
        self.loading.store(false, Relaxed);

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

    /// Seek to position in the current track.
    ///
    /// Re-decodes from the beginning with `skip_duration`, converting to
    /// the device format, then sends the new source to the render callback.
    pub fn seek(&self, position: Duration) -> Result<()> {
        // Get current track info before locking state
        let (track, was_playing, volume) = {
            let state = self.state.lock().unwrap();

            let track = state
                .current_track
                .as_ref()
                .ok_or(PlayerError::EmptyQueue)?
                .clone(); // clone: must escape MutexGuard scope

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
            info!("Seeked to {clamped_position:?} while paused");
            return Ok(());
        }

        self.ensure_engine()?;

        let (dev_ch, dev_sr) = self.with_controls(|c| c.device_config())?;
        let bg_handle = self.with_controls(|c| c.bg_handle())?;
        let bg_state = Arc::clone(&self.state);
        let bg_loading = Arc::clone(&self.loading);
        let file_path = track.file_path.clone();

        self.loading.store(true, Relaxed);

        // Update position tracking immediately
        {
            let mut state = self.state.lock().unwrap();
            state.playback_start_time = Some(Instant::now() - clamped_position);
            state.playback_state = PlaybackState::Playing;
        }

        // Background: read file, decode with skip, send to callback
        std::thread::spawn(move || {
            let file_data = match std::fs::read(&file_path) {
                Ok(data) => data,
                Err(e) => {
                    warn!("Seek background read failed: {e}");
                    bg_loading.store(false, Relaxed);
                    return;
                }
            };
            let decoder = match Decoder::new(std::io::Cursor::new(file_data)) {
                Ok(d) => d,
                Err(e) => {
                    warn!("Seek background decode failed: {e}");
                    bg_loading.store(false, Relaxed);
                    return;
                }
            };

            let source: Box<dyn Iterator<Item = f32> + Send> =
                Box::new(UniformSourceIterator::<_, f32>::new(
                    decoder
                        .skip_duration(clamped_position)
                        .convert_samples::<f32>(),
                    dev_ch,
                    dev_sr,
                ));

            bg_handle.set_volume(volume);
            bg_handle.play_source(source);
            bg_loading.store(false, Relaxed);

            bg_state.lock().unwrap().playback_start_time = Some(Instant::now() - clamped_position);
        });

        info!("Seeked to {clamped_position:?}");
        Ok(())
    }

    /// Skip to next track, auto-skipping up to 3 consecutive missing files
    pub fn next(&self) -> Result<()> {
        let mut consecutive_misses = 0;

        loop {
            let track_id = {
                let mut state = self.state.lock().unwrap();

                if state.queue.is_empty() {
                    drop(state);
                    return self.stop();
                }

                // RepeatMode::One replays the current track — do not advance
                if state.queue.repeat_mode == RepeatMode::One {
                    state.queue.current_track().ok_or(PlayerError::EmptyQueue)?
                } else {
                    // Advance one step
                    state.queue.current_index += 1;

                    // Handle end of queue
                    if state.queue.current_index >= state.queue.len() {
                        match state.queue.repeat_mode {
                            RepeatMode::All => state.queue.current_index = 0,
                            RepeatMode::Off => {
                                drop(state);
                                return self.stop();
                            }
                            RepeatMode::One => unreachable!(),
                        }
                    }

                    state.queue.current_track().ok_or(PlayerError::EmptyQueue)?
                }
            };

            match self.play(track_id) {
                Ok(()) => {
                    self.save_queue_state();
                    return Ok(());
                }
                Err(PlayerError::FileNotFound(_path)) if consecutive_misses < 3 => {
                    warn!("Track {track_id} file not found in next(), skipping");
                    let mut state = self.state.lock().unwrap();
                    // Cap the set at 500 entries to prevent unbounded growth
                    if state.skipped_track_ids.len() < 500 {
                        state.skipped_track_ids.insert(track_id);
                    }
                    drop(state);
                    consecutive_misses += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Skip to previous track (or restart if < 3s)
    pub fn previous(&self) -> Result<()> {
        let should_restart = false;

        if should_restart {
            // Restart current track
            let track_id = self
                .state
                .lock()
                .unwrap()
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

        let track_id = state.queue.current_track().ok_or(PlayerError::EmptyQueue)?;

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
                let track_id = state
                    .queue
                    .tracks
                    .get(current)
                    .copied()
                    .ok_or(PlayerError::EmptyQueue)?;

                // Set current_index to the exact visual position before calling
                // play(), so that play()'s duplicate-guard leaves it untouched.
                // With shuffle, map the raw position to its slot in shuffle_order.
                if state.queue.shuffle_enabled {
                    if let Some(shuffle_idx) =
                        state.queue.shuffle_order.iter().position(|&i| i == current)
                    {
                        state.queue.current_index = shuffle_idx;
                    }
                } else {
                    state.queue.current_index = current;
                }

                track_id
            };

            match self.play(track_id) {
                Ok(()) => {
                    self.save_queue_state();
                    return Ok(());
                }
                Err(PlayerError::FileNotFound(path)) => {
                    warn!("Track {track_id} file not found ({path}), skipping");
                    let mut state = self.state.lock().unwrap();
                    if state.skipped_track_ids.len() < 500 {
                        state.skipped_track_ids.insert(track_id);
                    }
                    drop(state);
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
                queue: state.queue.tracks.clone(), // clone: PersistedState needs owned Vec
                queue_index: state.queue.current_index,
                shuffle_order: state.queue.shuffle_order.clone(), // clone: PersistedState needs owned Vec
                volume: state.volume,
                shuffle_enabled: state.queue.shuffle_enabled,
                repeat_mode: state.queue.repeat_mode,
            }
        };
        if let Err(e) = app_state::save_state(&self.pool, &persisted) {
            warn!("Failed to persist queue state: {e}");
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

        info!("Added {count} tracks to queue");
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

        info!("Removed track at position {position}");
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

        info!("Moved track from position {from} to {to}");
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
        self.state.lock().unwrap().queue.tracks.clone() // clone: return owned Vec across lock boundary
    }

    /// Get current queue index
    pub fn get_queue_index(&self) -> usize {
        self.state.lock().unwrap().queue.current_index
    }

    /// Get track IDs that failed to play due to missing files
    pub fn get_skipped_track_ids(&self) -> Vec<i64> {
        self.state
            .lock()
            .unwrap()
            .skipped_track_ids
            .iter()
            .copied()
            .collect()
    }

    /// Clear the set of unavailable (skipped) track IDs.
    /// Call this after a network volume has been remounted so that previously
    /// unreachable tracks are retried on the next playback attempt.
    pub fn clear_skipped_tracks(&self) {
        self.state.lock().unwrap().skipped_track_ids.clear();
        info!("Cleared unavailable track list — network volume may have remounted");
    }

    // ========================================================================
    // Playback State
    // ========================================================================

    pub fn get_state(&self) -> PlaybackState {
        self.state.lock().unwrap().playback_state
    }

    pub fn get_current_track(&self) -> Option<Track> {
        self.state.lock().unwrap().current_track.clone() // clone: return owned Track across lock boundary
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
            PlaybackState::Paused => state.paused_at.map_or(0.0, |d| d.as_secs_f64()),
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
            if !state.queue.shuffle_order.is_empty()
                && state.queue.current_index < state.queue.shuffle_order.len()
            {
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
            log::debug!("SHUFFLE: Before shuffle - indices={indices:?}");
            indices.shuffle(&mut thread_rng());
            log::debug!("SHUFFLE: After shuffle - indices={indices:?}");
            state.queue.shuffle_order = indices;

            // Update current_index to point to the same track in the new shuffle order
            if let Some(track_pos) = current_track_position
                && let Some(shuffle_idx) = state
                    .queue
                    .shuffle_order
                    .iter()
                    .position(|&idx| idx == track_pos)
            {
                log::debug!("SHUFFLE: track_pos={track_pos}, shuffle_idx={shuffle_idx}");
                state.queue.current_index = shuffle_idx;
            }
        } else {
            // When disabling shuffle, set current_index to the actual track position
            if let Some(track_pos) = current_track_position {
                state.queue.current_index = track_pos;
            }
        }

        log::debug!(
            "SHUFFLE: {} (current_index={}, shuffle_order={:?})",
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
        info!("Repeat mode set to {mode:?}");
        self.save_queue_state();
        Ok(())
    }

    pub fn is_shuffle_enabled(&self) -> bool {
        self.state.lock().unwrap().queue.shuffle_enabled
    }

    pub fn get_shuffle_order(&self) -> Vec<usize> {
        self.state.lock().unwrap().queue.shuffle_order.clone() // clone: return owned Vec across lock boundary
    }

    pub fn get_repeat_mode(&self) -> RepeatMode {
        self.state.lock().unwrap().queue.repeat_mode
    }

    // ========================================================================
    // Volume
    // ========================================================================

    pub fn set_volume(&self, volume: f32) -> Result<()> {
        let volume = volume.clamp(0.0, 1.0);

        let _ = self.with_controls(|c| c.set_volume(volume));

        self.state.lock().unwrap().volume = volume;
        info!("Volume set to {volume}");
        Ok(())
    }

    pub fn get_volume(&self) -> f32 {
        self.state.lock().unwrap().volume
    }

    /// Returns true while the background thread is reading/decoding a file.
    pub fn is_loading(&self) -> bool {
        self.loading.load(Relaxed)
    }

    /// Returns true if the current track has finished playing naturally.
    ///
    /// Uses two independent checks:
    /// 1. Primary: `controls.is_finished()` — the render callback sets this
    ///    atomically when the source iterator returns `None`.
    /// 2. Fallback: elapsed wall-clock time > track duration + 1 s — catches
    ///    edge cases where the source never signals exhaustion.
    pub fn is_track_finished(&self) -> bool {
        // If a new track is currently being loaded, the audio engine's `finished`
        // flag may still reflect the previous track. Suppress auto-advance until
        // the background thread has sent the new source to the render callback.
        if self.loading.load(Relaxed) {
            return false;
        }

        let (is_playing, fallback_info) = {
            let state = self.state.lock().unwrap();
            let is_playing = matches!(state.playback_state, PlaybackState::Playing);
            let fallback = if is_playing {
                if let (Some(track), Some(start)) =
                    (&state.current_track, state.playback_start_time)
                {
                    Some((track.duration, start.elapsed()))
                } else {
                    None
                }
            } else {
                None
            };
            (is_playing, fallback)
        };

        if !is_playing {
            return false;
        }

        // Primary: render callback signalled source exhaustion
        if self.with_controls(|c| c.is_finished()).unwrap_or(false) {
            return true;
        }

        // Fallback: wall-clock elapsed > track duration + 1 s
        if let Some((duration, elapsed)) = fallback_info {
            let duration_secs = duration.as_secs_f64();
            if duration_secs > 0.0 && elapsed.as_secs_f64() > duration_secs + 1.0 {
                return true;
            }
        }

        false
    }

    /// Test-only helper: inject a "playing" state without real audio, so the
    /// fallback path of `is_track_finished()` can be exercised in isolation.
    /// The sink is left as `None`, which makes `sink.empty()` evaluate to `false`.
    #[cfg(test)]
    pub fn simulate_playing_for_test(&self, track: Track, start_time: std::time::Instant) {
        let mut state = self.state.lock().unwrap();
        state.current_track = Some(track);
        state.playback_state = PlaybackState::Playing;
        state.playback_start_time = Some(start_time);
        state.paused_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::db::queries;
    use crate::models::AudioFormat;
    use crate::models::source::SourceType;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tempfile::TempDir;

    // =========================================================================
    // Helpers
    // =========================================================================

    fn create_test_pool() -> (TempDir, DbPool) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = db::create_pool(db_path).unwrap();
        db::initialize_database(&pool).unwrap();
        (dir, pool)
    }

    /// Write a minimal valid WAV file (0.5 second, 44100 Hz, mono, 16-bit PCM).
    fn create_test_wav(path: &Path) {
        let sample_rate: u32 = 44100;
        let num_samples: u32 = 22050; // 0.5 s
        let num_channels: u16 = 1;
        let bits_per_sample: u16 = 16;
        let byte_rate = sample_rate * num_channels as u32 * bits_per_sample as u32 / 8;
        let block_align = num_channels * bits_per_sample / 8;
        let data_size = num_samples * num_channels as u32 * bits_per_sample as u32 / 8;
        let file_size = 36 + data_size;

        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(b"RIFF").unwrap();
        f.write_all(&file_size.to_le_bytes()).unwrap();
        f.write_all(b"WAVE").unwrap();
        f.write_all(b"fmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
        f.write_all(&num_channels.to_le_bytes()).unwrap();
        f.write_all(&sample_rate.to_le_bytes()).unwrap();
        f.write_all(&byte_rate.to_le_bytes()).unwrap();
        f.write_all(&block_align.to_le_bytes()).unwrap();
        f.write_all(&bits_per_sample.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&data_size.to_le_bytes()).unwrap();
        for _ in 0..num_samples {
            f.write_all(&0i16.to_le_bytes()).unwrap();
        }
    }

    /// Insert a test track into the database and return its ID.
    fn insert_test_track(pool: &DbPool, file_path: &Path, title: &str) -> i64 {
        let conn = pool.get().unwrap();
        let artist_id = queries::insert_artist(&conn, "Test Artist", None).unwrap();
        let source_id = queries::insert_source(
            &conn,
            title, // use title as unique source name to avoid conflicts
            SourceType::Disk,
            Some(&file_path.parent().unwrap().to_path_buf()),
        )
        .unwrap();
        queries::insert_track(
            &conn,
            title,
            None,
            artist_id,
            source_id,
            &file_path.to_path_buf(),
            500,
            None,
            Some(44100),
            Some(16),
            AudioFormat::Flac,
            0,
        )
        .unwrap()
    }

    fn make_player(pool: DbPool) -> PlayerService {
        PlayerService::new(pool).unwrap()
    }

    // =========================================================================
    // Queue management
    // =========================================================================

    #[test]
    fn test_set_queue_empty() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![]).unwrap();
        assert!(player.get_queue().is_empty());
        assert_eq!(player.get_queue_index(), 0);
    }

    #[test]
    fn test_set_queue_with_tracks() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![10, 20, 30]).unwrap();
        assert_eq!(player.get_queue(), vec![10, 20, 30]);
        assert_eq!(player.get_queue_index(), 0);
    }

    #[test]
    fn test_add_to_queue() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2]).unwrap();
        player.add_to_queue(vec![3, 4]).unwrap();
        assert_eq!(player.get_queue(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_add_to_empty_queue() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.add_to_queue(vec![5, 6]).unwrap();
        assert_eq!(player.get_queue(), vec![5, 6]);
    }

    #[test]
    fn test_remove_from_queue_middle() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3]).unwrap();
        player.remove_from_queue(1).unwrap();
        assert_eq!(player.get_queue(), vec![1, 3]);
    }

    #[test]
    fn test_remove_from_queue_last() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3]).unwrap();
        player.remove_from_queue(2).unwrap();
        assert_eq!(player.get_queue(), vec![1, 2]);
    }

    #[test]
    fn test_remove_from_queue_invalid_position() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3]).unwrap();
        assert!(matches!(
            player.remove_from_queue(5),
            Err(PlayerError::InvalidPosition)
        ));
    }

    #[test]
    fn test_remove_from_queue_adjusts_current_index() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3, 4]).unwrap();
        player.state.lock().unwrap().queue.current_index = 2;
        // remove position 1 (before current) → current should shift down
        player.remove_from_queue(1).unwrap();
        assert_eq!(player.get_queue_index(), 1);
        assert_eq!(player.get_queue(), vec![1, 3, 4]);
    }

    #[test]
    fn test_remove_at_index_zero_keeps_index_zero() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3]).unwrap();
        player.remove_from_queue(0).unwrap();
        assert_eq!(player.get_queue_index(), 0);
        assert_eq!(player.get_queue(), vec![2, 3]);
    }

    #[test]
    fn test_move_in_queue_forward() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3, 4]).unwrap();
        player.move_in_queue(0, 2).unwrap();
        assert_eq!(player.get_queue(), vec![2, 3, 1, 4]);
    }

    #[test]
    fn test_move_in_queue_backward() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3, 4]).unwrap();
        player.move_in_queue(3, 1).unwrap();
        assert_eq!(player.get_queue(), vec![1, 4, 2, 3]);
    }

    #[test]
    fn test_move_in_queue_invalid_from() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3]).unwrap();
        assert!(matches!(
            player.move_in_queue(10, 0),
            Err(PlayerError::InvalidPosition)
        ));
    }

    #[test]
    fn test_move_in_queue_invalid_to() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3]).unwrap();
        assert!(matches!(
            player.move_in_queue(0, 10),
            Err(PlayerError::InvalidPosition)
        ));
    }

    #[test]
    fn test_move_in_queue_updates_current_index_when_current_track_moves() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3, 4]).unwrap();
        player.state.lock().unwrap().queue.current_index = 0;
        player.move_in_queue(0, 2).unwrap();
        assert_eq!(player.get_queue_index(), 2);
    }

    #[test]
    fn test_clear_queue() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3]).unwrap();
        player.clear_queue().unwrap();
        assert!(player.get_queue().is_empty());
        assert_eq!(player.get_queue_index(), 0);
    }

    #[test]
    fn test_clear_queue_also_clears_skipped_set() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.state.lock().unwrap().skipped_track_ids.insert(42);
        player.clear_queue().unwrap();
        assert!(player.get_skipped_track_ids().is_empty());
    }

    // =========================================================================
    // Volume
    // =========================================================================

    #[test]
    fn test_initial_volume_is_one() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert_eq!(player.get_volume(), 1.0);
    }

    #[test]
    fn test_set_volume() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_volume(0.5).unwrap();
        assert_eq!(player.get_volume(), 0.5);
    }

    #[test]
    fn test_set_volume_clamps_below_zero() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_volume(-1.0).unwrap();
        assert_eq!(player.get_volume(), 0.0);
    }

    #[test]
    fn test_set_volume_clamps_above_one() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_volume(2.0).unwrap();
        assert_eq!(player.get_volume(), 1.0);
    }

    // =========================================================================
    // Repeat mode
    // =========================================================================

    #[test]
    fn test_initial_repeat_mode_is_off() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert_eq!(player.get_repeat_mode(), RepeatMode::Off);
    }

    #[test]
    fn test_set_repeat_all() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_repeat(RepeatMode::All).unwrap();
        assert_eq!(player.get_repeat_mode(), RepeatMode::All);
    }

    #[test]
    fn test_set_repeat_one() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_repeat(RepeatMode::One).unwrap();
        assert_eq!(player.get_repeat_mode(), RepeatMode::One);
    }

    #[test]
    fn test_set_repeat_off_from_all() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_repeat(RepeatMode::All).unwrap();
        player.set_repeat(RepeatMode::Off).unwrap();
        assert_eq!(player.get_repeat_mode(), RepeatMode::Off);
    }

    // =========================================================================
    // Shuffle
    // =========================================================================

    #[test]
    fn test_shuffle_initially_disabled() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert!(!player.is_shuffle_enabled());
    }

    #[test]
    fn test_enable_shuffle_generates_complete_order() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![10, 20, 30, 40]).unwrap();
        player.set_shuffle(true).unwrap();
        assert!(player.is_shuffle_enabled());
        let mut order = player.get_shuffle_order();
        assert_eq!(order.len(), 4);
        order.sort();
        assert_eq!(order, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_disable_shuffle() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3]).unwrap();
        player.set_shuffle(true).unwrap();
        player.set_shuffle(false).unwrap();
        assert!(!player.is_shuffle_enabled());
    }

    #[test]
    fn test_set_queue_while_shuffle_enabled_regenerates_order() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_shuffle(true).unwrap();
        player.set_queue(vec![1, 2, 3, 4, 5]).unwrap();
        let mut order = player.get_shuffle_order();
        assert_eq!(order.len(), 5);
        order.sort();
        assert_eq!(order, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_add_to_queue_while_shuffle_enabled_expands_order() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3]).unwrap();
        player.set_shuffle(true).unwrap();
        player.add_to_queue(vec![4, 5]).unwrap();
        assert_eq!(player.get_shuffle_order().len(), 5);
    }

    // =========================================================================
    // Initial playback state
    // =========================================================================

    #[test]
    fn test_initial_state_is_stopped() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert_eq!(player.get_state(), PlaybackState::Stopped);
    }

    #[test]
    fn test_initial_position_is_zero() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert_eq!(player.get_position(), 0.0);
    }

    #[test]
    fn test_initial_current_track_is_none() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert!(player.get_current_track().is_none());
    }

    #[test]
    fn test_initial_skipped_track_ids_is_empty() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert!(player.get_skipped_track_ids().is_empty());
    }

    #[test]
    fn test_is_track_finished_is_false_when_stopped() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert!(!player.is_track_finished());
    }

    #[test]
    fn test_is_track_finished_is_true_after_short_track_plays() {
        // Verify that is_track_finished() returns true once a short track is done.
        // This is the core requirement for auto-advance to work.
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav); // 0.5 s
        let track_id = insert_test_track(&pool, &wav, "Short Track");
        let player = make_player(pool);
        player.play(track_id).unwrap();
        // Wait long enough for the 0.5 s track to finish
        std::thread::sleep(Duration::from_millis(1000));
        assert!(
            player.is_track_finished(),
            "is_track_finished() must return true after the track plays to completion"
        );
    }

    #[test]
    fn test_is_track_finished_fallback_when_elapsed_exceeds_duration() {
        // Regression: decoders such as FLAC via symphonia may not signal EOF
        // cleanly, leaving sink.empty() == false indefinitely.  The duration-
        // based fallback must fire and return true when wall-clock elapsed time
        // exceeds track duration + 1 s.
        //
        // simulate_playing_for_test() leaves the sink as None, so
        // sink.empty() evaluates to false — isolating the fallback path.
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);

        // Construct a Track with a 3-second duration.
        let fake_track = Track {
            id: 1,
            title: "Fake FLAC".to_string(),
            album_id: None,
            artist_id: 1,
            source_id: 1,
            file_path: PathBuf::from("/fake/track.flac"),
            duration: Duration::from_secs(3),
            track_number: None,
            disc_number: 0,
            sample_rate: Some(44100),
            bit_depth: Some(16),
            file_type: crate::models::AudioFormat::Flac,
            file_size: 0,
            rating: crate::models::Rating(0),
            fingerprint: None,
            is_duplicate: false,
            duplicate_of: None,
            last_played_at: None,
            play_count: 0,
            artist_names: vec![],
        };

        // Simulate playback that started 5 seconds ago (3s duration + 1s buffer = 4s threshold).
        let start_time = std::time::Instant::now() - Duration::from_secs(5);
        player.simulate_playing_for_test(fake_track, start_time);

        assert!(
            player.is_track_finished(),
            "is_track_finished() must return true via the duration fallback \
             when elapsed > duration + 1 s even if sink.empty() is false"
        );
    }

    #[test]
    fn test_is_track_finished_fallback_does_not_fire_before_threshold() {
        // The fallback must NOT fire while the track is still within its
        // expected duration — this prevents premature auto-advance.
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);

        let fake_track = Track {
            id: 2,
            title: "Fake FLAC 2".to_string(),
            album_id: None,
            artist_id: 1,
            source_id: 1,
            file_path: PathBuf::from("/fake/track2.flac"),
            duration: Duration::from_secs(300), // 5-minute track
            track_number: None,
            disc_number: 0,
            sample_rate: Some(44100),
            bit_depth: Some(16),
            file_type: crate::models::AudioFormat::Flac,
            file_size: 0,
            rating: crate::models::Rating(0),
            fingerprint: None,
            is_duplicate: false,
            duplicate_of: None,
            last_played_at: None,
            play_count: 0,
            artist_names: vec![],
        };

        // Simulate playback that started 10 seconds ago — well within 300 s duration.
        let start_time = std::time::Instant::now() - Duration::from_secs(10);
        player.simulate_playing_for_test(fake_track, start_time);

        assert!(
            !player.is_track_finished(),
            "is_track_finished() must return false while the track is still playing"
        );
    }

    // =========================================================================
    // Error cases — no audio device needed
    // =========================================================================

    #[test]
    fn test_pause_without_engine_returns_error() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert!(player.pause().is_err());
    }

    #[test]
    fn test_resume_without_engine_returns_error() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert!(player.resume().is_err());
    }

    #[test]
    fn test_stop_without_audio_is_ok() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert!(player.stop().is_ok());
        assert_eq!(player.get_state(), PlaybackState::Stopped);
    }

    #[test]
    fn test_next_on_empty_queue_stops_player() {
        // next() on an empty queue stops playback (prevents stuck-in-playing loop)
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert!(player.next().is_ok());
        assert_eq!(player.get_state(), PlaybackState::Stopped);
    }

    #[test]
    fn test_previous_on_empty_queue_returns_error() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert!(matches!(player.previous(), Err(PlayerError::EmptyQueue)));
    }

    #[test]
    fn test_jump_to_index_on_empty_queue_returns_error() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert!(matches!(
            player.jump_to_index(0),
            Err(PlayerError::EmptyQueue)
        ));
    }

    #[test]
    fn test_jump_to_index_out_of_bounds_returns_invalid_position() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3]).unwrap();
        assert!(matches!(
            player.jump_to_index(10),
            Err(PlayerError::InvalidPosition)
        ));
    }

    #[test]
    fn test_seek_without_current_track_returns_error() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert!(matches!(
            player.seek(Duration::from_secs(1)),
            Err(PlayerError::EmptyQueue)
        ));
    }

    #[test]
    fn test_previous_at_start_with_repeat_off_returns_invalid_position() {
        // Uses fake IDs — previous() returns early before calling play()
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3]).unwrap();
        player.set_repeat(RepeatMode::Off).unwrap();
        assert!(matches!(
            player.previous(),
            Err(PlayerError::InvalidPosition)
        ));
    }

    #[test]
    fn test_next_at_end_with_repeat_off_stops_playback() {
        // Uses a fake ID — next() hits end-of-queue, calls stop() without playing
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![99]).unwrap();
        player.set_repeat(RepeatMode::Off).unwrap();
        player.next().unwrap();
        assert_eq!(player.get_state(), PlaybackState::Stopped);
    }

    // =========================================================================
    // Playback with real audio — requires CoreAudio (macOS)
    // =========================================================================

    #[test]
    fn test_play_nonexistent_track_id_returns_track_not_found() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        assert!(matches!(
            player.play(99999),
            Err(PlayerError::TrackNotFound(99999))
        ));
    }

    #[test]
    fn test_play_missing_file_returns_file_not_found() {
        let (dir, pool) = create_test_pool();
        let missing = dir.path().join("does_not_exist.wav");
        let track_id = insert_test_track(&pool, &missing, "Missing Track");
        let player = make_player(pool);
        assert!(matches!(
            player.play(track_id),
            Err(PlayerError::FileNotFound(_))
        ));
    }

    #[test]
    fn test_play_valid_file_changes_state_to_playing() {
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let track_id = insert_test_track(&pool, &wav, "Test Track");
        let player = make_player(pool);
        player.play(track_id).unwrap();
        assert_eq!(player.get_state(), PlaybackState::Playing);
        assert_eq!(player.get_current_track().unwrap().id, track_id);
    }

    #[test]
    fn test_play_then_pause_changes_state_to_paused() {
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let track_id = insert_test_track(&pool, &wav, "Test Track");
        let player = make_player(pool);
        player.play(track_id).unwrap();
        player.pause().unwrap();
        assert_eq!(player.get_state(), PlaybackState::Paused);
    }

    #[test]
    fn test_play_pause_resume_restores_playing_state() {
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let track_id = insert_test_track(&pool, &wav, "Test Track");
        let player = make_player(pool);
        player.play(track_id).unwrap();
        player.pause().unwrap();
        player.resume().unwrap();
        assert_eq!(player.get_state(), PlaybackState::Playing);
    }

    #[test]
    fn test_play_then_stop_clears_track_and_state() {
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let track_id = insert_test_track(&pool, &wav, "Test Track");
        let player = make_player(pool);
        player.play(track_id).unwrap();
        player.stop().unwrap();
        assert_eq!(player.get_state(), PlaybackState::Stopped);
        assert!(player.get_current_track().is_none());
        assert_eq!(player.get_position(), 0.0);
    }

    #[test]
    fn test_position_advances_while_playing() {
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let track_id = insert_test_track(&pool, &wav, "Test Track");
        let player = make_player(pool);
        player.play(track_id).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            player.get_position() > 0.0,
            "position should advance while playing"
        );
    }

    #[test]
    fn test_position_stable_while_paused() {
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let track_id = insert_test_track(&pool, &wav, "Test Track");
        let player = make_player(pool);
        player.play(track_id).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        player.pause().unwrap();
        let pos1 = player.get_position();
        std::thread::sleep(Duration::from_millis(50));
        let pos2 = player.get_position();
        assert_eq!(pos1, pos2, "position should not change while paused");
    }

    #[test]
    fn test_seek_while_paused_updates_position() {
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let track_id = insert_test_track(&pool, &wav, "Test Track");
        let player = make_player(pool);
        player.play(track_id).unwrap();
        player.pause().unwrap();
        player.seek(Duration::from_millis(200)).unwrap();
        let pos = player.get_position();
        assert!(
            (pos - 0.2).abs() < 0.05,
            "expected ~0.2 s after seek, got {:.3} s",
            pos
        );
    }

    #[test]
    fn test_play_updates_queue_current_index() {
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let track_id = insert_test_track(&pool, &wav, "Test Track");
        let player = make_player(pool);
        player.set_queue(vec![999, 998, track_id, 997]).unwrap();
        player.play(track_id).unwrap();
        assert_eq!(player.get_queue_index(), 2);
    }

    #[test]
    fn test_set_volume_while_playing_takes_effect() {
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let track_id = insert_test_track(&pool, &wav, "Test Track");
        let player = make_player(pool);
        player.play(track_id).unwrap();
        player.set_volume(0.3).unwrap();
        assert_eq!(player.get_volume(), 0.3);
    }

    #[test]
    fn test_next_skips_missing_file_and_plays_subsequent() {
        let (dir, pool) = create_test_pool();
        let wav1 = dir.path().join("track1.wav");
        let wav2 = dir.path().join("track2.wav");
        let missing = dir.path().join("missing.wav");
        create_test_wav(&wav1);
        create_test_wav(&wav2);
        let id1 = insert_test_track(&pool, &wav1, "Track 1");
        let missing_id = insert_test_track(&pool, &missing, "Missing");
        let id2 = insert_test_track(&pool, &wav2, "Track 2");
        let player = make_player(pool);
        // Queue: [id1, missing_id, id2], current at index 0
        player.set_queue(vec![id1, missing_id, id2]).unwrap();
        // next() → advances to index 1 (missing), auto-skips, plays index 2
        player.next().unwrap();
        assert_eq!(player.get_state(), PlaybackState::Playing);
        assert_eq!(player.get_current_track().unwrap().id, id2);
        assert!(player.get_skipped_track_ids().contains(&missing_id));
    }

    #[test]
    fn test_play_removes_track_from_skipped_set() {
        let (dir, pool) = create_test_pool();
        let wav1 = dir.path().join("track1.wav");
        let wav2 = dir.path().join("track2.wav");
        let missing = dir.path().join("missing.wav");
        create_test_wav(&wav1);
        create_test_wav(&wav2);
        let id1 = insert_test_track(&pool, &wav1, "Track 1");
        let missing_id = insert_test_track(&pool, &missing, "Missing");
        let id2 = insert_test_track(&pool, &wav2, "Track 2");
        let player = make_player(pool);
        player.set_queue(vec![id1, missing_id, id2]).unwrap();
        // Add missing_id to the skipped set via next()
        player.next().unwrap();
        assert!(player.get_skipped_track_ids().contains(&missing_id));
        // File comes back — playing it should remove it from the skipped set
        create_test_wav(&missing);
        player.play(missing_id).unwrap();
        assert!(!player.get_skipped_track_ids().contains(&missing_id));
    }

    #[test]
    fn test_next_at_end_with_repeat_all_wraps_to_first() {
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let track_id = insert_test_track(&pool, &wav, "Test Track");
        let player = make_player(pool);
        player.set_queue(vec![track_id]).unwrap();
        player.set_repeat(RepeatMode::All).unwrap();
        player.next().unwrap();
        assert_eq!(player.get_queue_index(), 0);
        assert_eq!(player.get_state(), PlaybackState::Playing);
    }

    #[test]
    fn test_previous_at_start_with_repeat_all_wraps_to_last() {
        let (dir, pool) = create_test_pool();
        let wav1 = dir.path().join("track1.wav");
        let wav2 = dir.path().join("track2.wav");
        create_test_wav(&wav1);
        create_test_wav(&wav2);
        let id1 = insert_test_track(&pool, &wav1, "Track 1");
        let id2 = insert_test_track(&pool, &wav2, "Track 2");
        let player = make_player(pool);
        player.set_queue(vec![id1, id2]).unwrap();
        player.set_repeat(RepeatMode::All).unwrap();
        // At index 0, previous() with RepeatAll should jump to last track (index 1)
        player.previous().unwrap();
        assert_eq!(player.get_queue_index(), 1);
    }

    // =========================================================================
    // Regression: play() must not discard a multi-track queue
    // Bug: EventHandler::PlayTrack was calling set_queue([id]) after SetQueue,
    // reducing a multi-track queue to a single track and breaking auto-advance.
    // =========================================================================

    #[test]
    fn test_play_preserves_multitrack_queue() {
        // After set_queue([t1,t2,t3]) + play(t1), the queue must still have 3 tracks.
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let id1 = insert_test_track(&pool, &wav, "Track 1");
        let player = make_player(pool);
        player.set_queue(vec![id1, 20, 30]).unwrap();
        player.play(id1).unwrap();
        assert_eq!(
            player.get_queue(),
            vec![id1, 20, 30],
            "play() must not discard the multi-track queue"
        );
    }

    #[test]
    fn test_set_queue_then_play_first_next_plays_second() {
        // The full "play all" scenario: set_queue([t1,t2]) → play(t1) → next() → t2.
        let (dir, pool) = create_test_pool();
        let wav1 = dir.path().join("track1.wav");
        let wav2 = dir.path().join("track2.wav");
        create_test_wav(&wav1);
        create_test_wav(&wav2);
        let id1 = insert_test_track(&pool, &wav1, "Track 1");
        let id2 = insert_test_track(&pool, &wav2, "Track 2");
        let player = make_player(pool);
        player.set_queue(vec![id1, id2]).unwrap();
        player.play(id1).unwrap();
        assert_eq!(player.get_queue_index(), 0);
        player.next().unwrap();
        assert_eq!(player.get_queue_index(), 1);
        assert_eq!(player.get_current_track().unwrap().id, id2);
    }

    #[test]
    fn test_play_track_already_in_queue_does_not_reset_queue() {
        // Simulates the fixed EventHandler behaviour: when track is already in
        // the queue, play() is called directly without set_queue().
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let id1 = insert_test_track(&pool, &wav, "Track 1");
        let player = make_player(pool);
        player.set_queue(vec![id1, 20, 30]).unwrap();
        // Calling play() directly (as EventHandler now does when track is in queue)
        player.play(id1).unwrap();
        assert_eq!(player.get_queue(), vec![id1, 20, 30]);
        assert_eq!(player.get_queue_index(), 0);
    }

    // =========================================================================
    // Regression: RepeatMode::One must replay the *current* track immediately,
    // not just loop the last track when the queue is exhausted.
    // Bug: next() only activated RepeatMode::One at end-of-queue, so in a
    // multi-track queue every track advanced normally until the last one, which
    // then looped forever instead of staying on the current track throughout.
    // =========================================================================

    #[test]
    fn test_next_with_repeat_one_replays_current_track() {
        // With a single-track queue and RepeatMode::One, next() must replay
        // the same track (index stays at 0).
        let (dir, pool) = create_test_pool();
        let wav = dir.path().join("test.wav");
        create_test_wav(&wav);
        let track_id = insert_test_track(&pool, &wav, "Test Track");
        let player = make_player(pool);
        player.set_queue(vec![track_id]).unwrap();
        player.set_repeat(RepeatMode::One).unwrap();
        player.next().unwrap();
        assert_eq!(
            player.get_queue_index(),
            0,
            "RepeatMode::One must keep index at 0"
        );
        assert_eq!(
            player.get_current_track().unwrap().id,
            track_id,
            "RepeatMode::One must replay the same track"
        );
        assert_eq!(player.get_state(), PlaybackState::Playing);
    }

    #[test]
    fn test_next_with_repeat_one_does_not_advance_mid_queue() {
        // With a multi-track queue and RepeatMode::One, next() must NOT advance
        // to the next track — it must replay the current track at the same index.
        let (dir, pool) = create_test_pool();
        let wav1 = dir.path().join("track1.wav");
        let wav2 = dir.path().join("track2.wav");
        create_test_wav(&wav1);
        create_test_wav(&wav2);
        let id1 = insert_test_track(&pool, &wav1, "Track 1");
        let id2 = insert_test_track(&pool, &wav2, "Track 2");
        let player = make_player(pool);
        player.set_queue(vec![id1, id2]).unwrap();
        player.set_repeat(RepeatMode::One).unwrap();
        // Queue starts at index 0 (id1). next() must stay on id1, not jump to id2.
        player.next().unwrap();
        assert_eq!(
            player.get_queue_index(),
            0,
            "RepeatMode::One must not advance the queue index"
        );
        assert_eq!(
            player.get_current_track().unwrap().id,
            id1,
            "RepeatMode::One must replay track 1, not advance to track 2"
        );
    }

    // =========================================================================
    // Shuffle — T004
    // =========================================================================

    #[test]
    fn test_set_shuffle_enable_toggle() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![1, 2, 3, 4, 5]).unwrap();

        assert!(!player.is_shuffle_enabled());
        player.set_shuffle(true).unwrap();
        assert!(player.is_shuffle_enabled());
        player.set_shuffle(false).unwrap();
        assert!(!player.is_shuffle_enabled());
    }

    #[test]
    fn test_set_shuffle_order_length_invariant() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        let queue = vec![10i64, 20, 30, 40, 50];
        player.set_queue(queue.clone()).unwrap();

        player.set_shuffle(true).unwrap();
        let order = player.get_shuffle_order();
        assert_eq!(
            order.len(),
            queue.len(),
            "shuffle_order must contain exactly one entry per queue track"
        );
    }

    #[test]
    fn test_set_shuffle_order_contains_all_indices() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![10, 20, 30]).unwrap();

        player.set_shuffle(true).unwrap();
        let mut order = player.get_shuffle_order();
        order.sort_unstable();
        assert_eq!(
            order,
            vec![0, 1, 2],
            "shuffle_order must be a permutation of 0..queue_len"
        );
    }

    #[test]
    fn test_set_shuffle_current_track_preserved_in_order() {
        // When shuffle is enabled, the current track must appear at current_index
        // within the shuffle_order so the player still points to the same track.
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![10, 20, 30, 40, 50]).unwrap();

        // Move to index 2 (the third track) via direct state access
        {
            let mut state = player.state.lock().unwrap();
            state.queue.current_index = 2;
        }
        let expected_pos = 2usize;

        player.set_shuffle(true).unwrap();
        let shuffle_idx = player.get_queue_index();
        let order = player.get_shuffle_order();

        assert_eq!(
            order[shuffle_idx], expected_pos,
            "shuffle_order[current_index] must equal the original track position"
        );
    }

    #[test]
    fn test_set_shuffle_disable_restores_track_position() {
        let (_dir, pool) = create_test_pool();
        let player = make_player(pool);
        player.set_queue(vec![10, 20, 30]).unwrap();

        // Move to index 1 via direct state access
        {
            let mut state = player.state.lock().unwrap();
            state.queue.current_index = 1;
        }

        player.set_shuffle(true).unwrap();
        player.set_shuffle(false).unwrap();

        assert_eq!(
            player.get_queue_index(),
            1,
            "disabling shuffle must restore current_index to the original track position"
        );
        assert!(!player.is_shuffle_enabled());
    }
}
