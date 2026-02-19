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

    // Migration 4: Merge duplicate album entries caused by missing ALBUMARTIST tags.
    //
    // Before the ALBUMARTIST fix, compilation/mixtape tracks without an ALBUMARTIST
    // tag created one album row per unique track artist (e.g. "70s Mixtape / The Who"
    // and "70s Mixtape / Stevie Wonder" as separate albums).
    //
    // This migration:
    //   1. For each group of albums sharing the same title, picks the one with the
    //      most tracks as the canonical album (winner).
    //   2. Re-points all tracks from the other duplicates to the winner.
    //   3. Deletes the now-empty duplicate album rows.
    if current_version < 4 {
        conn.execute_batch(
            "
            -- For every duplicate title group, find the album_id with the most tracks (winner).
            -- Then reparent all tracks from other albums with the same title to the winner.
            UPDATE tracks
               SET album_id = (
                       SELECT a2.id
                         FROM albums a2
                        WHERE a2.title = (SELECT title FROM albums WHERE id = tracks.album_id)
                        ORDER BY (SELECT COUNT(*) FROM tracks t2 WHERE t2.album_id = a2.id) DESC,
                                 a2.id ASC
                        LIMIT 1
                   )
             WHERE album_id IS NOT NULL;

            -- Delete albums that now have no tracks and are duplicates of another album
            -- with the same title (i.e. they lost the winner election above).
            DELETE FROM albums
             WHERE id NOT IN (SELECT DISTINCT album_id FROM tracks WHERE album_id IS NOT NULL)
               AND title IN (
                       SELECT title FROM albums
                       GROUP BY title HAVING COUNT(*) > 1
                   );
            ",
        )?;
        conn.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [4],
        )?;
    }

    Ok(())
}
