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

    // Migration 3: Add MusicBrainz metadata fields + rename artwork files to MBID-based names.
    //
    // Artwork files previously used {album_db_id}.jpg naming which breaks across
    // rescans and DB resets. New naming is {musicbrainz_release_id}.jpg.
    //
    // This migration:
    //   1. Adds MB metadata columns to albums
    //   2. Clears online_artwork_path so albums are re-scraped with correct naming
    //   3. Sets a flag so ArtworkService deletes the old {integer}.jpg files on next run
    if current_version < 3 {
        conn.execute_batch(
            "ALTER TABLE albums ADD COLUMN musicbrainz_id TEXT;
             ALTER TABLE albums ADD COLUMN label TEXT;
             ALTER TABLE albums ADD COLUMN country TEXT;
             ALTER TABLE albums ADD COLUMN barcode TEXT;
             ALTER TABLE albums ADD COLUMN album_type TEXT;
             ALTER TABLE albums ADD COLUMN release_status TEXT;

             UPDATE albums
                SET online_artwork_path = NULL,
                    artwork_source = NULL,
                    artwork_fetched_at = NULL
              WHERE online_artwork_path IS NOT NULL;

             INSERT OR IGNORE INTO app_state (key, value)
             VALUES ('pending_legacy_artwork_cleanup', 'true');",
        )?;
        conn.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [3],
        )?;
    }

    Ok(())
}
