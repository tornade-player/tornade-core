// Playlist management service

use crate::db::{queries, DbPool};
use crate::models::Playlist;
use crate::services::error::PlaylistError;
use log::info;
use std::path::Path;

type Result<T> = std::result::Result<T, PlaylistError>;

pub struct PlaylistService {
    pool: DbPool,
}

impl PlaylistService {
    pub fn new(pool: DbPool) -> Self {
        PlaylistService { pool }
    }

    // ========================================================================
    // Playlist Management
    // ========================================================================

    /// Create a new playlist
    pub fn create_playlist(&self, name: &str, description: Option<&str>) -> Result<Playlist> {
        let conn = self.pool.get().map_err(|e| {
            PlaylistError::Database(rusqlite::Error::InvalidPath(
                std::path::PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        let playlist_id = queries::create_playlist(&conn, name, description)?;

        queries::get_playlist(&conn, playlist_id)?
            .ok_or(PlaylistError::PlaylistNotFound(playlist_id))
    }

    /// Get playlist by ID
    pub fn get_playlist(&self, id: i64) -> Result<Option<Playlist>> {
        let conn = self.pool.get().map_err(|e| {
            PlaylistError::Database(rusqlite::Error::InvalidPath(
                std::path::PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        Ok(queries::get_playlist(&conn, id)?)
    }

    /// List all playlists
    pub fn list_playlists(&self) -> Result<Vec<Playlist>> {
        let conn = self.pool.get().map_err(|e| {
            PlaylistError::Database(rusqlite::Error::InvalidPath(
                std::path::PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        let mut stmt = conn.prepare(
            "SELECT id, name, description, created_at, updated_at FROM playlists ORDER BY name"
        )?;

        let playlists = stmt.query_map([], |row| {
            let playlist_id = row.get::<_, i64>(0)?;
            let name = row.get::<_, String>(1)?;
            let description = row.get::<_, Option<String>>(2)?;
            let created_at = row.get::<_, String>(3)?;
            let updated_at = row.get::<_, String>(4)?;

            // Get tracks
            let mut track_stmt = conn.prepare(
                "SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position"
            )?;
            let tracks: rusqlite::Result<Vec<i64>> = track_stmt
                .query_map([playlist_id], |row| row.get(0))?
                .collect();

            Ok(Playlist {
                id: playlist_id,
                name,
                description,
                tracks: tracks?,
                created_at,
                updated_at,
            })
        })?;

        playlists.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(PlaylistError::Database)
    }

    /// Rename playlist
    pub fn rename_playlist(&self, id: i64, name: &str) -> Result<()> {
        let conn = self.pool.get().map_err(|e| {
            PlaylistError::Database(rusqlite::Error::InvalidPath(
                std::path::PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        conn.execute(
            "UPDATE playlists SET name = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            rusqlite::params![name, id],
        )?;

        info!("Renamed playlist {} to '{}'", id, name);
        Ok(())
    }

    /// Delete playlist
    pub fn delete_playlist(&self, id: i64) -> Result<()> {
        let conn = self.pool.get().map_err(|e| {
            PlaylistError::Database(rusqlite::Error::InvalidPath(
                std::path::PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        conn.execute("DELETE FROM playlists WHERE id = ?1", rusqlite::params![id])?;

        info!("Deleted playlist {}", id);
        Ok(())
    }

    /// Add tracks to playlist
    pub fn add_tracks(&self, playlist_id: i64, track_ids: Vec<i64>) -> Result<()> {
        let conn = self.pool.get().map_err(|e| {
            PlaylistError::Database(rusqlite::Error::InvalidPath(
                std::path::PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        for track_id in track_ids {
            queries::add_track_to_playlist(&conn, playlist_id, track_id)?;
        }

        info!("Added tracks to playlist {}", playlist_id);
        Ok(())
    }

    /// Remove track from playlist at position
    pub fn remove_track(&self, playlist_id: i64, position: usize) -> Result<()> {
        let conn = self.pool.get().map_err(|e| {
            PlaylistError::Database(rusqlite::Error::InvalidPath(
                std::path::PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND position = ?2",
            rusqlite::params![playlist_id, position as i64],
        )?;

        // Reorder remaining tracks
        conn.execute(
            "UPDATE playlist_tracks SET position = position - 1
             WHERE playlist_id = ?1 AND position > ?2",
            rusqlite::params![playlist_id, position as i64],
        )?;

        conn.execute(
            "UPDATE playlists SET updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            rusqlite::params![playlist_id],
        )?;

        info!("Removed track at position {} from playlist {}", position, playlist_id);
        Ok(())
    }

    /// Move track in playlist
    pub fn move_track(&self, playlist_id: i64, from: usize, to: usize) -> Result<()> {
        let conn = self.pool.get().map_err(|e| {
            PlaylistError::Database(rusqlite::Error::InvalidPath(
                std::path::PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        // Simple implementation: get all tracks, reorder, and update
        let playlist = queries::get_playlist(&conn, playlist_id)?
            .ok_or(PlaylistError::PlaylistNotFound(playlist_id))?;

        if from >= playlist.tracks.len() || to >= playlist.tracks.len() {
            return Err(PlaylistError::Database(rusqlite::Error::InvalidParameterName(
                "Invalid position".to_string()
            )));
        }

        let mut tracks = playlist.tracks;
        let track_id = tracks.remove(from);
        tracks.insert(to, track_id);

        // Delete all and recreate
        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            rusqlite::params![playlist_id],
        )?;

        for (pos, track_id) in tracks.iter().enumerate() {
            conn.execute(
                "INSERT INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                rusqlite::params![playlist_id, track_id, pos as i64],
            )?;
        }

        conn.execute(
            "UPDATE playlists SET updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            rusqlite::params![playlist_id],
        )?;

        info!("Moved track from position {} to {} in playlist {}", from, to, playlist_id);
        Ok(())
    }

    // ========================================================================
    // M3U Import/Export
    // ========================================================================

    /// Import playlist from M3U file
    pub fn import_m3u(&self, path: &Path) -> Result<Playlist> {
        let m3u_data = crate::utils::m3u::parse_m3u(path)?;

        let conn = self.pool.get().map_err(|e| {
            PlaylistError::Database(rusqlite::Error::InvalidPath(
                std::path::PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        // Create playlist
        let name = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported Playlist");

        let playlist_id = queries::create_playlist(&conn, name, Some("Imported from M3U"))?;

        // Find tracks by file path and add to playlist
        let mut added = 0;
        for track_path in m3u_data.tracks {
            // Try to find track in database by file path
            let track_id: rusqlite::Result<i64> = conn.query_row(
                "SELECT id FROM tracks WHERE file_path = ?1",
                rusqlite::params![track_path.to_string_lossy()],
                |row| row.get(0),
            );

            if let Ok(track_id) = track_id {
                queries::add_track_to_playlist(&conn, playlist_id, track_id)?;
                added += 1;
            }
        }

        info!("Imported M3U playlist '{}' with {} tracks", name, added);

        queries::get_playlist(&conn, playlist_id)?
            .ok_or(PlaylistError::PlaylistNotFound(playlist_id))
    }

    /// Export playlist to M3U file
    pub fn export_m3u(&self, playlist_id: i64, path: &Path) -> Result<()> {
        let conn = self.pool.get().map_err(|e| {
            PlaylistError::Database(rusqlite::Error::InvalidPath(
                std::path::PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        let playlist = queries::get_playlist(&conn, playlist_id)?
            .ok_or(PlaylistError::PlaylistNotFound(playlist_id))?;

        // Get track details
        let mut tracks = Vec::new();
        for track_id in playlist.tracks {
            if let Ok(Some(track)) = queries::get_track(&conn, track_id) {
                tracks.push(track);
            }
        }

        crate::utils::m3u::write_m3u(path, &tracks)?;

        info!("Exported playlist {} to M3U file: {:?}", playlist_id, path);
        Ok(())
    }
}
