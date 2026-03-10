//! Incremental database migrations applied on top of the base schema.
//!
//! Each migration is guarded by a `schema_migrations` table so it only runs once,
//! even if [`run_migrations`] is called multiple times on the same database.

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
        conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [1])?;
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
        conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [2])?;
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
        conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [3])?;
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
        conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [4])?;
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
            let mut stmt =
                conn.prepare("SELECT id, name FROM artists WHERE INSTR(name, ', ') > 0")?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
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
            let primary_id: i64 = if let Ok(id) = conn.query_row(
                "SELECT id FROM artists WHERE name = ?1",
                [primary_name],
                |r| r.get(0),
            ) {
                id
            } else {
                conn.execute("INSERT INTO artists (name) VALUES (?1)", [primary_name])?;
                conn.last_insert_rowid()
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

        conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [5])?;
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

        conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [6])?;
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

    // Migration 8: Create track_artists junction table and seed from existing data.
    //
    // Before this migration, secondary artists in composite ARTIST tags (ft/feat/avec/vs)
    // were discarded at scan time. This migration:
    //   1. Creates the track_artists junction table.
    //   2. Seeds it from the existing tracks.artist_id column (primary artist, position=0).
    //   3. Finds any remaining composite artist rows (ft/feat/avec/vs patterns that
    //      migration 5 did not handle — it only handled comma-separated names) and
    //      splits them: updates tracks/albums to point to the primary artist, removes
    //      the composite ghost row.
    if current_version < 8 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS track_artists (
                track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
                position INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (track_id, artist_id)
            );
            CREATE INDEX IF NOT EXISTS idx_track_artists_artist ON track_artists(artist_id);
            INSERT OR IGNORE INTO track_artists (track_id, artist_id, position)
            SELECT id, artist_id, 0 FROM tracks;",
        )?;

        // Fix remaining composite artists (ft/feat/avec/vs) missed by migration 5
        let composite_artists: Vec<(i64, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, name FROM artists WHERE
                 LOWER(name) LIKE '% ft %' OR LOWER(name) LIKE '% ft. %' OR
                 LOWER(name) LIKE '% feat %' OR LOWER(name) LIKE '% feat. %' OR
                 LOWER(name) LIKE '% featuring %' OR LOWER(name) LIKE '% avec %' OR
                 LOWER(name) LIKE '% vs %' OR LOWER(name) LIKE '% vs. %'",
            )?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        for (composite_id, composite_name) in composite_artists {
            let parts = crate::services::library::split_artists(&composite_name);
            if parts.len() <= 1 {
                continue;
            }

            let primary_name = &parts[0];
            let primary_id: i64 = if let Ok(id) = conn.query_row(
                "SELECT id FROM artists WHERE name = ?1",
                [primary_name],
                |r| r.get(0),
            ) {
                id
            } else {
                conn.execute("INSERT INTO artists (name) VALUES (?1)", [primary_name])?;
                conn.last_insert_rowid()
            };

            if primary_id != composite_id {
                conn.execute(
                    "UPDATE tracks SET artist_id = ?1 WHERE artist_id = ?2",
                    [primary_id, composite_id],
                )?;
                conn.execute(
                    "UPDATE albums SET artist_id = ?1 WHERE artist_id = ?2",
                    [primary_id, composite_id],
                )?;
                conn.execute(
                    "UPDATE track_artists SET artist_id = ?1 WHERE artist_id = ?2",
                    [primary_id, composite_id],
                )?;
                conn.execute("DELETE FROM artists WHERE id = ?1", [composite_id])?;
            }
        }

        conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [8])?;
    }

    // Migration 9: Remove "Various Artists" — assign the dominant track artist to each
    // compilation album so that the fictional "Various Artists" entity disappears.
    if current_version < 9 {
        apply_migration_9_logic(conn)?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [9])?;
    }

    // Migration 10: Parse track titles for feat markers and link the extracted artists
    // in track_artists. Recovers featuring artists that are only present in the title
    // (e.g. "Titanium (feat. Sia)") and were never stored in the ARTIST tag.
    if current_version < 10 {
        apply_migration_10_logic(conn)?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [10])?;
    }

    // Migration 11: Add FTS5 virtual tables for albums and artists to enable
    // prefix search and fuzzy fallback on those entity types.
    // The tables are created via initialize_fts() (idempotent), so this migration
    // only needs to back-fill existing data.
    if current_version < 11 {
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS albums_fts USING fts5(
                title,
                artist_name,
                content='',
                tokenize='unicode61 remove_diacritics 1'
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS artists_fts USING fts5(
                name,
                content='',
                tokenize='unicode61 remove_diacritics 1'
            );

            INSERT OR IGNORE INTO albums_fts(rowid, title, artist_name)
            SELECT al.id, al.title, ar.name
            FROM albums al
            JOIN artists ar ON ar.id = al.artist_id;

            INSERT OR IGNORE INTO artists_fts(rowid, name)
            SELECT id, name FROM artists;",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [11])?;
    }

    Ok(())
}

fn apply_migration_9_logic(conn: &Connection) -> Result<()> {
    use rusqlite::OptionalExtension;

    // Nothing to do if Various Artists was never created
    let va_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM artists WHERE name = 'Various Artists'",
            [],
            |r| r.get(0),
        )
        .optional()?;

    let Some(va_id) = va_id else {
        return Ok(());
    };

    // Collect all albums owned by Various Artists
    let va_albums: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM albums WHERE artist_id = ?1")?;
        stmt.query_map([va_id], |r| r.get(0))?
            .collect::<Result<Vec<_>>>()?
    };

    for album_id in va_albums {
        // Pick the artist with the most tracks on this album (primary artist, position=0)
        let dominant: Option<i64> = conn
            .query_row(
                "SELECT t.artist_id FROM tracks t
                 WHERE t.album_id = ?1
                 GROUP BY t.artist_id
                 ORDER BY COUNT(*) DESC
                 LIMIT 1",
                [album_id],
                |r| r.get(0),
            )
            .optional()?;

        if let Some(dominant_id) = dominant {
            conn.execute(
                "UPDATE albums SET artist_id = ?1 WHERE id = ?2",
                [dominant_id, album_id],
            )?;
        }
    }

    // Delete Various Artists only if it is no longer referenced anywhere
    conn.execute(
        "DELETE FROM artists WHERE id = ?1
         AND id NOT IN (SELECT DISTINCT artist_id FROM albums)
         AND id NOT IN (SELECT DISTINCT artist_id FROM tracks)
         AND id NOT IN (SELECT DISTINCT artist_id FROM track_artists)",
        [va_id],
    )?;

    Ok(())
}

fn apply_migration_10_logic(conn: &Connection) -> Result<()> {
    // Load all tracks once — titles can be large but are bounded in practice.
    let tracks: Vec<(i64, String)> = {
        let mut stmt = conn.prepare("SELECT id, title FROM tracks")?;
        stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>>>()?
    };

    for (track_id, title) in tracks {
        let feat_artists =
            crate::services::library::extract_feat_from_title(&title);

        for name in feat_artists {
            // Find or create the artist row.
            let artist_id: i64 = match conn.query_row(
                "SELECT id FROM artists WHERE name = ?1",
                [&name],
                |r| r.get(0),
            ) {
                Ok(id) => id,
                Err(_) => {
                    conn.execute(
                        "INSERT INTO artists (name) VALUES (?1)",
                        [&name],
                    )?;
                    conn.last_insert_rowid()
                }
            };

            // INSERT OR IGNORE: primary key is (track_id, artist_id), so if the artist
            // was already linked via the ARTIST tag, this is a no-op.
            conn.execute(
                "INSERT OR IGNORE INTO track_artists (track_id, artist_id, position)
                 VALUES (?1, ?2, (
                     SELECT COALESCE(MAX(position), -1) + 1
                     FROM track_artists WHERE track_id = ?1
                 ))",
                rusqlite::params![track_id, artist_id],
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::db::queries;
    use crate::models::AudioFormat;
    use crate::models::source::SourceType;
    use crate::test_helpers::TestEnv;
    use rusqlite::{Connection, OptionalExtension};
    use std::path::PathBuf;

    // ── Helpers that replay the migration logic on a live connection ──────
    //
    // We test each migration's logic in isolation by running its SQL/Rust
    // directly against a freshly seeded TestEnv rather than via
    // run_migrations() (which would be a no-op since the DB is already at
    // the latest version).

    fn apply_migration_4_sql(conn: &Connection) {
        conn.execute_batch(
            "UPDATE tracks
                SET album_id = (
                    SELECT a2.id FROM albums a2
                     WHERE a2.title = (SELECT title FROM albums WHERE id = tracks.album_id)
                     ORDER BY (SELECT COUNT(*) FROM tracks t2 WHERE t2.album_id = a2.id) DESC,
                              a2.id ASC
                     LIMIT 1
                )
              WHERE album_id IS NOT NULL;

             DELETE FROM albums
              WHERE id NOT IN (SELECT DISTINCT album_id FROM tracks WHERE album_id IS NOT NULL)
                AND title IN (
                    SELECT title FROM albums
                    GROUP BY title HAVING COUNT(*) > 1
                );",
        )
        .unwrap();
    }

    fn apply_migration_5_logic(conn: &Connection) {
        let multi_artists: Vec<(i64, String)> = {
            let mut stmt = conn
                .prepare("SELECT id, name FROM artists WHERE INSTR(name, ', ') > 0")
                .unwrap();
            stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
        };

        for (multi_id, multi_name) in multi_artists {
            let parts: Vec<&str> = multi_name.split(", ").collect();
            let is_list = parts.iter().all(|p| {
                let lower = p.to_lowercase();
                !lower.contains(" & ") && !lower.contains(" and ")
            });
            if !is_list {
                continue;
            }

            let primary_name = parts[0].trim();
            let primary_id: i64 = match conn.query_row(
                "SELECT id FROM artists WHERE name = ?1",
                [primary_name],
                |r| r.get(0),
            ) {
                Ok(id) => id,
                Err(_) => {
                    conn.execute("INSERT INTO artists (name) VALUES (?1)", [primary_name])
                        .unwrap();
                    conn.last_insert_rowid()
                }
            };

            if primary_id == multi_id {
                continue;
            }

            conn.execute(
                "UPDATE tracks SET artist_id = ?1 WHERE artist_id = ?2",
                [primary_id, multi_id],
            )
            .unwrap();
            conn.execute(
                "UPDATE albums SET artist_id = ?1 WHERE artist_id = ?2",
                [primary_id, multi_id],
            )
            .unwrap();
            conn.execute("DELETE FROM artists WHERE id = ?1", [multi_id])
                .unwrap();
        }
    }

    fn apply_migration_6_logic(conn: &Connection) {
        let has_multi: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM (
                 SELECT album_id FROM tracks WHERE album_id IS NOT NULL
                 GROUP BY album_id HAVING COUNT(DISTINCT artist_id) > 1
             )",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);

        if !has_multi {
            return;
        }

        conn.execute(
            "INSERT OR IGNORE INTO artists (name) VALUES ('Various Artists')",
            [],
        )
        .unwrap();
        let va_id: i64 = conn
            .query_row(
                "SELECT id FROM artists WHERE name = 'Various Artists'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        conn.execute(
            "UPDATE albums SET artist_id = ?1
              WHERE id IN (
                  SELECT album_id FROM tracks WHERE album_id IS NOT NULL
                  GROUP BY album_id HAVING COUNT(DISTINCT artist_id) > 1
              )",
            [va_id],
        )
        .unwrap();
    }

    // ── Migration 4: duplicate-album deduplication ────────────────────────

    #[test]
    fn test_migration_4_reparents_all_tracks_to_winner_album() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist1 = queries::insert_artist(&conn, "Artist1", None).unwrap();
        let artist2 = queries::insert_artist(&conn, "Artist2", None).unwrap();

        // Pre-migration state: same title, different artist_id → allowed by UNIQUE(title,artist_id)
        let album_a = queries::insert_album(&conn, "Mixtape", artist1, None).unwrap(); // 1 track
        let album_b = queries::insert_album(&conn, "Mixtape", artist2, None).unwrap(); // 3 tracks → winner

        queries::insert_track(
            &conn,
            "S1",
            Some(album_a),
            artist1,
            source,
            &PathBuf::from("/a/1.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "S2",
            Some(album_b),
            artist2,
            source,
            &PathBuf::from("/b/2.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "S3",
            Some(album_b),
            artist2,
            source,
            &PathBuf::from("/b/3.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "S4",
            Some(album_b),
            artist2,
            source,
            &PathBuf::from("/b/4.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        apply_migration_4_sql(&conn);

        let all_album_ids: Vec<Option<i64>> = {
            let mut stmt = conn
                .prepare("SELECT album_id FROM tracks ORDER BY id")
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert!(
            all_album_ids.iter().all(|&id| id == Some(album_b)),
            "all tracks must be reparented to the album with the most tracks"
        );
    }

    #[test]
    fn test_migration_4_deletes_loser_album() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist1 = queries::insert_artist(&conn, "A1", None).unwrap();
        let artist2 = queries::insert_artist(&conn, "A2", None).unwrap();

        let album_a = queries::insert_album(&conn, "Shared Title", artist1, None).unwrap();
        let album_b = queries::insert_album(&conn, "Shared Title", artist2, None).unwrap();
        queries::insert_track(
            &conn,
            "T1",
            Some(album_a),
            artist1,
            source,
            &PathBuf::from("/a.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "T2",
            Some(album_b),
            artist2,
            source,
            &PathBuf::from("/b1.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "T3",
            Some(album_b),
            artist2,
            source,
            &PathBuf::from("/b2.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        apply_migration_4_sql(&conn);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM albums WHERE title = 'Shared Title'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "loser album must be deleted, leaving only the winner"
        );
    }

    #[test]
    fn test_migration_4_unique_album_is_untouched() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist = queries::insert_artist(&conn, "Solo", None).unwrap();
        let album = queries::insert_album(&conn, "Unique Album", artist, None).unwrap();
        queries::insert_track(
            &conn,
            "T",
            Some(album),
            artist,
            source,
            &PathBuf::from("/t.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        apply_migration_4_sql(&conn);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM albums", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "album with a unique title must not be touched");
        let album_id: i64 = conn
            .query_row("SELECT album_id FROM tracks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            album_id, album,
            "track must still reference the original album"
        );
    }

    // ── Migration 5: multi-artist artist-tag collapse ─────────────────────

    #[test]
    fn test_migration_5_splits_comma_artist_into_primary() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();

        conn.execute("INSERT INTO artists (name) VALUES ('Akhenaton, Disiz')", [])
            .unwrap();
        let multi_id: i64 = conn.last_insert_rowid();
        let album = queries::insert_album(&conn, "Collab", multi_id, None).unwrap();
        queries::insert_track(
            &conn,
            "T",
            Some(album),
            multi_id,
            source,
            &PathBuf::from("/t.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        apply_migration_5_logic(&conn);

        let primary_id: Option<i64> = conn
            .query_row("SELECT id FROM artists WHERE name = 'Akhenaton'", [], |r| {
                r.get(0)
            })
            .optional()
            .unwrap();
        assert!(
            primary_id.is_some(),
            "primary artist 'Akhenaton' must be created"
        );

        let track_artist: i64 = conn
            .query_row("SELECT artist_id FROM tracks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            track_artist,
            primary_id.unwrap(),
            "track must point to the primary artist"
        );

        let multi_gone: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artists WHERE name = 'Akhenaton, Disiz'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(multi_gone, 0, "multi-artist row must be deleted");
    }

    #[test]
    fn test_migration_5_preserves_band_name_with_ampersand() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();

        conn.execute(
            "INSERT INTO artists (name) VALUES ('Earth, Wind & Fire')",
            [],
        )
        .unwrap();
        let band_id: i64 = conn.last_insert_rowid();
        queries::insert_track(
            &conn,
            "T",
            None,
            band_id,
            source,
            &PathBuf::from("/t.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        apply_migration_5_logic(&conn);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artists WHERE name = 'Earth, Wind & Fire'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "band name containing '&' must not be split");
    }

    #[test]
    fn test_migration_5_reuses_existing_primary_artist_row() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();

        let jayz_id = queries::insert_artist(&conn, "Jay-Z", None).unwrap();
        conn.execute(
            "INSERT INTO artists (name) VALUES ('Jay-Z, Kanye West')",
            [],
        )
        .unwrap();
        let multi_id: i64 = conn.last_insert_rowid();
        queries::insert_track(
            &conn,
            "T",
            None,
            multi_id,
            source,
            &PathBuf::from("/t.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        apply_migration_5_logic(&conn);

        let track_artist: i64 = conn
            .query_row("SELECT artist_id FROM tracks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            track_artist, jayz_id,
            "must reuse the existing Jay-Z row, not create a new one"
        );

        let jayz_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artists WHERE name = 'Jay-Z'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(jayz_count, 1, "must not create a duplicate Jay-Z row");
    }

    #[test]
    fn test_migration_5_preserves_single_name_artist() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist = queries::insert_artist(&conn, "Adele", None).unwrap();
        queries::insert_track(
            &conn,
            "T",
            None,
            artist,
            source,
            &PathBuf::from("/t.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        apply_migration_5_logic(&conn);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artists WHERE name = 'Adele'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "artist with a single name must be untouched");
    }

    // ── Migration 6: Various Artists assignment ───────────────────────────

    #[test]
    fn test_migration_6_assigns_various_artists_to_multi_artist_album() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist1 = queries::insert_artist(&conn, "Artist1", None).unwrap();
        let artist2 = queries::insert_artist(&conn, "Artist2", None).unwrap();
        let album = queries::insert_album(&conn, "Compilation", artist1, None).unwrap();

        queries::insert_track(
            &conn,
            "T1",
            Some(album),
            artist1,
            source,
            &PathBuf::from("/t1.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "T2",
            Some(album),
            artist2,
            source,
            &PathBuf::from("/t2.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        apply_migration_6_logic(&conn);

        let va_id: i64 = conn
            .query_row(
                "SELECT id FROM artists WHERE name = 'Various Artists'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let album_artist: i64 = conn
            .query_row("SELECT artist_id FROM albums WHERE id = ?1", [album], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            album_artist, va_id,
            "compilation album must be assigned to Various Artists"
        );
    }

    #[test]
    fn test_migration_6_preserves_single_artist_album() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist = queries::insert_artist(&conn, "Solo Artist", None).unwrap();
        let album = queries::insert_album(&conn, "Solo Album", artist, None).unwrap();

        queries::insert_track(
            &conn,
            "T1",
            Some(album),
            artist,
            source,
            &PathBuf::from("/t1.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "T2",
            Some(album),
            artist,
            source,
            &PathBuf::from("/t2.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        apply_migration_6_logic(&conn);

        let album_artist: i64 = conn
            .query_row("SELECT artist_id FROM albums WHERE id = ?1", [album], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            album_artist, artist,
            "single-artist album must keep its original artist"
        );

        let va_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artists WHERE name = 'Various Artists'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            va_count, 0,
            "Various Artists must not be created when there are no compilations"
        );
    }

    #[test]
    fn test_migration_6_uses_existing_various_artists_row() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist1 = queries::insert_artist(&conn, "Artist1", None).unwrap();
        let artist2 = queries::insert_artist(&conn, "Artist2", None).unwrap();
        let va_id = queries::insert_artist(&conn, "Various Artists", None).unwrap();
        let album = queries::insert_album(&conn, "Mix", artist1, None).unwrap();

        queries::insert_track(
            &conn,
            "T1",
            Some(album),
            artist1,
            source,
            &PathBuf::from("/t1.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        queries::insert_track(
            &conn,
            "T2",
            Some(album),
            artist2,
            source,
            &PathBuf::from("/t2.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        apply_migration_6_logic(&conn);

        let va_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artists WHERE name = 'Various Artists'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            va_count, 1,
            "must not create a duplicate Various Artists row"
        );

        let album_artist: i64 = conn
            .query_row("SELECT artist_id FROM albums WHERE id = ?1", [album], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            album_artist, va_id,
            "must use the pre-existing Various Artists ID"
        );
    }

    // ── Migration 8 helpers ───────────────────────────────────────────────

    fn apply_migration_8_logic(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS track_artists (
                track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
                position INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (track_id, artist_id)
            );
            CREATE INDEX IF NOT EXISTS idx_track_artists_artist ON track_artists(artist_id);
            INSERT OR IGNORE INTO track_artists (track_id, artist_id, position)
            SELECT id, artist_id, 0 FROM tracks;",
        )
        .unwrap();

        let composite_artists: Vec<(i64, String)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name FROM artists WHERE
                     LOWER(name) LIKE '% ft %' OR LOWER(name) LIKE '% ft. %' OR
                     LOWER(name) LIKE '% feat %' OR LOWER(name) LIKE '% feat. %' OR
                     LOWER(name) LIKE '% featuring %' OR LOWER(name) LIKE '% avec %' OR
                     LOWER(name) LIKE '% vs %' OR LOWER(name) LIKE '% vs. %'",
                )
                .unwrap();
            stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
        };

        for (composite_id, composite_name) in composite_artists {
            let parts = crate::services::library::split_artists(&composite_name);
            if parts.len() <= 1 {
                continue;
            }

            let primary_name = &parts[0];
            let primary_id: i64 = match conn.query_row(
                "SELECT id FROM artists WHERE name = ?1",
                [primary_name],
                |r| r.get(0),
            ) {
                Ok(id) => id,
                Err(_) => {
                    conn.execute("INSERT INTO artists (name) VALUES (?1)", [primary_name])
                        .unwrap();
                    conn.last_insert_rowid()
                }
            };

            if primary_id != composite_id {
                conn.execute(
                    "UPDATE tracks SET artist_id = ?1 WHERE artist_id = ?2",
                    [primary_id, composite_id],
                )
                .unwrap();
                conn.execute(
                    "UPDATE albums SET artist_id = ?1 WHERE artist_id = ?2",
                    [primary_id, composite_id],
                )
                .unwrap();
                conn.execute(
                    "UPDATE track_artists SET artist_id = ?1 WHERE artist_id = ?2",
                    [primary_id, composite_id],
                )
                .unwrap();
                conn.execute("DELETE FROM artists WHERE id = ?1", [composite_id])
                    .unwrap();
            }
        }
    }

    // ── Migration 8: track_artists table ─────────────────────────────────

    #[test]
    fn test_migration_8_creates_track_artists_table() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();

        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='track_artists'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(exists, "track_artists table must exist after migration 8");
    }

    #[test]
    fn test_migration_8_seeds_from_existing_tracks() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist = queries::insert_artist(&conn, "Adele", None).unwrap();
        let track_id = queries::insert_track(
            &conn,
            "Hello",
            None,
            artist,
            source,
            &PathBuf::from("/hello.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        // The migration already ran as part of initialize_database in TestEnv::new().
        // The track was inserted AFTER the migration, so track_artists won't have it.
        // We manually apply seeding to test the logic:
        conn.execute(
            "INSERT OR IGNORE INTO track_artists (track_id, artist_id, position)
             SELECT id, artist_id, 0 FROM tracks WHERE id = ?1",
            [track_id],
        )
        .unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM track_artists WHERE track_id = ?1 AND artist_id = ?2 AND position = 0",
                [track_id, artist],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "seeding must create a position=0 entry");
    }

    #[test]
    fn test_migration_8_fixes_composite_ft_artist() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();

        // Insert a composite artist that migration 5 would not have split (no comma)
        conn.execute(
            "INSERT INTO artists (name) VALUES ('Doc Gynéco ft El maestro')",
            [],
        )
        .unwrap();
        let composite_id: i64 = conn.last_insert_rowid();
        let track_id = queries::insert_track(
            &conn,
            "T",
            None,
            composite_id,
            source,
            &PathBuf::from("/t.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        // Seed the track_artists table as migration 8 would
        conn.execute(
            "INSERT OR IGNORE INTO track_artists (track_id, artist_id, position)
             SELECT id, artist_id, 0 FROM tracks",
            [],
        )
        .unwrap();

        apply_migration_8_logic(&conn);

        // Composite artist must be gone
        let composite_gone: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artists WHERE name = 'Doc Gynéco ft El maestro'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(composite_gone, 0, "composite ft artist must be removed");

        // Primary artist must exist
        let primary_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artists WHERE name = 'Doc Gynéco'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(primary_exists, 1, "primary artist must be created");

        // Track must point to primary artist
        let track_artist: i64 = conn
            .query_row("SELECT artist_id FROM tracks WHERE id = ?1", [track_id], |r| r.get(0))
            .unwrap();
        let primary_id: i64 = conn
            .query_row("SELECT id FROM artists WHERE name = 'Doc Gynéco'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(track_artist, primary_id);
    }

    // ── Migration 9: Remove Various Artists ───────────────────────────────

    fn apply_migration_9_logic_test(conn: &Connection) {
        super::apply_migration_9_logic(conn).unwrap();
    }

    #[test]
    fn test_migration_9_assigns_dominant_artist_to_va_album() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();

        let va_id = queries::insert_artist(&conn, "Various Artists", None).unwrap();
        let artist1 = queries::insert_artist(&conn, "Akhenaton", None).unwrap();
        let artist2 = queries::insert_artist(&conn, "Soprano", None).unwrap();
        let album = queries::insert_album(&conn, "Compilation", va_id, None).unwrap();

        // artist1 has 2 tracks, artist2 has 1 → artist1 is dominant
        for path in ["/t1.flac", "/t2.flac"] {
            queries::insert_track(
                &conn, "T", Some(album), artist1, source,
                &PathBuf::from(path), 60_000, None, None, None,
                AudioFormat::Flac, 1_000_000,
            ).unwrap();
        }
        queries::insert_track(
            &conn, "T3", Some(album), artist2, source,
            &PathBuf::from("/t3.flac"), 60_000, None, None, None,
            AudioFormat::Flac, 1_000_000,
        ).unwrap();

        apply_migration_9_logic_test(&conn);

        let album_artist: i64 = conn
            .query_row("SELECT artist_id FROM albums WHERE id = ?1", [album], |r| r.get(0))
            .unwrap();
        assert_eq!(album_artist, artist1, "dominant artist must become the album artist");
    }

    #[test]
    fn test_migration_9_deletes_various_artists() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();

        let va_id = queries::insert_artist(&conn, "Various Artists", None).unwrap();
        let artist = queries::insert_artist(&conn, "Oxmo Puccino", None).unwrap();
        let album = queries::insert_album(&conn, "Mix", va_id, None).unwrap();
        queries::insert_track(
            &conn, "T", Some(album), artist, source,
            &PathBuf::from("/t.flac"), 60_000, None, None, None,
            AudioFormat::Flac, 1_000_000,
        ).unwrap();

        apply_migration_9_logic_test(&conn);

        let va_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM artists WHERE name = 'Various Artists'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(va_count, 0, "Various Artists artist must be deleted");
    }

    #[test]
    fn test_migration_9_noop_when_no_various_artists() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist = queries::insert_artist(&conn, "Adele", None).unwrap();
        let album = queries::insert_album(&conn, "21", artist, None).unwrap();
        queries::insert_track(
            &conn, "T", Some(album), artist, source,
            &PathBuf::from("/t.flac"), 60_000, None, None, None,
            AudioFormat::Flac, 1_000_000,
        ).unwrap();

        // Must not panic
        apply_migration_9_logic_test(&conn);

        let artist_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM artists", [], |r| r.get(0))
            .unwrap();
        assert_eq!(artist_count, 1, "unrelated artist must be untouched");
    }

    // ── extract_feat_from_title unit tests ────────────────────────────────

    #[test]
    fn test_extract_feat_from_title_parentheses_feat_dot() {
        let result = crate::services::library::extract_feat_from_title("Titanium (feat. Sia)");
        assert_eq!(result, vec!["Sia"]);
    }

    #[test]
    fn test_extract_feat_from_title_brackets_ft() {
        let result = crate::services::library::extract_feat_from_title("Diamond [ft. Rihanna]");
        assert_eq!(result, vec!["Rihanna"]);
    }

    #[test]
    fn test_extract_feat_from_title_multiple_artists_ampersand() {
        let result = crate::services::library::extract_feat_from_title(
            "Avf (avec OrelSan & Maitre Gims)",
        );
        assert_eq!(result, vec!["OrelSan", "Maitre Gims"]);
    }

    #[test]
    fn test_extract_feat_from_title_no_feat_returns_empty() {
        let result = crate::services::library::extract_feat_from_title("Normal Title");
        assert!(result.is_empty());
    }

    #[test]
    fn test_extract_feat_from_title_case_insensitive() {
        let result =
            crate::services::library::extract_feat_from_title("Track (FEAT. Artist Name)");
        assert_eq!(result, vec!["Artist Name"]);
    }

    // ── Migration 10 tests ────────────────────────────────────────────────

    fn apply_migration_10_logic_test(conn: &Connection) {
        super::apply_migration_10_logic(conn).unwrap();
    }

    #[test]
    fn test_migration_10_links_feat_artist_from_title() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let guetta = queries::insert_artist(&conn, "David Guetta", None).unwrap();
        let track_id = queries::insert_track(
            &conn,
            "Titanium (feat. Sia)",
            None,
            guetta,
            source,
            &PathBuf::from("/t.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        // Seed track_artists with the primary artist (position 0)
        queries::link_track_artist(&conn, track_id, guetta, 0).unwrap();

        apply_migration_10_logic_test(&conn);

        // Sia must now exist as an artist
        let sia_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM artists WHERE name = 'Sia'",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert!(sia_id.is_some(), "Sia must be created as an artist");

        // Sia must be linked to the track
        let linked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM track_artists WHERE track_id = ?1 AND artist_id = ?2",
                rusqlite::params![track_id, sia_id.unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(linked, 1, "Sia must be linked via track_artists");
    }

    #[test]
    fn test_migration_10_skips_track_with_no_feat() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let adele = queries::insert_artist(&conn, "Adele", None).unwrap();
        queries::insert_track(
            &conn,
            "Rolling in the Deep",
            None,
            adele,
            source,
            &PathBuf::from("/t.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        apply_migration_10_logic_test(&conn);

        // No new artists should be created
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM artists", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "no new artists must be created for a plain title");
    }

    #[test]
    fn test_migration_10_does_not_duplicate_already_linked_artist() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = queries::insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        // Artist tag already has "David Guetta feat. Sia" → both are in track_artists
        let guetta = queries::insert_artist(&conn, "David Guetta", None).unwrap();
        let sia = queries::insert_artist(&conn, "Sia", None).unwrap();
        let track_id = queries::insert_track(
            &conn,
            "Titanium (feat. Sia)",
            None,
            guetta,
            source,
            &PathBuf::from("/t.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        queries::link_track_artist(&conn, track_id, guetta, 0).unwrap();
        queries::link_track_artist(&conn, track_id, sia, 1).unwrap();

        apply_migration_10_logic_test(&conn);

        // Still only 1 row for Sia in track_artists (INSERT OR IGNORE)
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM track_artists WHERE track_id = ?1 AND artist_id = ?2",
                rusqlite::params![track_id, sia],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "Sia must not be duplicated in track_artists");
    }
}
