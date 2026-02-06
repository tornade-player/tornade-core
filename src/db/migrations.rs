// Database migrations

use rusqlite::{Connection, Result};

/// Run database migrations
pub fn run_migrations(conn: &Connection) -> Result<()> {
    // Create migrations table if it doesn't exist
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    // Get current version
    let current_version: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // Migration 1: Initial schema (already applied via schema.rs)
    if current_version < 1 {
        conn.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [1],
        )?;
    }

    // Migration 2: Add online artwork support
    if current_version < 2 {
        conn.execute_batch(
            "ALTER TABLE albums ADD COLUMN online_artwork_path TEXT;
             ALTER TABLE albums ADD COLUMN artwork_source TEXT;
             ALTER TABLE albums ADD COLUMN artwork_fetched_at DATETIME;

             ALTER TABLE artists ADD COLUMN photo_path TEXT;
             ALTER TABLE artists ADD COLUMN photo_source TEXT;
             ALTER TABLE artists ADD COLUMN photo_fetched_at DATETIME;",
        )?;
        conn.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [2],
        )?;
    }

    Ok(())
}
