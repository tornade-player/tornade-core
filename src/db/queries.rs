// Database query operations

use crate::models::source::SourceType;
use crate::models::{Album, Artist, AudioFormat, Genre, Playlist, Source, Track};
use rusqlite::{Connection, OptionalExtension, Result, params};
use std::path::{Path, PathBuf};

// ============================================================================
// Source operations
// ============================================================================

pub fn insert_source(
    conn: &Connection,
    name: &str,
    source_type: SourceType,
    path: Option<&Path>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO sources (name, type, path) VALUES (?1, ?2, ?3)",
        params![
            name,
            source_type.as_str(),
            path.map(|p| p.to_string_lossy().into_owned())
        ],
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
    )
    .optional()
}

pub fn list_sources(conn: &Connection) -> Result<Vec<Source>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, type, path, device_id, last_scanned_at FROM sources ORDER BY name",
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
        "SELECT id, name, name_sort, bio, country, genre, style, mood,
                formed_year, born_year, died_year, disbanded, musicbrainz_id, theaudiodb_id
         FROM artists WHERE id = ?1",
        params![id],
        |row| {
            Ok(Artist {
                id: row.get(0)?,
                name: row.get(1)?,
                name_sort: row.get(2)?,
                bio: row.get(3)?,
                country: row.get(4)?,
                genre: row.get(5)?,
                style: row.get(6)?,
                mood: row.get(7)?,
                formed_year: row.get(8)?,
                born_year: row.get(9)?,
                died_year: row.get(10)?,
                disbanded: row.get(11)?,
                musicbrainz_id: row.get(12)?,
                theaudiodb_id: row.get(13)?,
            })
        },
    )
    .optional()
}

// ============================================================================
// Album operations
// ============================================================================

pub fn insert_album(
    conn: &Connection,
    title: &str,
    artist_id: i64,
    year: Option<u16>,
) -> Result<i64> {
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
        "SELECT a.id, a.title, a.artist_id, ar.name as artist_name, a.year, a.rating,
                a.artwork_path, a.online_artwork_path, a.description,
                a.musicbrainz_id, a.label, a.country, a.barcode, a.album_type, a.release_status
         FROM albums a
         JOIN artists ar ON ar.id = a.artist_id
         WHERE a.id = ?1",
        params![id],
        |row| {
            Ok(Album {
                id: row.get(0)?,
                title: row.get(1)?,
                artist_id: row.get(2)?,
                artist_name: row.get(3)?,
                year: row.get(4)?,
                rating: row.get(5)?,
                artwork_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
                online_artwork_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
                description: row.get(8)?,
                musicbrainz_id: row.get(9)?,
                label: row.get(10)?,
                country: row.get(11)?,
                barcode: row.get(12)?,
                album_type: row.get(13)?,
                release_status: row.get(14)?,
            })
        },
    )
    .optional()
}

/// Find any existing album with this exact title, regardless of artist.
/// Used when no ALBUMARTIST tag is present to avoid creating a separate album
/// entry for every featured artist in a compilation.
/// Find an album by title, returning `(album_id, artist_id)`.
/// Used when no ALBUMARTIST tag is present so we can detect multi-artist albums.
pub fn find_album_by_title(conn: &Connection, title: &str) -> Result<Option<(i64, i64)>> {
    conn.query_row(
        "SELECT id, artist_id FROM albums WHERE title = ?1 LIMIT 1",
        params![title],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

pub fn update_album_artist(conn: &Connection, album_id: i64, artist_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE albums SET artist_id = ?1 WHERE id = ?2",
        params![artist_id, album_id],
    )?;
    Ok(())
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
    )
    .optional()
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
    file_path: &Path,
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
            file_path.to_string_lossy().into_owned(),
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
    )
    .optional()
}

pub fn get_album_tracks(conn: &Connection, album_id: i64) -> Result<Vec<Track>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, album_id, artist_id, source_id, file_path,
                duration, track_number, disc_number, sample_rate, bit_depth,
                file_type, file_size, rating, fingerprint, is_duplicate,
                duplicate_of, last_played_at, play_count
         FROM tracks WHERE album_id = ?1
         ORDER BY disc_number, track_number",
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

pub fn delete_track(conn: &Connection, track_id: i64) -> Result<()> {
    conn.execute("DELETE FROM tracks WHERE id = ?1", params![track_id])?;
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
    use rusqlite::types::Value;

    let mut sql = String::from("SELECT COUNT(DISTINCT a.id) FROM albums a");
    let mut conditions = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    if let Some(gid) = genre_id {
        sql.push_str(
            " JOIN tracks t ON t.album_id = a.id
                       JOIN track_genres tg ON tg.track_id = t.id",
        );
        conditions.push("tg.genre_id = ?");
        params.push(Value::Integer(gid));
    }
    if let Some(aid) = artist_id {
        conditions.push("a.artist_id = ?");
        params.push(Value::Integer(aid));
    }
    if let Some(rating) = min_rating {
        conditions.push("a.rating >= ?");
        params.push(Value::Integer(i64::from(rating)));
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    let mut stmt = conn.prepare(&sql)?;
    let count: i64 = stmt.query_row(rusqlite::params_from_iter(params.iter()), |row| row.get(0))?;
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
    use rusqlite::types::Value;

    let mut sql = String::from(
        "SELECT DISTINCT a.id, a.title, a.artist_id, ar.name as artist_name, a.year, a.rating,
                a.artwork_path, a.online_artwork_path, a.description,
                a.musicbrainz_id, a.label, a.country, a.barcode, a.album_type, a.release_status
         FROM albums a
         JOIN artists ar ON ar.id = a.artist_id",
    );

    let mut conditions = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    if let Some(gid) = genre_id {
        sql.push_str(
            " JOIN tracks t ON t.album_id = a.id
                       JOIN track_genres tg ON tg.track_id = t.id",
        );
        conditions.push("tg.genre_id = ?");
        params.push(Value::Integer(gid));
    }
    if let Some(aid) = artist_id {
        conditions.push("a.artist_id = ?");
        params.push(Value::Integer(aid));
    }
    if let Some(rating) = min_rating {
        conditions.push("a.rating >= ?");
        params.push(Value::Integer(i64::from(rating)));
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(" ORDER BY a.title");

    if let Some(lim) = limit {
        sql.push_str(&format!(" LIMIT {lim}"));
    }
    if let Some(off) = offset {
        sql.push_str(&format!(" OFFSET {off}"));
    }

    let mut stmt = conn.prepare(&sql)?;
    let albums = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(Album {
            id: row.get(0)?,
            title: row.get(1)?,
            artist_id: row.get(2)?,
            artist_name: row.get(3)?,
            year: row.get(4)?,
            rating: row.get(5)?,
            artwork_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
            online_artwork_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
            description: row.get(8)?,
            musicbrainz_id: row.get(9)?,
            label: row.get(10)?,
            country: row.get(11)?,
            barcode: row.get(12)?,
            album_type: row.get(13)?,
            release_status: row.get(14)?,
        })
    })?;

    albums.collect()
}

pub fn get_artist_albums(conn: &Connection, artist_id: i64) -> Result<Vec<Album>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT a.id, a.title, a.artist_id, ar.name as artist_name, a.year, a.rating,
                a.artwork_path, a.online_artwork_path, a.description,
                a.musicbrainz_id, a.label, a.country, a.barcode, a.album_type, a.release_status
         FROM albums a
         JOIN artists ar ON ar.id = a.artist_id
         WHERE a.artist_id = ?1
            OR EXISTS (SELECT 1 FROM tracks t WHERE t.album_id = a.id AND t.artist_id = ?1)
         ORDER BY a.year DESC, a.title",
    )?;

    let albums = stmt.query_map(params![artist_id], |row| {
        Ok(Album {
            id: row.get(0)?,
            title: row.get(1)?,
            artist_id: row.get(2)?,
            artist_name: row.get(3)?,
            year: row.get(4)?,
            rating: row.get(5)?,
            artwork_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
            online_artwork_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
            description: row.get(8)?,
            musicbrainz_id: row.get(9)?,
            label: row.get(10)?,
            country: row.get(11)?,
            barcode: row.get(12)?,
            album_type: row.get(13)?,
            release_status: row.get(14)?,
        })
    })?;

    albums.collect()
}

// ============================================================================
// Artist operations (extended)
// ============================================================================

pub fn list_artists(conn: &Connection) -> Result<Vec<Artist>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, name_sort, bio, country, genre, style, mood,
                formed_year, born_year, died_year, disbanded, musicbrainz_id, theaudiodb_id
         FROM artists ORDER BY COALESCE(name_sort, name)",
    )?;

    let artists = stmt.query_map([], |row| {
        Ok(Artist {
            id: row.get(0)?,
            name: row.get(1)?,
            name_sort: row.get(2)?,
            bio: row.get(3)?,
            country: row.get(4)?,
            genre: row.get(5)?,
            style: row.get(6)?,
            mood: row.get(7)?,
            formed_year: row.get(8)?,
            born_year: row.get(9)?,
            died_year: row.get(10)?,
            disbanded: row.get(11)?,
            musicbrainz_id: row.get(12)?,
            theaudiodb_id: row.get(13)?,
        })
    })?;

    artists.collect()
}

pub fn get_genre_albums(conn: &Connection, genre_id: i64) -> Result<Vec<Album>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT a.id, a.title, a.artist_id, ar.name as artist_name, a.year, a.rating,
                a.artwork_path, a.online_artwork_path, a.description,
                a.musicbrainz_id, a.label, a.country, a.barcode, a.album_type, a.release_status
         FROM albums a
         JOIN artists ar ON ar.id = a.artist_id
         JOIN tracks t ON t.album_id = a.id
         JOIN track_genres tg ON tg.track_id = t.id
         WHERE tg.genre_id = ?1
         ORDER BY a.year DESC, a.title",
    )?;

    let albums = stmt.query_map(params![genre_id], |row| {
        Ok(Album {
            id: row.get(0)?,
            title: row.get(1)?,
            artist_id: row.get(2)?,
            artist_name: row.get(3)?,
            year: row.get(4)?,
            rating: row.get(5)?,
            artwork_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
            online_artwork_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
            description: row.get(8)?,
            musicbrainz_id: row.get(9)?,
            label: row.get(10)?,
            country: row.get(11)?,
            barcode: row.get(12)?,
            album_type: row.get(13)?,
            release_status: row.get(14)?,
        })
    })?;

    albums.collect()
}

pub fn get_genre_artists(conn: &Connection, genre_id: i64) -> Result<Vec<Artist>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT a.id, a.name, a.name_sort, a.bio, a.country, a.genre, a.style, a.mood,
                a.formed_year, a.born_year, a.died_year, a.disbanded, a.musicbrainz_id, a.theaudiodb_id
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
            country: row.get(4)?,
            genre: row.get(5)?,
            style: row.get(6)?,
            mood: row.get(7)?,
            formed_year: row.get(8)?,
            born_year: row.get(9)?,
            died_year: row.get(10)?,
            disbanded: row.get(11)?,
            musicbrainz_id: row.get(12)?,
            theaudiodb_id: row.get(13)?,
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
         ORDER BY g.name",
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
         ORDER BY t.title",
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
         ORDER BY g.name",
    )?;

    let genres = stmt.query_map(params![album_id], |row| {
        Ok(Genre {
            id: row.get(0)?,
            name: row.get(1)?,
        })
    })?;

    genres.collect()
}

pub fn get_artist_genres(conn: &Connection, artist_id: i64) -> Result<Vec<Genre>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT g.id, g.name
         FROM genres g
         JOIN track_genres tg ON g.id = tg.genre_id
         JOIN tracks t ON tg.track_id = t.id
         WHERE t.artist_id = ?1
         ORDER BY g.name",
    )?;

    let genres = stmt.query_map(params![artist_id], |row| {
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
         ORDER BY title",
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

pub fn search_library(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<(Vec<Track>, Vec<Album>, Vec<Artist>)> {
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
         LIMIT ?2",
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
        "SELECT DISTINCT a.id, a.title, a.artist_id, ar.name as artist_name, a.year, a.rating,
                a.artwork_path, a.online_artwork_path, a.description,
                a.musicbrainz_id, a.label, a.country, a.barcode, a.album_type, a.release_status
         FROM albums a
         JOIN artists ar ON ar.id = a.artist_id
         WHERE a.title LIKE ?1
         LIMIT ?2",
    )?;

    let search_pattern = format!("%{query}%");
    let album_results = stmt.query_map(params![search_pattern, limit], |row| {
        Ok(Album {
            id: row.get(0)?,
            title: row.get(1)?,
            artist_id: row.get(2)?,
            artist_name: row.get(3)?,
            year: row.get(4)?,
            rating: row.get(5)?,
            artwork_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
            online_artwork_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
            description: row.get(8)?,
            musicbrainz_id: row.get(9)?,
            label: row.get(10)?,
            country: row.get(11)?,
            barcode: row.get(12)?,
            album_type: row.get(13)?,
            release_status: row.get(14)?,
        })
    })?;

    for album in album_results {
        albums.push(album?);
    }

    // Search artists
    let mut artists = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, name, name_sort, bio, country, genre, style, mood,
                formed_year, born_year, died_year, disbanded, musicbrainz_id, theaudiodb_id
         FROM artists
         WHERE name LIKE ?1
         LIMIT ?2",
    )?;

    let artist_results = stmt.query_map(params![search_pattern, limit], |row| {
        Ok(Artist {
            id: row.get(0)?,
            name: row.get(1)?,
            name_sort: row.get(2)?,
            bio: row.get(3)?,
            country: row.get(4)?,
            genre: row.get(5)?,
            style: row.get(6)?,
            mood: row.get(7)?,
            formed_year: row.get(8)?,
            born_year: row.get(9)?,
            died_year: row.get(10)?,
            disbanded: row.get(11)?,
            musicbrainz_id: row.get(12)?,
            theaudiodb_id: row.get(13)?,
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
    let playlist = conn
        .query_row(
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
        )
        .optional()?;

    if let Some((id, name, description, created_at, updated_at)) = playlist {
        // Get track IDs in order
        let mut stmt = conn.prepare(
            "SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position",
        )?;
        let tracks: Result<Vec<i64>> = stmt.query_map(params![id], |row| row.get(0))?.collect();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Rating;
    use crate::test_helpers::TestEnv;

    // ====================================================================
    // Source tests
    // ====================================================================

    #[test]
    fn test_insert_and_get_source() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let id = insert_source(
            &conn,
            "My Library",
            SourceType::Disk,
            Some(&PathBuf::from("/music")),
        )
        .unwrap();
        assert!(id > 0);
        let source = get_source(&conn, id).unwrap().unwrap();
        assert_eq!(source.name, "My Library");
        assert_eq!(source.source_type, SourceType::Disk);
        assert_eq!(source.path, Some(PathBuf::from("/music")));
    }

    #[test]
    fn test_list_sources() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        insert_source(&conn, "B Library", SourceType::Disk, None).unwrap();
        insert_source(&conn, "A Library", SourceType::Ipod, None).unwrap();
        let sources = list_sources(&conn).unwrap();
        assert_eq!(sources.len(), 2);
        // Should be ordered by name
        assert_eq!(sources[0].name, "A Library");
        assert_eq!(sources[1].name, "B Library");
    }

    #[test]
    fn test_source_without_path() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let id = insert_source(&conn, "iPod", SourceType::Ipod, None).unwrap();
        let source = get_source(&conn, id).unwrap().unwrap();
        assert_eq!(source.source_type, SourceType::Ipod);
        assert!(source.path.is_none());
    }

    #[test]
    fn test_get_source_not_found() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let result = get_source(&conn, 9999).unwrap();
        assert!(result.is_none());
    }

    // ====================================================================
    // Artist tests
    // ====================================================================

    #[test]
    fn test_insert_and_get_artist() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let id = insert_artist(&conn, "The Beatles", Some("Beatles, The")).unwrap();
        assert!(id > 0);
        let artist = get_artist(&conn, id).unwrap().unwrap();
        assert_eq!(artist.name, "The Beatles");
        assert_eq!(artist.name_sort, Some("Beatles, The".to_string()));
    }

    #[test]
    fn test_insert_artist_returns_existing() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let id1 = insert_artist(&conn, "Pink Floyd", None).unwrap();
        let id2 = insert_artist(&conn, "Pink Floyd", Some("sort")).unwrap();
        // Should return the same ID (ON CONFLICT DO NOTHING)
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_list_artists() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        insert_artist(&conn, "Zappa, Frank", Some("Zappa, Frank")).unwrap();
        insert_artist(&conn, "Beatles, The", Some("Beatles, The")).unwrap();
        insert_artist(&conn, "Miles Davis", Some("Davis, Miles")).unwrap();
        let artists = list_artists(&conn).unwrap();
        assert_eq!(artists.len(), 3);
        // Ordered by COALESCE(name_sort, name)
        assert_eq!(artists[0].name, "Beatles, The");
        assert_eq!(artists[1].name, "Miles Davis");
        assert_eq!(artists[2].name, "Zappa, Frank");
    }

    #[test]
    fn test_get_artist_albums() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist_id = insert_artist(&conn, "Test Artist", None).unwrap();
        insert_album(&conn, "Album A", artist_id, Some(2020)).unwrap();
        insert_album(&conn, "Album B", artist_id, Some(2021)).unwrap();
        let albums = get_artist_albums(&conn, artist_id).unwrap();
        assert_eq!(albums.len(), 2);
    }

    #[test]
    fn test_get_artist_albums_includes_track_artist() {
        // Albums where the artist only appears as a track artist (not album artist)
        // should also be returned — covers "feat." artists and VA-upgraded albums.
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source_id = insert_source(&conn, "Library", SourceType::Disk, None).unwrap();
        let album_artist_id = insert_artist(&conn, "Album Artist", None).unwrap();
        let feat_artist_id = insert_artist(&conn, "Featured Artist", None).unwrap();
        let album_id = insert_album(&conn, "Collab Album", album_artist_id, Some(2022)).unwrap();
        insert_track(
            &conn,
            "Collab Track",
            Some(album_id),
            feat_artist_id,
            source_id,
            std::path::Path::new("/music/collab.flac"),
            180_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        // feat_artist_id is not the album artist but has a track on the album
        let albums = get_artist_albums(&conn, feat_artist_id).unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].title, "Collab Album");

        // album_artist_id still gets the album too
        let albums = get_artist_albums(&conn, album_artist_id).unwrap();
        assert_eq!(albums.len(), 1);
    }

    // ====================================================================
    // Album tests
    // ====================================================================

    #[test]
    fn test_insert_and_get_album() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist_id = insert_artist(&conn, "Artist", None).unwrap();
        let album_id = insert_album(&conn, "My Album", artist_id, Some(2023)).unwrap();
        assert!(album_id > 0);
        let album = get_album(&conn, album_id).unwrap().unwrap();
        assert_eq!(album.title, "My Album");
        assert_eq!(album.artist_id, artist_id);
        assert_eq!(album.year, Some(2023));
    }

    #[test]
    fn test_insert_album_returns_existing() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist_id = insert_artist(&conn, "Artist", None).unwrap();
        let id1 = insert_album(&conn, "Same Album", artist_id, Some(2020)).unwrap();
        let id2 = insert_album(&conn, "Same Album", artist_id, Some(2021)).unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_list_albums() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist_id = insert_artist(&conn, "Artist", None).unwrap();
        insert_album(&conn, "Zebra Album", artist_id, None).unwrap();
        insert_album(&conn, "Alpha Album", artist_id, None).unwrap();
        let albums = list_albums(&conn, None, None, None, None, None).unwrap();
        assert_eq!(albums.len(), 2);
        assert_eq!(albums[0].title, "Alpha Album");
    }

    #[test]
    fn test_album_track_count() {
        let env = TestEnv::new();
        let (_, _artist_id, album_id, _, _, _) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        let tracks = get_album_tracks(&conn, album_id).unwrap();
        assert_eq!(tracks.len(), 2);
    }

    #[test]
    fn test_find_album_by_title_returns_id_and_artist_id() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist_id = insert_artist(&conn, "Akhenaton", None).unwrap();
        let album_id = insert_album(&conn, "Sol Invictus", artist_id, Some(2001)).unwrap();
        let found = find_album_by_title(&conn, "Sol Invictus").unwrap();
        assert_eq!(found, Some((album_id, artist_id)));
    }

    #[test]
    fn test_find_album_by_title_returns_none_when_missing() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        assert!(find_album_by_title(&conn, "Unknown").unwrap().is_none());
    }

    #[test]
    fn test_update_album_artist() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist1 = insert_artist(&conn, "Akhenaton", None).unwrap();
        let artist2 = insert_artist(&conn, "Various Artists", None).unwrap();
        let album_id = insert_album(&conn, "Compilation", artist1, None).unwrap();
        update_album_artist(&conn, album_id, artist2).unwrap();
        let (_, returned_artist) = find_album_by_title(&conn, "Compilation").unwrap().unwrap();
        assert_eq!(returned_artist, artist2);
    }

    #[test]
    fn test_update_album_rating() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist_id = insert_artist(&conn, "Artist", None).unwrap();
        let album_id = insert_album(&conn, "Album", artist_id, None).unwrap();
        update_album_rating(&conn, album_id, 4).unwrap();
        let album = get_album(&conn, album_id).unwrap().unwrap();
        assert_eq!(album.rating, Rating(4));
    }

    // ====================================================================
    // Genre tests
    // ====================================================================

    #[test]
    fn test_insert_and_get_genre() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let id = insert_genre(&conn, "Jazz").unwrap();
        assert!(id > 0);
        let genre = get_genre(&conn, id).unwrap().unwrap();
        assert_eq!(genre.name, "Jazz");
    }

    #[test]
    fn test_insert_genre_returns_existing() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let id1 = insert_genre(&conn, "Rock").unwrap();
        let id2 = insert_genre(&conn, "Rock").unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_list_genres_with_count() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        let genres = list_genres_with_count(&conn).unwrap();
        assert_eq!(genres.len(), 1);
        assert_eq!(genres[0].0.name, "Rock");
        assert_eq!(genres[0].1, 2); // 2 tracks
    }

    // ====================================================================
    // Track tests
    // ====================================================================

    #[test]
    fn test_insert_and_get_track() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source_id = insert_source(&conn, "Library", SourceType::Disk, None).unwrap();
        let artist_id = insert_artist(&conn, "Artist", None).unwrap();
        let album_id = insert_album(&conn, "Album", artist_id, None).unwrap();
        let track_id = insert_track(
            &conn,
            "My Track",
            Some(album_id),
            artist_id,
            source_id,
            &PathBuf::from("/music/track.flac"),
            200_000,
            Some(1),
            Some(44100),
            Some(16),
            AudioFormat::Flac,
            10_000_000,
        )
        .unwrap();
        assert!(track_id > 0);
        let track = get_track(&conn, track_id).unwrap().unwrap();
        assert_eq!(track.title, "My Track");
        assert_eq!(track.file_type, AudioFormat::Flac);
        assert_eq!(track.duration.as_millis(), 200_000);
    }

    #[test]
    fn test_list_tracks_by_album() {
        let env = TestEnv::new();
        let (_, _, album_id, _, t1, t2) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        let tracks = get_album_tracks(&conn, album_id).unwrap();
        assert_eq!(tracks.len(), 2);
        let ids: Vec<i64> = tracks.iter().map(|t| t.id).collect();
        assert!(ids.contains(&t1));
        assert!(ids.contains(&t2));
    }

    #[test]
    fn test_insert_duplicate_track_path_updates() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source_id = insert_source(&conn, "Library", SourceType::Disk, None).unwrap();
        let artist_id = insert_artist(&conn, "Artist", None).unwrap();
        let path = PathBuf::from("/music/track.flac");

        insert_track(
            &conn,
            "Old Title",
            None,
            artist_id,
            source_id,
            &path,
            100_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000,
        )
        .unwrap();
        insert_track(
            &conn,
            "New Title",
            None,
            artist_id,
            source_id,
            &path,
            200_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            2_000,
        )
        .unwrap();

        let conn2 = env.pool.get().unwrap();
        let count: i64 = conn2
            .query_row(
                "SELECT COUNT(*) FROM tracks WHERE source_id = ?1",
                [source_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1); // only one row (upserted)
    }

    #[test]
    fn test_delete_tracks_by_source() {
        let env = TestEnv::new();
        let (source_id, _, _, _, _, _) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();

        conn.execute("DELETE FROM tracks WHERE source_id = ?1", [source_id])
            .unwrap();
        let tracks = get_source_tracks(&conn, source_id).unwrap();
        assert!(tracks.is_empty());
    }

    #[test]
    fn test_track_with_all_metadata() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source_id = insert_source(&conn, "Library", SourceType::Disk, None).unwrap();
        let artist_id = insert_artist(&conn, "Artist", None).unwrap();
        let album_id = insert_album(&conn, "Album", artist_id, Some(2020)).unwrap();
        let track_id = insert_track(
            &conn,
            "Full Track",
            Some(album_id),
            artist_id,
            source_id,
            &PathBuf::from("/music/full.mp3"),
            300_000,
            Some(5),
            Some(48000),
            Some(24),
            AudioFormat::Mp3,
            15_000_000,
        )
        .unwrap();
        let track = get_track(&conn, track_id).unwrap().unwrap();
        assert_eq!(track.track_number, Some(5));
        assert_eq!(track.sample_rate, Some(48000));
        assert_eq!(track.bit_depth, Some(24));
        assert_eq!(track.file_type, AudioFormat::Mp3);
        assert_eq!(track.file_size, 15_000_000);
    }

    #[test]
    fn test_update_track_rating() {
        let env = TestEnv::new();
        let (_, _, _, _, track_id, _) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        update_track_rating(&conn, track_id, 5).unwrap();
        let track = get_track(&conn, track_id).unwrap().unwrap();
        assert_eq!(track.rating, Rating(5));
    }

    #[test]
    fn test_link_track_genre() {
        let env = TestEnv::new();
        let (_, _, _, genre_id, track_id, _) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        // Genre already linked in seed, linking again should not error (ON CONFLICT DO NOTHING)
        link_track_genre(&conn, track_id, genre_id).unwrap();
        let genres = get_album_genres(&conn, 1).unwrap();
        assert_eq!(genres.len(), 1);
    }

    // ====================================================================
    // Playlist tests
    // ====================================================================

    #[test]
    fn test_create_playlist() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let id = create_playlist(&conn, "My Playlist", Some("A test playlist")).unwrap();
        assert!(id > 0);
        let pl = get_playlist(&conn, id).unwrap().unwrap();
        assert_eq!(pl.name, "My Playlist");
        assert_eq!(pl.description, Some("A test playlist".to_string()));
        assert!(pl.tracks.is_empty());
    }

    #[test]
    fn test_add_tracks_to_playlist() {
        let env = TestEnv::new();
        let (_, _, _, _, t1, t2) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        let pl_id = create_playlist(&conn, "Playlist", None).unwrap();
        add_track_to_playlist(&conn, pl_id, t1).unwrap();
        add_track_to_playlist(&conn, pl_id, t2).unwrap();
        let pl = get_playlist(&conn, pl_id).unwrap().unwrap();
        assert_eq!(pl.tracks, vec![t1, t2]);
    }

    #[test]
    fn test_remove_track_from_playlist() {
        let env = TestEnv::new();
        let (_, _, _, _, t1, t2) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        let pl_id = create_playlist(&conn, "Playlist", None).unwrap();
        add_track_to_playlist(&conn, pl_id, t1).unwrap();
        add_track_to_playlist(&conn, pl_id, t2).unwrap();

        // Remove first track (position 0)
        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND position = 0",
            [pl_id],
        )
        .unwrap();
        let pl = get_playlist(&conn, pl_id).unwrap().unwrap();
        assert_eq!(pl.tracks.len(), 1);
        assert_eq!(pl.tracks[0], t2);
    }

    #[test]
    fn test_delete_playlist() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let pl_id = create_playlist(&conn, "To Delete", None).unwrap();
        conn.execute("DELETE FROM playlists WHERE id = ?1", [pl_id])
            .unwrap();
        let result = get_playlist(&conn, pl_id).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_playlist_tracks_ordered_by_position() {
        let env = TestEnv::new();
        let (_, _, _, _, t1, t2) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        let pl_id = create_playlist(&conn, "Ordered", None).unwrap();
        add_track_to_playlist(&conn, pl_id, t1).unwrap();
        add_track_to_playlist(&conn, pl_id, t2).unwrap();
        let pl = get_playlist(&conn, pl_id).unwrap().unwrap();
        // Should be in insertion order (positions 0, 1)
        assert_eq!(pl.tracks[0], t1);
        assert_eq!(pl.tracks[1], t2);
    }

    #[test]
    fn test_create_playlist_no_description() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let id = create_playlist(&conn, "Simple", None).unwrap();
        let pl = get_playlist(&conn, id).unwrap().unwrap();
        assert_eq!(pl.description, None);
    }

    // ====================================================================
    // Search FTS5 tests
    // ====================================================================

    #[test]
    fn test_search_tracks_by_title() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        let (tracks, _, _) = search_library(&conn, "Track One", 10).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "Track One");
    }

    #[test]
    fn test_search_tracks_by_artist() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        let (tracks, _, artists) = search_library(&conn, "Test Artist", 10).unwrap();
        assert!(!tracks.is_empty() || !artists.is_empty());
    }

    #[test]
    fn test_search_returns_empty_for_no_match() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        let (tracks, albums, artists) = search_library(&conn, "xyznotfoundxyz", 10).unwrap();
        assert!(tracks.is_empty());
        assert!(albums.is_empty());
        assert!(artists.is_empty());
    }

    // ====================================================================
    // list_albums with filters
    // ====================================================================

    #[test]
    fn test_list_albums_filtered_by_artist() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist1 = insert_artist(&conn, "Artist One", None).unwrap();
        let artist2 = insert_artist(&conn, "Artist Two", None).unwrap();
        insert_album(&conn, "Album A1", artist1, None).unwrap();
        insert_album(&conn, "Album A2", artist1, None).unwrap();
        insert_album(&conn, "Album B1", artist2, None).unwrap();

        let albums = list_albums(&conn, Some(artist1), None, None, None, None).unwrap();
        assert_eq!(albums.len(), 2);
        assert!(albums.iter().all(|a| a.artist_id == artist1));
    }

    #[test]
    fn test_list_albums_filtered_by_min_rating() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist_id = insert_artist(&conn, "Artist", None).unwrap();
        let low = insert_album(&conn, "Low Rated", artist_id, None).unwrap();
        let high = insert_album(&conn, "High Rated", artist_id, None).unwrap();
        update_album_rating(&conn, low, 1).unwrap();
        update_album_rating(&conn, high, 4).unwrap();

        let albums = list_albums(&conn, None, None, Some(3), None, None).unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].id, high);
    }

    #[test]
    fn test_list_albums_with_limit_and_offset() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist_id = insert_artist(&conn, "Artist", None).unwrap();
        insert_album(&conn, "Album A", artist_id, None).unwrap();
        insert_album(&conn, "Album B", artist_id, None).unwrap();
        insert_album(&conn, "Album C", artist_id, None).unwrap();

        let page1 = list_albums(&conn, None, None, None, Some(2), Some(0)).unwrap();
        let page2 = list_albums(&conn, None, None, None, Some(2), Some(2)).unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 1);
        assert_ne!(page1[0].id, page2[0].id);
    }

    // ====================================================================
    // count_albums
    // ====================================================================

    #[test]
    fn test_count_albums_all() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist_id = insert_artist(&conn, "Artist", None).unwrap();
        insert_album(&conn, "Album A", artist_id, None).unwrap();
        insert_album(&conn, "Album B", artist_id, None).unwrap();

        let count = count_albums(&conn, None, None, None).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_count_albums_filtered_by_artist() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let a1 = insert_artist(&conn, "A1", None).unwrap();
        let a2 = insert_artist(&conn, "A2", None).unwrap();
        insert_album(&conn, "X", a1, None).unwrap();
        insert_album(&conn, "Y", a1, None).unwrap();
        insert_album(&conn, "Z", a2, None).unwrap();

        assert_eq!(count_albums(&conn, Some(a1), None, None).unwrap(), 2);
        assert_eq!(count_albums(&conn, Some(a2), None, None).unwrap(), 1);
    }

    #[test]
    fn test_count_albums_filtered_by_min_rating() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist_id = insert_artist(&conn, "Artist", None).unwrap();
        let low = insert_album(&conn, "Low", artist_id, None).unwrap();
        let high = insert_album(&conn, "High", artist_id, None).unwrap();
        update_album_rating(&conn, low, 2).unwrap();
        update_album_rating(&conn, high, 5).unwrap();

        assert_eq!(count_albums(&conn, None, None, Some(4)).unwrap(), 1);
        assert_eq!(count_albums(&conn, None, None, Some(1)).unwrap(), 2);
    }

    // ====================================================================
    // get_genre_artists / get_album_genres / get_artist_genres
    // ====================================================================

    #[test]
    fn test_get_genre_artists() {
        let env = TestEnv::new();
        let (_, artist_id, _, genre_id, _, _) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();

        let artists = get_genre_artists(&conn, genre_id).unwrap();
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].id, artist_id);
    }

    #[test]
    fn test_get_genre_artists_empty_for_unknown_genre() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artists = get_genre_artists(&conn, 9999).unwrap();
        assert!(artists.is_empty());
    }

    #[test]
    fn test_get_album_genres() {
        let env = TestEnv::new();
        let (_, _, album_id, genre_id, _, _) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();

        let genres = get_album_genres(&conn, album_id).unwrap();
        assert_eq!(genres.len(), 1);
        assert_eq!(genres[0].id, genre_id);
        assert_eq!(genres[0].name, "Rock");
    }

    #[test]
    fn test_get_album_genres_empty_for_unknown_album() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let genres = get_album_genres(&conn, 9999).unwrap();
        assert!(genres.is_empty());
    }

    #[test]
    fn test_get_artist_genres() {
        let env = TestEnv::new();
        let (_, artist_id, _, genre_id, _, _) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();

        let genres = get_artist_genres(&conn, artist_id).unwrap();
        assert_eq!(genres.len(), 1);
        assert_eq!(genres[0].id, genre_id);
    }

    #[test]
    fn test_get_artist_genres_empty_for_unknown_artist() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let genres = get_artist_genres(&conn, 9999).unwrap();
        assert!(genres.is_empty());
    }

    // ====================================================================
    // get_source_tracks
    // ====================================================================

    #[test]
    fn test_get_source_tracks() {
        let env = TestEnv::new();
        let (source_id, _, _, _, t1, t2) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();

        let tracks = get_source_tracks(&conn, source_id).unwrap();
        assert_eq!(tracks.len(), 2);
        let ids: Vec<i64> = tracks.iter().map(|t| t.id).collect();
        assert!(ids.contains(&t1));
        assert!(ids.contains(&t2));
    }

    #[test]
    fn test_get_source_tracks_empty_for_unknown_source() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let tracks = get_source_tracks(&conn, 9999).unwrap();
        assert!(tracks.is_empty());
    }

    // ====================================================================
    // insert_track — upsert contract
    // ====================================================================

    #[test]
    fn test_insert_track_upsert_does_not_create_duplicate() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist = insert_artist(&conn, "A", None).unwrap();
        let path = PathBuf::from("/music/x.flac");

        insert_track(
            &conn,
            "Original",
            None,
            artist,
            source,
            &path,
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        insert_track(
            &conn,
            "Updated",
            None,
            artist,
            source,
            &path,
            90_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "re-inserting same source+path must not create a duplicate row"
        );
    }

    #[test]
    fn test_insert_track_upsert_updates_title() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist = insert_artist(&conn, "A", None).unwrap();
        let path = PathBuf::from("/music/x.flac");

        insert_track(
            &conn,
            "Original Title",
            None,
            artist,
            source,
            &path,
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        insert_track(
            &conn,
            "Updated Title",
            None,
            artist,
            source,
            &path,
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        let title: String = conn
            .query_row("SELECT title FROM tracks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            title, "Updated Title",
            "upsert must update the title column"
        );
    }

    #[test]
    fn test_insert_track_upsert_updates_duration() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist = insert_artist(&conn, "A", None).unwrap();
        let path = PathBuf::from("/music/x.flac");

        insert_track(
            &conn,
            "T",
            None,
            artist,
            source,
            &path,
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        insert_track(
            &conn,
            "T",
            None,
            artist,
            source,
            &path,
            180_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        let duration: i64 = conn
            .query_row("SELECT duration FROM tracks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(duration, 180_000, "upsert must update the duration column");
    }

    #[test]
    fn test_insert_track_upsert_updates_artist_id() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist1 = insert_artist(&conn, "Old Artist", None).unwrap();
        let artist2 = insert_artist(&conn, "New Artist", None).unwrap();
        let path = PathBuf::from("/music/x.flac");

        insert_track(
            &conn,
            "T",
            None,
            artist1,
            source,
            &path,
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        insert_track(
            &conn,
            "T",
            None,
            artist2,
            source,
            &path,
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        let stored_artist: i64 = conn
            .query_row("SELECT artist_id FROM tracks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            stored_artist, artist2,
            "upsert must update the artist_id column"
        );
    }

    // ====================================================================
    // get_album_tracks — sort order
    // ====================================================================

    #[test]
    fn test_get_album_tracks_sorted_by_track_number() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist = insert_artist(&conn, "A", None).unwrap();
        let album = insert_album(&conn, "Album", artist, None).unwrap();

        // Insert deliberately out of order
        let t3 = insert_track(
            &conn,
            "T3",
            Some(album),
            artist,
            source,
            &PathBuf::from("/t3.flac"),
            60_000,
            Some(3),
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        let t1 = insert_track(
            &conn,
            "T1",
            Some(album),
            artist,
            source,
            &PathBuf::from("/t1.flac"),
            60_000,
            Some(1),
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        let t2 = insert_track(
            &conn,
            "T2",
            Some(album),
            artist,
            source,
            &PathBuf::from("/t2.flac"),
            60_000,
            Some(2),
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        let ids: Vec<i64> = get_album_tracks(&conn, album)
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(
            ids,
            vec![t1, t2, t3],
            "tracks must be returned sorted by track_number ASC"
        );
    }

    #[test]
    fn test_get_album_tracks_disc_number_takes_precedence_over_track_number() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist = insert_artist(&conn, "A", None).unwrap();
        let album = insert_album(&conn, "2xCD", artist, None).unwrap();

        // Disc 2 track 1 inserted before Disc 1 track 5
        let d2t1 = insert_track(
            &conn,
            "D2T1",
            Some(album),
            artist,
            source,
            &PathBuf::from("/d2t1.flac"),
            60_000,
            Some(1),
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        let d1t5 = insert_track(
            &conn,
            "D1T5",
            Some(album),
            artist,
            source,
            &PathBuf::from("/d1t5.flac"),
            60_000,
            Some(5),
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();

        // Set disc numbers via SQL (insert_track doesn't expose disc_number)
        conn.execute("UPDATE tracks SET disc_number = 2 WHERE id = ?1", [d2t1])
            .unwrap();
        conn.execute("UPDATE tracks SET disc_number = 1 WHERE id = ?1", [d1t5])
            .unwrap();

        let ids: Vec<i64> = get_album_tracks(&conn, album)
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(
            ids,
            vec![d1t5, d2t1],
            "Disc 1 Track 5 must precede Disc 2 Track 1"
        );
    }

    // ====================================================================
    // count_albums — genre filter
    // ====================================================================

    #[test]
    fn test_count_albums_genre_filter_alone() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let source = insert_source(&conn, "s", SourceType::Disk, None).unwrap();
        let artist = insert_artist(&conn, "A", None).unwrap();
        let genre_rock = insert_genre(&conn, "Rock").unwrap();
        let genre_jazz = insert_genre(&conn, "Jazz").unwrap();
        let album_rock = insert_album(&conn, "Rock Album", artist, None).unwrap();
        let album_jazz = insert_album(&conn, "Jazz Album", artist, None).unwrap();

        let t_rock = insert_track(
            &conn,
            "R",
            Some(album_rock),
            artist,
            source,
            &PathBuf::from("/r.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        let t_jazz = insert_track(
            &conn,
            "J",
            Some(album_jazz),
            artist,
            source,
            &PathBuf::from("/j.flac"),
            60_000,
            None,
            None,
            None,
            AudioFormat::Flac,
            1_000_000,
        )
        .unwrap();
        link_track_genre(&conn, t_rock, genre_rock).unwrap();
        link_track_genre(&conn, t_jazz, genre_jazz).unwrap();

        assert_eq!(
            count_albums(&conn, None, Some(genre_rock), None).unwrap(),
            1,
            "genre=rock must count 1"
        );
        assert_eq!(
            count_albums(&conn, None, Some(genre_jazz), None).unwrap(),
            1,
            "genre=jazz must count 1"
        );
        assert_eq!(
            count_albums(&conn, None, None, None).unwrap(),
            2,
            "no filter must count all"
        );
    }

    #[test]
    fn test_count_albums_genre_filter_unknown_genre_returns_zero() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        let count = count_albums(&conn, None, Some(99999), None).unwrap();
        assert_eq!(count, 0, "unknown genre must yield 0, not an error");
    }

    // ====================================================================
    // list_albums — pagination edge cases
    // ====================================================================

    #[test]
    fn test_list_albums_offset_beyond_total_returns_empty() {
        let env = TestEnv::new();
        env.seed_basic_library(); // 1 album
        let conn = env.pool.get().unwrap();
        let albums = list_albums(&conn, None, None, None, Some(10), Some(999)).unwrap();
        assert!(
            albums.is_empty(),
            "offset beyond total must return an empty list"
        );
    }

    #[test]
    fn test_list_albums_limit_zero_returns_empty() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let conn = env.pool.get().unwrap();
        let albums = list_albums(&conn, None, None, None, Some(0), None).unwrap();
        assert!(albums.is_empty(), "limit=0 must return an empty list");
    }

    #[test]
    fn test_list_albums_pagination_pages_are_contiguous_and_non_overlapping() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist = insert_artist(&conn, "Art", None).unwrap();
        for i in 0..5u32 {
            insert_album(&conn, &format!("Album {:02}", i), artist, None).unwrap();
        }

        let page1 = list_albums(&conn, None, None, None, Some(2), Some(0)).unwrap();
        let page2 = list_albums(&conn, None, None, None, Some(2), Some(2)).unwrap();
        let page3 = list_albums(&conn, None, None, None, Some(2), Some(4)).unwrap();

        assert_eq!(page1.len(), 2, "page 1 must have 2 albums");
        assert_eq!(page2.len(), 2, "page 2 must have 2 albums");
        assert_eq!(page3.len(), 1, "page 3 must have the remaining 1 album");

        let p1_ids: std::collections::HashSet<i64> = page1.iter().map(|a| a.id).collect();
        let p2_ids: std::collections::HashSet<i64> = page2.iter().map(|a| a.id).collect();
        assert!(
            p1_ids.is_disjoint(&p2_ids),
            "consecutive pages must not overlap"
        );
    }
}
