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

    // Migration 5: Collapse multi-artist ARTIST tag entries into their primary artist.
    //
    // Before this fix, a track tagged ARTIST="Akhenaton, Disiz la Peste" created a single
    // artist row with that full string. We now keep only the primary artist (first element).
    //
    // Heuristic: split on ", " only when no resulting part contains " & " or " and ",
    // which preserves legitimate band names like "Earth, Wind & Fire".
    if current_version < 5 {
        // Collect all artists whose name looks like a comma-separated list
        let multi_artists: Vec<(i64, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, name FROM artists WHERE INSTR(name, ', ') > 0",
            )?;
            stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };

        for (multi_id, multi_name) in multi_artists {
            let parts: Vec<&str> = multi_name.split(", ").collect();

            // Skip if any part contains " & " or " and " → likely a band name
            let is_list = parts.iter().all(|p| {
                let lower = p.to_lowercase();
                !lower.contains(" & ") && !lower.contains(" and ")
            });
            if !is_list {
                continue;
            }

            let primary_name = parts[0].trim();

            // Find or create the primary artist row
            let primary_id: i64 = match conn.query_row(
                "SELECT id FROM artists WHERE name = ?1",
                [primary_name],
                |r| r.get(0),
            ) {
                Ok(id) => id,
                Err(_) => {
                    conn.execute(
                        "INSERT INTO artists (name) VALUES (?1)",
                        [primary_name],
                    )?;
                    conn.last_insert_rowid()
                }
            };

            if primary_id == multi_id {
                continue;
            }

            // Remap tracks and albums that point to the multi-artist entry
            conn.execute(
                "UPDATE tracks SET artist_id = ?1 WHERE artist_id = ?2",
                [primary_id, multi_id],
            )?;
            conn.execute(
                "UPDATE albums SET artist_id = ?1 WHERE artist_id = ?2",
                [primary_id, multi_id],
            )?;

            // Remove the now-orphaned multi-artist row
            conn.execute("DELETE FROM artists WHERE id = ?1", [multi_id])?;
        }

        conn.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [5],
        )?;
    }

    // Migration 6: Assign "Various Artists" to compilation albums.
    //
    // Albums that have tracks from more than one distinct artist (and no explicit
    // ALBUMARTIST tag to override this) ended up arbitrarily owned by whichever
    // artist was imported first. This migration corrects that by finding every album
    // whose tracks span multiple distinct artists and re-pointing artist_id to a
    // shared "Various Artists" row.
    if current_version < 6 {
        // Only act if there are multi-artist albums to fix
        let has_multi_artist: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM (
                    SELECT album_id FROM tracks
                     WHERE album_id IS NOT NULL
                     GROUP BY album_id
                    HAVING COUNT(DISTINCT artist_id) > 1
                 )",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);

        if has_multi_artist {
            // Find or create the "Various Artists" placeholder
            conn.execute(
                "INSERT OR IGNORE INTO artists (name) VALUES ('Various Artists')",
                [],
            )?;
            let va_id: i64 = conn.query_row(
                "SELECT id FROM artists WHERE name = 'Various Artists'",
                [],
                |r| r.get(0),
            )?;

            conn.execute(
                "UPDATE albums SET artist_id = ?1
                  WHERE id IN (
                      SELECT album_id FROM tracks
                       WHERE album_id IS NOT NULL
                       GROUP BY album_id
                      HAVING COUNT(DISTINCT artist_id) > 1
                  )",
                [va_id],
            )?;
        }

        conn.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [6],
        )?;
    }

    // Migration 7: Add rich artist metadata from TheAudioDB
    if current_version < 7 {
        conn.execute_batch(
            "ALTER TABLE artists ADD COLUMN country TEXT;
             ALTER TABLE artists ADD COLUMN genre TEXT;
             ALTER TABLE artists ADD COLUMN style TEXT;
             ALTER TABLE artists ADD COLUMN mood TEXT;
             ALTER TABLE artists ADD COLUMN formed_year INTEGER;
             ALTER TABLE artists ADD COLUMN born_year INTEGER;
             ALTER TABLE artists ADD COLUMN died_year INTEGER;
             ALTER TABLE artists ADD COLUMN disbanded TEXT;
             ALTER TABLE artists ADD COLUMN musicbrainz_id TEXT;
             ALTER TABLE artists ADD COLUMN theaudiodb_id TEXT;",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [7])?;
    }

    Ok(())
}
