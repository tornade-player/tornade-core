//! SQLite schema definitions for the Tornade music library.
//!
//! Tables: `sources`, `artists`, `albums`, `tracks`, `genres`, `track_genres`,
//! `track_artists`, `playlists`, `playlist_tracks`, `app_state`.
//! FTS5 virtual tables: `tracks_fts`, `albums_fts`, `artists_fts`.
//!
//! All `CREATE TABLE` statements use `IF NOT EXISTS` so this module is safe
//! to call on a database that already has the schema applied.

use rusqlite::{Connection, Result};

pub const SCHEMA_VERSION: i32 = 1;

/// Initialize database schema
pub fn initialize_schema(conn: &Connection) -> Result<()> {
    // Create sources table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sources (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            type TEXT NOT NULL CHECK(type IN ('disk', 'ipod', 'iphone')),
            path TEXT,
            device_id TEXT,
            last_scanned_at DATETIME,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_sources_type ON sources(type);",
    )?;

    // Create artists table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS artists (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            name_sort TEXT,
            bio TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(name)
        );

        CREATE INDEX IF NOT EXISTS idx_artists_name ON artists(name);
        CREATE INDEX IF NOT EXISTS idx_artists_name_sort ON artists(name_sort);",
    )?;

    // Create albums table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS albums (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
            year INTEGER,
            rating INTEGER DEFAULT 0 CHECK(rating >= 0 AND rating <= 5),
            artwork_path TEXT,
            artwork_hash TEXT,
            description TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(title, artist_id)
        );

        CREATE INDEX IF NOT EXISTS idx_albums_artist ON albums(artist_id);
        CREATE INDEX IF NOT EXISTS idx_albums_year ON albums(year);
        CREATE INDEX IF NOT EXISTS idx_albums_rating ON albums(rating);",
    )?;

    // Create genres table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS genres (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_genres_name ON genres(name);",
    )?;

    // Create tracks table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tracks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            album_id INTEGER REFERENCES albums(id) ON DELETE SET NULL,
            artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
            source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            file_path TEXT NOT NULL,
            duration INTEGER NOT NULL,
            track_number INTEGER,
            disc_number INTEGER DEFAULT 1,
            sample_rate INTEGER,
            bit_depth INTEGER,
            file_type TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            rating INTEGER DEFAULT 0 CHECK(rating >= 0 AND rating <= 5),
            fingerprint TEXT,
            is_duplicate INTEGER DEFAULT 0,
            duplicate_of INTEGER REFERENCES tracks(id),
            last_played_at DATETIME,
            play_count INTEGER DEFAULT 0,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(source_id, file_path)
        );

        CREATE INDEX IF NOT EXISTS idx_tracks_album ON tracks(album_id);
        CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist_id);
        CREATE INDEX IF NOT EXISTS idx_tracks_source ON tracks(source_id);
        CREATE INDEX IF NOT EXISTS idx_tracks_rating ON tracks(rating);
        CREATE INDEX IF NOT EXISTS idx_tracks_file_type ON tracks(file_type);
        CREATE INDEX IF NOT EXISTS idx_tracks_duplicate ON tracks(is_duplicate);",
    )?;

    // Create track_genres junction table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS track_genres (
            track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
            genre_id INTEGER NOT NULL REFERENCES genres(id) ON DELETE CASCADE,
            PRIMARY KEY (track_id, genre_id)
        );

        CREATE INDEX IF NOT EXISTS idx_track_genres_genre ON track_genres(genre_id);",
    )?;

    // Create track_artists junction table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS track_artists (
            track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
            artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
            position INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (track_id, artist_id)
        );

        CREATE INDEX IF NOT EXISTS idx_track_artists_artist ON track_artists(artist_id);",
    )?;

    // Create playlists table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS playlists (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            description TEXT,
            is_smart INTEGER DEFAULT 0,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_playlists_name ON playlists(name);",
    )?;

    // Create playlist_tracks junction table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS playlist_tracks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
            track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
            position INTEGER NOT NULL,
            added_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(playlist_id, position)
        );

        CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist ON playlist_tracks(playlist_id);
        CREATE INDEX IF NOT EXISTS idx_playlist_tracks_position ON playlist_tracks(playlist_id, position);",
    )?;

    // Create app_state table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );",
    )?;

    Ok(())
}

/// Create FTS5 virtual tables for full-text search (tracks, albums, artists)
pub fn initialize_fts(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS tracks_fts USING fts5(
            title,
            artist_name,
            album_title,
            genre_names,
            content='',
            tokenize='unicode61 remove_diacritics 1'
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS albums_fts USING fts5(
            title,
            artist_name,
            content='',
            tokenize='unicode61 remove_diacritics 1'
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS artists_fts USING fts5(
            name,
            content='',
            tokenize='unicode61 remove_diacritics 1'
        );",
    )?;

    Ok(())
}

/// Create triggers to keep FTS tables in sync with tracks, albums, and artists
pub fn initialize_fts_triggers(conn: &Connection) -> Result<()> {
    // ── tracks_fts triggers ───────────────────────────────────────────────
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS tracks_ai AFTER INSERT ON tracks BEGIN
            INSERT INTO tracks_fts(rowid, title, artist_name, album_title, genre_names)
            SELECT
                NEW.id,
                NEW.title,
                (SELECT name FROM artists WHERE id = NEW.artist_id),
                (SELECT title FROM albums WHERE id = NEW.album_id),
                (SELECT GROUP_CONCAT(g.name, ' ')
                 FROM track_genres tg
                 JOIN genres g ON g.id = tg.genre_id
                 WHERE tg.track_id = NEW.id);
        END;

        CREATE TRIGGER IF NOT EXISTS tracks_ad AFTER DELETE ON tracks BEGIN
            INSERT INTO tracks_fts(tracks_fts, rowid, title, artist_name, album_title, genre_names)
            VALUES('delete', OLD.id, OLD.title,
                   (SELECT name FROM artists WHERE id = OLD.artist_id),
                   (SELECT title FROM albums WHERE id = OLD.album_id),
                   '');
        END;

        CREATE TRIGGER IF NOT EXISTS tracks_au AFTER UPDATE ON tracks BEGIN
            INSERT INTO tracks_fts(tracks_fts, rowid, title, artist_name, album_title, genre_names)
            VALUES('delete', OLD.id, OLD.title, '', '', '');
            INSERT INTO tracks_fts(rowid, title, artist_name, album_title, genre_names)
            SELECT
                NEW.id,
                NEW.title,
                (SELECT name FROM artists WHERE id = NEW.artist_id),
                (SELECT title FROM albums WHERE id = NEW.album_id),
                (SELECT GROUP_CONCAT(g.name, ' ')
                 FROM track_genres tg
                 JOIN genres g ON g.id = tg.genre_id
                 WHERE tg.track_id = NEW.id);
        END;",
    )?;

    // ── albums_fts triggers ───────────────────────────────────────────────
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS albums_ai AFTER INSERT ON albums BEGIN
            INSERT INTO albums_fts(rowid, title, artist_name)
            SELECT NEW.id, NEW.title, ar.name FROM artists ar WHERE ar.id = NEW.artist_id;
        END;

        CREATE TRIGGER IF NOT EXISTS albums_ad AFTER DELETE ON albums BEGIN
            INSERT INTO albums_fts(albums_fts, rowid, title, artist_name)
            VALUES('delete', OLD.id, OLD.title, '');
        END;

        CREATE TRIGGER IF NOT EXISTS albums_au AFTER UPDATE ON albums BEGIN
            INSERT INTO albums_fts(albums_fts, rowid, title, artist_name)
            VALUES('delete', OLD.id, OLD.title, '');
            INSERT INTO albums_fts(rowid, title, artist_name)
            SELECT NEW.id, NEW.title, ar.name FROM artists ar WHERE ar.id = NEW.artist_id;
        END;",
    )?;

    // ── artists_fts triggers ──────────────────────────────────────────────
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS artists_ai AFTER INSERT ON artists BEGIN
            INSERT INTO artists_fts(rowid, name) VALUES(NEW.id, NEW.name);
        END;

        CREATE TRIGGER IF NOT EXISTS artists_ad AFTER DELETE ON artists BEGIN
            INSERT INTO artists_fts(artists_fts, rowid, name)
            VALUES('delete', OLD.id, OLD.name);
        END;

        CREATE TRIGGER IF NOT EXISTS artists_au AFTER UPDATE ON artists BEGIN
            INSERT INTO artists_fts(artists_fts, rowid, name)
            VALUES('delete', OLD.id, OLD.name);
            INSERT INTO artists_fts(rowid, name) VALUES(NEW.id, NEW.name);
        END;",
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_helpers::TestEnv;

    #[test]
    fn test_initialize_database() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(tables.contains(&"sources".to_string()));
        assert!(tables.contains(&"artists".to_string()));
        assert!(tables.contains(&"albums".to_string()));
        assert!(tables.contains(&"genres".to_string()));
        assert!(tables.contains(&"tracks".to_string()));
        assert!(tables.contains(&"track_genres".to_string()));
        assert!(tables.contains(&"track_artists".to_string()));
        assert!(tables.contains(&"playlists".to_string()));
        assert!(tables.contains(&"playlist_tracks".to_string()));
        assert!(tables.contains(&"app_state".to_string()));
    }

    #[test]
    fn test_initialize_database_idempotent() {
        let env = TestEnv::new();
        // Initialize again — should not error
        crate::db::initialize_database(&env.pool).unwrap();
        // And a third time
        crate::db::initialize_database(&env.pool).unwrap();

        let conn = env.pool.get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tracks'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_fts_tables_exist() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();

        let fts_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='tracks_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(fts_exists);

        let albums_fts_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='albums_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(albums_fts_exists);

        let artists_fts_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='artists_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(artists_fts_exists);

        // Check FTS triggers
        let triggers: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='trigger' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(triggers.contains(&"tracks_ai".to_string()));
        assert!(triggers.contains(&"tracks_ad".to_string()));
        assert!(triggers.contains(&"tracks_au".to_string()));
        assert!(triggers.contains(&"albums_ai".to_string()));
        assert!(triggers.contains(&"albums_ad".to_string()));
        assert!(triggers.contains(&"albums_au".to_string()));
        assert!(triggers.contains(&"artists_ai".to_string()));
        assert!(triggers.contains(&"artists_ad".to_string()));
        assert!(triggers.contains(&"artists_au".to_string()));
    }

    #[test]
    fn test_migrations_applied() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();

        // Check migration version
        let max_version: i32 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(max_version, 11);

        // Verify migration 2 columns exist
        let album_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(albums)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(album_cols.contains(&"online_artwork_path".to_string()));
        assert!(album_cols.contains(&"artwork_source".to_string()));
        assert!(album_cols.contains(&"artwork_fetched_at".to_string()));
        // Migration 3
        assert!(album_cols.contains(&"musicbrainz_id".to_string()));
        assert!(album_cols.contains(&"label".to_string()));
        assert!(album_cols.contains(&"country".to_string()));
        assert!(album_cols.contains(&"barcode".to_string()));
        assert!(album_cols.contains(&"album_type".to_string()));
        assert!(album_cols.contains(&"release_status".to_string()));

        let artist_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(artists)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(artist_cols.contains(&"photo_path".to_string()));
        assert!(artist_cols.contains(&"photo_source".to_string()));
        assert!(artist_cols.contains(&"photo_fetched_at".to_string()));
        // Migration 7
        assert!(artist_cols.contains(&"country".to_string()));
        assert!(artist_cols.contains(&"genre".to_string()));
        assert!(artist_cols.contains(&"style".to_string()));
        assert!(artist_cols.contains(&"mood".to_string()));
        assert!(artist_cols.contains(&"formed_year".to_string()));
        assert!(artist_cols.contains(&"musicbrainz_id".to_string()));
        assert!(artist_cols.contains(&"theaudiodb_id".to_string()));
    }
}
