// SQLite database schema for Tornade Music Player
// Based on data-model.md

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

/// Create FTS5 virtual table for full-text search
pub fn initialize_fts(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS tracks_fts USING fts5(
            title,
            artist_name,
            album_title,
            genre_names,
            content='',
            tokenize='unicode61 remove_diacritics 1'
        );",
    )?;

    Ok(())
}

/// Create triggers to keep FTS in sync with tracks
pub fn initialize_fts_triggers(conn: &Connection) -> Result<()> {
    // Trigger for INSERT
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
        END;",
    )?;

    // Trigger for DELETE
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS tracks_ad AFTER DELETE ON tracks BEGIN
            INSERT INTO tracks_fts(tracks_fts, rowid, title, artist_name, album_title, genre_names)
            VALUES('delete', OLD.id, OLD.title,
                   (SELECT name FROM artists WHERE id = OLD.artist_id),
                   (SELECT title FROM albums WHERE id = OLD.album_id),
                   '');
        END;",
    )?;

    // Trigger for UPDATE
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS tracks_au AFTER UPDATE ON tracks BEGIN
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

    Ok(())
}
