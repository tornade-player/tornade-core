// Database query operations

use crate::models::{Track, Album, Artist, Genre, Source, Playlist, AudioFormat};
use crate::models::source::SourceType;
use rusqlite::{Connection, Result, params, OptionalExtension};
use std::path::PathBuf;

// ============================================================================
// Source operations
// ============================================================================

pub fn insert_source(conn: &Connection, name: &str, source_type: SourceType, path: Option<&PathBuf>) -> Result<i64> {
    conn.execute(
        "INSERT INTO sources (name, type, path) VALUES (?1, ?2, ?3)",
        params![name, source_type.as_str(), path.map(|p| p.to_string_lossy().to_string())],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_source(conn: &Connection, id: i64) -> Result<Option<Source>> {
    conn.query_row(
        "SELECT id, name, type, path, device_id, last_scanned_at FROM sources WHERE id = ?1",
        params![id],
        |row| {
            Ok(Source {
                id: row.get(0)?,
                name: row.get(1)?,
                source_type: SourceType::from_str(&row.get::<_, String>(2)?).unwrap(),
                path: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
                device_id: row.get(4)?,
                last_scanned_at: row.get(5)?,
            })
        },
    ).optional()
}

pub fn list_sources(conn: &Connection) -> Result<Vec<Source>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, type, path, device_id, last_scanned_at FROM sources ORDER BY name"
    )?;

    let sources = stmt.query_map([], |row| {
        Ok(Source {
            id: row.get(0)?,
            name: row.get(1)?,
            source_type: SourceType::from_str(&row.get::<_, String>(2)?).unwrap(),
            path: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
            device_id: row.get(4)?,
            last_scanned_at: row.get(5)?,
        })
    })?;

    sources.collect()
}

// ============================================================================
// Artist operations
// ============================================================================

pub fn insert_artist(conn: &Connection, name: &str, name_sort: Option<&str>) -> Result<i64> {
    conn.execute(
        "INSERT INTO artists (name, name_sort) VALUES (?1, ?2) ON CONFLICT(name) DO NOTHING",
        params![name, name_sort],
    )?;

    // Get the ID (either newly inserted or existing)
    conn.query_row(
        "SELECT id FROM artists WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )
}

pub fn get_artist(conn: &Connection, id: i64) -> Result<Option<Artist>> {
    conn.query_row(
        "SELECT id, name, name_sort, bio FROM artists WHERE id = ?1",
        params![id],
        |row| {
            Ok(Artist {
                id: row.get(0)?,
                name: row.get(1)?,
                name_sort: row.get(2)?,
                bio: row.get(3)?,
            })
        },
    ).optional()
}

// ============================================================================
// Album operations
// ============================================================================

pub fn insert_album(conn: &Connection, title: &str, artist_id: i64, year: Option<u16>) -> Result<i64> {
    conn.execute(
        "INSERT INTO albums (title, artist_id, year) VALUES (?1, ?2, ?3) ON CONFLICT(title, artist_id) DO NOTHING",
        params![title, artist_id, year],
    )?;

    // Get the ID
    conn.query_row(
        "SELECT id FROM albums WHERE title = ?1 AND artist_id = ?2",
        params![title, artist_id],
        |row| row.get(0),
    )
}

pub fn get_album(conn: &Connection, id: i64) -> Result<Option<Album>> {
    conn.query_row(
        "SELECT id, title, artist_id, year, rating, artwork_path, description FROM albums WHERE id = ?1",
        params![id],
        |row| {
            Ok(Album {
                id: row.get(0)?,
                title: row.get(1)?,
                artist_id: row.get(2)?,
                year: row.get(3)?,
                rating: row.get(4)?,
                artwork_path: row.get::<_, Option<String>>(5)?.map(PathBuf::from),
                description: row.get(6)?,
            })
        },
    ).optional()
}

pub fn update_album_rating(conn: &Connection, album_id: i64, rating: u8) -> Result<()> {
    conn.execute(
        "UPDATE albums SET rating = ?1 WHERE id = ?2",
        params![rating, album_id],
    )?;
    Ok(())
}

// ============================================================================
// Genre operations
// ============================================================================

pub fn insert_genre(conn: &Connection, name: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO genres (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
        params![name],
    )?;

    conn.query_row(
        "SELECT id FROM genres WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )
}

pub fn get_genre(conn: &Connection, id: i64) -> Result<Option<Genre>> {
    conn.query_row(
        "SELECT id, name FROM genres WHERE id = ?1",
        params![id],
        |row| {
            Ok(Genre {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        },
    ).optional()
}

// ============================================================================
// Track operations
// ============================================================================

pub fn insert_track(
    conn: &Connection,
    title: &str,
    album_id: Option<i64>,
    artist_id: i64,
    source_id: i64,
    file_path: &PathBuf,
    duration_ms: i64,
    track_number: Option<u32>,
    sample_rate: Option<u32>,
    bit_depth: Option<u8>,
    file_type: AudioFormat,
    file_size: u64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO tracks (
            title, album_id, artist_id, source_id, file_path,
            duration, track_number, sample_rate, bit_depth,
            file_type, file_size
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(source_id, file_path) DO UPDATE SET
            title = excluded.title,
            album_id = excluded.album_id,
            artist_id = excluded.artist_id,
            duration = excluded.duration,
            updated_at = CURRENT_TIMESTAMP",
        params![
            title,
            album_id,
            artist_id,
            source_id,
            file_path.to_string_lossy().to_string(),
            duration_ms,
            track_number,
            sample_rate,
            bit_depth,
            file_type.as_str(),
            file_size as i64,
        ],
    )?;

    Ok(conn.last_insert_rowid())
}

pub fn get_track(conn: &Connection, id: i64) -> Result<Option<Track>> {
    conn.query_row(
        "SELECT id, title, album_id, artist_id, source_id, file_path,
                duration, track_number, disc_number, sample_rate, bit_depth,
                file_type, file_size, rating, fingerprint, is_duplicate,
                duplicate_of, last_played_at, play_count
         FROM tracks WHERE id = ?1",
        params![id],
        |row| {
            Ok(Track {
                id: row.get(0)?,
                title: row.get(1)?,
                album_id: row.get(2)?,
                artist_id: row.get(3)?,
                source_id: row.get(4)?,
                file_path: PathBuf::from(row.get::<_, String>(5)?),
                duration: std::time::Duration::from_millis(row.get::<_, i64>(6)? as u64),
                track_number: row.get(7)?,
                disc_number: row.get(8)?,
                sample_rate: row.get(9)?,
                bit_depth: row.get(10)?,
                file_type: AudioFormat::from_str(&row.get::<_, String>(11)?).unwrap(),
                file_size: row.get::<_, i64>(12)? as u64,
                rating: row.get(13)?,
                fingerprint: row.get(14)?,
                is_duplicate: row.get::<_, i32>(15)? != 0,
                duplicate_of: row.get(16)?,
                last_played_at: row.get(17)?,
                play_count: row.get::<_, i32>(18)? as u32,
            })
        },
    ).optional()
}

pub fn get_album_tracks(conn: &Connection, album_id: i64) -> Result<Vec<Track>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, album_id, artist_id, source_id, file_path,
                duration, track_number, disc_number, sample_rate, bit_depth,
                file_type, file_size, rating, fingerprint, is_duplicate,
                duplicate_of, last_played_at, play_count
         FROM tracks WHERE album_id = ?1
         ORDER BY disc_number, track_number"
    )?;

    let tracks = stmt.query_map(params![album_id], |row| {
        Ok(Track {
            id: row.get(0)?,
            title: row.get(1)?,
            album_id: row.get(2)?,
            artist_id: row.get(3)?,
            source_id: row.get(4)?,
            file_path: PathBuf::from(row.get::<_, String>(5)?),
            duration: std::time::Duration::from_millis(row.get::<_, i64>(6)? as u64),
            track_number: row.get(7)?,
            disc_number: row.get(8)?,
            sample_rate: row.get(9)?,
            bit_depth: row.get(10)?,
            file_type: AudioFormat::from_str(&row.get::<_, String>(11)?).unwrap(),
            file_size: row.get::<_, i64>(12)? as u64,
            rating: row.get(13)?,
            fingerprint: row.get(14)?,
            is_duplicate: row.get::<_, i32>(15)? != 0,
            duplicate_of: row.get(16)?,
            last_played_at: row.get(17)?,
            play_count: row.get::<_, i32>(18)? as u32,
        })
    })?;

    tracks.collect()
}

pub fn update_track_rating(conn: &Connection, track_id: i64, rating: u8) -> Result<()> {
    conn.execute(
        "UPDATE tracks SET rating = ?1 WHERE id = ?2",
        params![rating, track_id],
    )?;
    Ok(())
}

pub fn link_track_genre(conn: &Connection, track_id: i64, genre_id: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO track_genres (track_id, genre_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        params![track_id, genre_id],
    )?;
    Ok(())
}

// ============================================================================
// Album operations (extended)
// ============================================================================

pub fn count_albums(
    conn: &Connection,
    artist_id: Option<i64>,
    genre_id: Option<i64>,
    min_rating: Option<u8>,
) -> Result<usize> {
    let mut sql = String::from(
        "SELECT COUNT(DISTINCT a.id) FROM albums a"
    );

    let mut conditions = Vec::new();

    if genre_id.is_some() {
        sql.push_str(" JOIN tracks t ON t.album_id = a.id
                       JOIN track_genres tg ON tg.track_id = t.id");
        conditions.push("tg.genre_id = ?");
    }

    if artist_id.is_some() {
        conditions.push("a.artist_id = ?");
    }

    if min_rating.is_some() {
        conditions.push("a.rating >= ?");
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    let mut stmt = conn.prepare(&sql)?;

    let mut idx = 1;
    if let Some(gid) = genre_id {
        stmt.raw_bind_parameter(idx, gid)?;
        idx += 1;
    }
    if let Some(aid) = artist_id {
        stmt.raw_bind_parameter(idx, aid)?;
        idx += 1;
    }
    if let Some(rating) = min_rating {
        stmt.raw_bind_parameter(idx, rating)?;
    }

    let count: i64 = stmt.query_row([], |row| row.get(0))?;
    Ok(count as usize)
}

pub fn list_albums(
    conn: &Connection,
    artist_id: Option<i64>,
    genre_id: Option<i64>,
    min_rating: Option<u8>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<Album>> {
    let mut sql = String::from(
        "SELECT DISTINCT a.id, a.title, a.artist_id, a.year, a.rating, a.artwork_path, a.description
         FROM albums a"
    );

    let mut conditions = Vec::new();

    if genre_id.is_some() {
        sql.push_str(" JOIN tracks t ON t.album_id = a.id
                       JOIN track_genres tg ON tg.track_id = t.id");
        conditions.push("tg.genre_id = ?");
    }

    if artist_id.is_some() {
        conditions.push("a.artist_id = ?");
    }

    if min_rating.is_some() {
        conditions.push("a.rating >= ?");
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(" ORDER BY a.title");

    if let Some(lim) = limit {
        sql.push_str(&format!(" LIMIT {}", lim));
    }

    if let Some(off) = offset {
        sql.push_str(&format!(" OFFSET {}", off));
    }

    let mut stmt = conn.prepare(&sql)?;

    let mut idx = 1;
    if let Some(gid) = genre_id {
        stmt.raw_bind_parameter(idx, gid)?;
        idx += 1;
    }
    if let Some(aid) = artist_id {
        stmt.raw_bind_parameter(idx, aid)?;
        idx += 1;
    }
    if let Some(rating) = min_rating {
        stmt.raw_bind_parameter(idx, rating)?;
    }

    let albums = stmt.query_map([], |row| {
        Ok(Album {
            id: row.get(0)?,
            title: row.get(1)?,
            artist_id: row.get(2)?,
            year: row.get(3)?,
            rating: row.get(4)?,
            artwork_path: row.get::<_, Option<String>>(5)?.map(PathBuf::from),
            description: row.get(6)?,
        })
    })?;

    albums.collect()
}

pub fn get_artist_albums(conn: &Connection, artist_id: i64) -> Result<Vec<Album>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, artist_id, year, rating, artwork_path, description
         FROM albums
         WHERE artist_id = ?1
         ORDER BY year DESC, title"
    )?;

    let albums = stmt.query_map(params![artist_id], |row| {
        Ok(Album {
            id: row.get(0)?,
            title: row.get(1)?,
            artist_id: row.get(2)?,
            year: row.get(3)?,
            rating: row.get(4)?,
            artwork_path: row.get::<_, Option<String>>(5)?.map(PathBuf::from),
            description: row.get(6)?,
        })
    })?;

    albums.collect()
}

// ============================================================================
// Artist operations (extended)
// ============================================================================

pub fn list_artists(conn: &Connection) -> Result<Vec<Artist>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, name_sort, bio FROM artists ORDER BY COALESCE(name_sort, name)"
    )?;

    let artists = stmt.query_map([], |row| {
        Ok(Artist {
            id: row.get(0)?,
            name: row.get(1)?,
            name_sort: row.get(2)?,
            bio: row.get(3)?,
        })
    })?;

    artists.collect()
}

pub fn get_genre_artists(conn: &Connection, genre_id: i64) -> Result<Vec<Artist>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT a.id, a.name, a.name_sort, a.bio
         FROM artists a
         JOIN tracks t ON t.artist_id = a.id
         JOIN track_genres tg ON tg.track_id = t.id
         WHERE tg.genre_id = ?1
         ORDER BY a.name"
    )?;

    let artists = stmt.query_map(params![genre_id], |row| {
        Ok(Artist {
            id: row.get(0)?,
            name: row.get(1)?,
            name_sort: row.get(2)?,
            bio: row.get(3)?,
        })
    })?;

    artists.collect()
}

// ============================================================================
// Genre operations (extended)
// ============================================================================

pub fn list_genres_with_count(conn: &Connection) -> Result<Vec<(Genre, u32, u32)>> {
    let mut stmt = conn.prepare(
        "SELECT g.id, g.name,
                COUNT(DISTINCT tg.track_id) as track_count,
                COUNT(DISTINCT t.album_id) as album_count
         FROM genres g
         LEFT JOIN track_genres tg ON tg.genre_id = g.id
         LEFT JOIN tracks t ON t.id = tg.track_id
         GROUP BY g.id, g.name
         ORDER BY g.name"
    )?;

    let genres = stmt.query_map([], |row| {
        Ok((
            Genre {
                id: row.get(0)?,
                name: row.get(1)?,
            },
            row.get::<_, i64>(2)? as u32,
            row.get::<_, i64>(3)? as u32,
        ))
    })?;

    genres.collect()
}

pub fn get_genre_tracks(conn: &Connection, genre_id: i64) -> Result<Vec<Track>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.title, t.album_id, t.artist_id, t.source_id, t.file_path,
                t.duration, t.track_number, t.disc_number, t.sample_rate, t.bit_depth,
                t.file_type, t.file_size, t.rating, t.fingerprint, t.is_duplicate,
                t.duplicate_of, t.last_played_at, t.play_count
         FROM tracks t
         JOIN track_genres tg ON tg.track_id = t.id
         WHERE tg.genre_id = ?1
         ORDER BY t.title"
    )?;

    let tracks = stmt.query_map(params![genre_id], |row| {
        Ok(Track {
            id: row.get(0)?,
            title: row.get(1)?,
            album_id: row.get(2)?,
            artist_id: row.get(3)?,
            source_id: row.get(4)?,
            file_path: PathBuf::from(row.get::<_, String>(5)?),
            duration: std::time::Duration::from_millis(row.get::<_, i64>(6)? as u64),
            track_number: row.get(7)?,
            disc_number: row.get(8)?,
            sample_rate: row.get(9)?,
            bit_depth: row.get(10)?,
            file_type: AudioFormat::from_str(&row.get::<_, String>(11)?).unwrap(),
            file_size: row.get::<_, i64>(12)? as u64,
            rating: row.get(13)?,
            fingerprint: row.get(14)?,
            is_duplicate: row.get::<_, i32>(15)? != 0,
            duplicate_of: row.get(16)?,
            last_played_at: row.get(17)?,
            play_count: row.get::<_, i32>(18)? as u32,
        })
    })?;

    tracks.collect()
}

pub fn get_album_genres(conn: &Connection, album_id: i64) -> Result<Vec<Genre>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT g.id, g.name
         FROM genres g
         JOIN track_genres tg ON g.id = tg.genre_id
         JOIN tracks t ON tg.track_id = t.id
         WHERE t.album_id = ?1
         ORDER BY g.name"
    )?;

    let genres = stmt.query_map(params![album_id], |row| {
        Ok(Genre {
            id: row.get(0)?,
            name: row.get(1)?,
        })
    })?;

    genres.collect()
}

pub fn get_source_tracks(conn: &Connection, source_id: i64) -> Result<Vec<Track>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, album_id, artist_id, source_id, file_path,
                duration, track_number, disc_number, sample_rate, bit_depth,
                file_type, file_size, rating, fingerprint, is_duplicate,
                duplicate_of, last_played_at, play_count
         FROM tracks
         WHERE source_id = ?1
         ORDER BY title"
    )?;

    let tracks = stmt.query_map(params![source_id], |row| {
        Ok(Track {
            id: row.get(0)?,
            title: row.get(1)?,
            album_id: row.get(2)?,
            artist_id: row.get(3)?,
            source_id: row.get(4)?,
            file_path: PathBuf::from(row.get::<_, String>(5)?),
            duration: std::time::Duration::from_millis(row.get::<_, i64>(6)? as u64),
            track_number: row.get(7)?,
            disc_number: row.get(8)?,
            sample_rate: row.get(9)?,
            bit_depth: row.get(10)?,
            file_type: AudioFormat::from_str(&row.get::<_, String>(11)?).unwrap(),
            file_size: row.get::<_, i64>(12)? as u64,
            rating: row.get(13)?,
            fingerprint: row.get(14)?,
            is_duplicate: row.get::<_, i32>(15)? != 0,
            duplicate_of: row.get(16)?,
            last_played_at: row.get(17)?,
            play_count: row.get::<_, i32>(18)? as u32,
        })
    })?;

    tracks.collect()
}

// ============================================================================
// Search operations
// ============================================================================

pub fn search_library(conn: &Connection, query: &str, limit: usize) -> Result<(Vec<Track>, Vec<Album>, Vec<Artist>)> {
    // Search tracks using FTS5
    let mut tracks = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT t.id, t.title, t.album_id, t.artist_id, t.source_id, t.file_path,
                t.duration, t.track_number, t.disc_number, t.sample_rate, t.bit_depth,
                t.file_type, t.file_size, t.rating, t.fingerprint, t.is_duplicate,
                t.duplicate_of, t.last_played_at, t.play_count
         FROM tracks_fts
         JOIN tracks t ON tracks_fts.rowid = t.id
         WHERE tracks_fts MATCH ?1
         LIMIT ?2"
    )?;

    let track_results = stmt.query_map(params![query, limit], |row| {
        Ok(Track {
            id: row.get(0)?,
            title: row.get(1)?,
            album_id: row.get(2)?,
            artist_id: row.get(3)?,
            source_id: row.get(4)?,
            file_path: PathBuf::from(row.get::<_, String>(5)?),
            duration: std::time::Duration::from_millis(row.get::<_, i64>(6)? as u64),
            track_number: row.get(7)?,
            disc_number: row.get(8)?,
            sample_rate: row.get(9)?,
            bit_depth: row.get(10)?,
            file_type: AudioFormat::from_str(&row.get::<_, String>(11)?).unwrap(),
            file_size: row.get::<_, i64>(12)? as u64,
            rating: row.get(13)?,
            fingerprint: row.get(14)?,
            is_duplicate: row.get::<_, i32>(15)? != 0,
            duplicate_of: row.get(16)?,
            last_played_at: row.get(17)?,
            play_count: row.get::<_, i32>(18)? as u32,
        })
    })?;

    for track in track_results {
        tracks.push(track?);
    }

    // Search albums
    let mut albums = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT DISTINCT a.id, a.title, a.artist_id, a.year, a.rating, a.artwork_path, a.description
         FROM albums a
         WHERE a.title LIKE ?1
         LIMIT ?2"
    )?;

    let search_pattern = format!("%{}%", query);
    let album_results = stmt.query_map(params![search_pattern, limit], |row| {
        Ok(Album {
            id: row.get(0)?,
            title: row.get(1)?,
            artist_id: row.get(2)?,
            year: row.get(3)?,
            rating: row.get(4)?,
            artwork_path: row.get::<_, Option<String>>(5)?.map(PathBuf::from),
            description: row.get(6)?,
        })
    })?;

    for album in album_results {
        albums.push(album?);
    }

    // Search artists
    let mut artists = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, name, name_sort, bio
         FROM artists
         WHERE name LIKE ?1
         LIMIT ?2"
    )?;

    let artist_results = stmt.query_map(params![search_pattern, limit], |row| {
        Ok(Artist {
            id: row.get(0)?,
            name: row.get(1)?,
            name_sort: row.get(2)?,
            bio: row.get(3)?,
        })
    })?;

    for artist in artist_results {
        artists.push(artist?);
    }

    Ok((tracks, albums, artists))
}

// ============================================================================
// Playlist operations
// ============================================================================

pub fn create_playlist(conn: &Connection, name: &str, description: Option<&str>) -> Result<i64> {
    conn.execute(
        "INSERT INTO playlists (name, description) VALUES (?1, ?2)",
        params![name, description],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_playlist(conn: &Connection, id: i64) -> Result<Option<Playlist>> {
    let playlist = conn.query_row(
        "SELECT id, name, description, created_at, updated_at FROM playlists WHERE id = ?1",
        params![id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    ).optional()?;

    if let Some((id, name, description, created_at, updated_at)) = playlist {
        // Get track IDs in order
        let mut stmt = conn.prepare(
            "SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position"
        )?;
        let tracks: Result<Vec<i64>> = stmt
            .query_map(params![id], |row| row.get(0))?
            .collect();

        Ok(Some(Playlist {
            id,
            name,
            description,
            tracks: tracks?,
            created_at,
            updated_at,
        }))
    } else {
        Ok(None)
    }
}

pub fn add_track_to_playlist(conn: &Connection, playlist_id: i64, track_id: i64) -> Result<()> {
    // Get max position
    let max_pos: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), -1) FROM playlist_tracks WHERE playlist_id = ?1",
        params![playlist_id],
        |row| row.get(0),
    )?;

    conn.execute(
        "INSERT INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
        params![playlist_id, track_id, max_pos + 1],
    )?;

    // Update playlist timestamp
    conn.execute(
        "UPDATE playlists SET updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
        params![playlist_id],
    )?;

    Ok(())
}
