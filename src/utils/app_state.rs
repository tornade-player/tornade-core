// Application state persistence

use serde::{Serialize, Deserialize};

use crate::db::DbPool;
use crate::models::RepeatMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    pub current_track_id: Option<i64>,
    pub playback_position: f64,  // Position in seconds
    pub queue: Vec<i64>,
    pub queue_index: usize,
    #[serde(default)]
    pub shuffle_order: Vec<usize>,
    pub volume: f32,
    pub shuffle_enabled: bool,
    pub repeat_mode: RepeatMode,
}

impl Default for PersistedState {
    fn default() -> Self {
        PersistedState {
            current_track_id: None,
            playback_position: 0.0,
            queue: Vec::new(),
            queue_index: 0,
            shuffle_order: Vec::new(),
            volume: 1.0,
            shuffle_enabled: false,
            repeat_mode: RepeatMode::Off,
        }
    }
}

/// Save application state to database
pub fn save_state(pool: &DbPool, state: &PersistedState) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get()?;
    let state_json = serde_json::to_string(state)?;

    conn.execute(
        "INSERT OR REPLACE INTO app_state (key, value, updated_at)
         VALUES ('playback_state', ?1, CURRENT_TIMESTAMP)",
        [&state_json],
    )?;

    Ok(())
}

/// Load application state from database
pub fn load_state(pool: &DbPool) -> Result<PersistedState, Box<dyn std::error::Error>> {
    let conn = pool.get()?;

    let state_json: String = conn.query_row(
        "SELECT value FROM app_state WHERE key = 'playback_state'",
        [],
        |row| row.get(0),
    ).unwrap_or_else(|_| {
        // Return default state if not found
        serde_json::to_string(&PersistedState::default()).unwrap()
    });

    let state: PersistedState = serde_json::from_str(&state_json)
        .unwrap_or_default();

    Ok(state)
}

/// Clear application state from database
pub fn clear_state(pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get()?;

    conn.execute(
        "DELETE FROM app_state WHERE key = 'playback_state'",
        [],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_pool() -> (tempfile::TempDir, DbPool) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = crate::db::create_pool(db_path.clone()).unwrap();
        crate::db::initialize_database(&pool).unwrap();
        (dir, pool)
    }

    #[test]
    fn test_save_and_load_state() {
        let (_dir, pool) = create_test_pool();

        let state = PersistedState {
            current_track_id: Some(42),
            playback_position: 123.45,
            queue: vec![1, 2, 3, 4, 5],
            queue_index: 2,
            shuffle_order: vec![],
            volume: 0.75,
            shuffle_enabled: true,
            repeat_mode: RepeatMode::All,
        };

        // Save state
        save_state(&pool, &state).unwrap();

        // Load state
        let loaded = load_state(&pool).unwrap();

        assert_eq!(loaded.current_track_id, Some(42));
        assert_eq!(loaded.playback_position, 123.45);
        assert_eq!(loaded.queue, vec![1, 2, 3, 4, 5]);
        assert_eq!(loaded.queue_index, 2);
        assert_eq!(loaded.volume, 0.75);
        assert!(loaded.shuffle_enabled);
        assert_eq!(loaded.repeat_mode, RepeatMode::All);
    }

    #[test]
    fn test_load_default_state() {
        let (_dir, pool) = create_test_pool();

        // Load state when nothing is saved
        let loaded = load_state(&pool).unwrap();

        assert_eq!(loaded.current_track_id, None);
        assert_eq!(loaded.playback_position, 0.0);
        assert!(loaded.queue.is_empty());
        assert_eq!(loaded.volume, 1.0);
        assert!(!loaded.shuffle_enabled);
    }

    #[test]
    fn test_clear_state() {
        let (_dir, pool) = create_test_pool();

        let state = PersistedState {
            current_track_id: Some(99),
            ..Default::default()
        };

        save_state(&pool, &state).unwrap();
        clear_state(&pool).unwrap();

        let loaded = load_state(&pool).unwrap();
        assert_eq!(loaded.current_track_id, None);
    }

    #[test]
    fn test_corrupted_json_returns_default() {
        let (_dir, pool) = create_test_pool();

        // Manually insert corrupted JSON
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO app_state (key, value) VALUES ('playback_state', ?1)",
            ["{ not valid json !!!"],
        ).unwrap();
        drop(conn);

        // Should silently fall back to default instead of panicking
        let loaded = load_state(&pool).unwrap();
        assert_eq!(loaded.current_track_id, None);
        assert_eq!(loaded.volume, 1.0);
    }

    #[test]
    fn test_save_overwrites_previous_state() {
        let (_dir, pool) = create_test_pool();

        let state1 = PersistedState { current_track_id: Some(10), volume: 0.5, ..Default::default() };
        let state2 = PersistedState { current_track_id: Some(20), volume: 0.8, ..Default::default() };

        save_state(&pool, &state1).unwrap();
        save_state(&pool, &state2).unwrap();

        let loaded = load_state(&pool).unwrap();
        assert_eq!(loaded.current_track_id, Some(20));
        assert_eq!(loaded.volume, 0.8);
    }

    #[test]
    fn test_save_and_load_shuffle_order() {
        let (_dir, pool) = create_test_pool();

        let state = PersistedState {
            shuffle_enabled: true,
            shuffle_order: vec![2, 0, 4, 1, 3],
            queue: vec![10, 11, 12, 13, 14],
            ..Default::default()
        };

        save_state(&pool, &state).unwrap();
        let loaded = load_state(&pool).unwrap();

        assert!(loaded.shuffle_enabled);
        assert_eq!(loaded.shuffle_order, vec![2, 0, 4, 1, 3]);
        assert_eq!(loaded.queue, vec![10, 11, 12, 13, 14]);
    }

    #[test]
    fn test_save_and_load_repeat_mode_one() {
        let (_dir, pool) = create_test_pool();

        let state = PersistedState { repeat_mode: RepeatMode::One, ..Default::default() };
        save_state(&pool, &state).unwrap();

        let loaded = load_state(&pool).unwrap();
        assert_eq!(loaded.repeat_mode, RepeatMode::One);
    }

    #[test]
    fn test_clear_state_then_load_returns_default() {
        let (_dir, pool) = create_test_pool();

        // Nothing saved: clear should be a no-op, load should return default
        clear_state(&pool).unwrap();
        let loaded = load_state(&pool).unwrap();
        assert_eq!(loaded.current_track_id, None);
        assert!(!loaded.shuffle_enabled);
    }

    #[test]
    fn test_save_large_queue() {
        let (_dir, pool) = create_test_pool();

        let large_queue: Vec<i64> = (1..=1000).collect();
        let state = PersistedState {
            queue: large_queue.clone(),
            queue_index: 500,
            ..Default::default()
        };

        save_state(&pool, &state).unwrap();
        let loaded = load_state(&pool).unwrap();

        assert_eq!(loaded.queue.len(), 1000);
        assert_eq!(loaded.queue_index, 500);
        assert_eq!(loaded.queue, large_queue);
    }
}
