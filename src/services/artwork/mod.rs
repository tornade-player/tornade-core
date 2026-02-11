// Artwork fetching service module

mod client;
mod matching;

pub use client::{MusicBrainzClient, RateLimiter};
pub use matching::fuzzy_match;

use crate::db::DbPool;
use crate::utils::paths::AppPaths;
use crate::services::reports::ArtworkReport;
use chrono::Local;
use std::sync::{Arc, Mutex};
use serde::{Serialize, Deserialize};

/// Progress tracking for artwork fetching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtworkFetchProgress {
    pub total_items: u32,
    pub processed_items: u32,
    pub current_item: String,
    pub successful: u32,
    pub failed: u32,
}

impl ArtworkFetchProgress {
    pub fn new(total_items: u32) -> Self {
        Self {
            total_items,
            processed_items: 0,
            current_item: String::new(),
            successful: 0,
            failed: 0,
        }
    }

    pub fn update(&mut self, item: String, success: bool) {
        self.processed_items += 1;
        self.current_item = item;
        if success {
            self.successful += 1;
        } else {
            self.failed += 1;
        }
    }
}

/// Main artwork service
#[derive(Clone)]
pub struct ArtworkService {
    pool: DbPool,
    app_paths: AppPaths,
    http_client: reqwest::Client,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    fetch_progress: Arc<Mutex<Option<ArtworkFetchProgress>>>,
    fetch_cancelled: Arc<Mutex<bool>>,
}

impl ArtworkService {
    /// Create a new artwork service
    pub fn new(pool: DbPool, app_paths: AppPaths) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))         // Hard timeout per request
            .connect_timeout(std::time::Duration::from_secs(5))  // Connection timeout
            .pool_max_idle_per_host(2)                           // Limit connections per host
            .user_agent("Tornade-Music-Player/1.0 ( contact@tornade.app )")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            pool,
            app_paths,
            http_client,
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(1100))), // 1.1 req/sec (safe margin)
            fetch_progress: Arc::new(Mutex::new(None)),
            fetch_cancelled: Arc::new(Mutex::new(false)),
        }
    }

    /// Get current fetch progress
    pub fn get_progress(&self) -> Option<ArtworkFetchProgress> {
        self.fetch_progress.lock().unwrap().clone()
    }

    /// Cancel ongoing fetch operation
    pub fn cancel_fetch(&self) {
        *self.fetch_cancelled.lock().unwrap() = true;
    }

    /// Check if fetch was cancelled
    fn is_cancelled(&self) -> bool {
        *self.fetch_cancelled.lock().unwrap()
    }

    /// Reset cancellation flag
    fn reset_cancel(&self) {
        *self.fetch_cancelled.lock().unwrap() = false;
    }

    /// Fetch artwork for all albums (and optionally artists)
    pub async fn fetch_all_artwork(&self, fetch_artists: bool) -> Result<(), String> {
        self.reset_cancel();
        let start_time = Local::now();

        // Get albums without online artwork
        let albums = self.get_albums_without_artwork()?;
        let total_albums = albums.len();
        let total_items = albums.len() as u32 + if fetch_artists {
            self.get_artists_without_photos()?.len() as u32
        } else {
            0
        };

        // Initialize progress
        {
            let mut progress = self.fetch_progress.lock().unwrap();
            *progress = Some(ArtworkFetchProgress::new(total_items));
        }

        // Track failures
        let mut albums_failed = Vec::new();
        let mut albums_successful = 0;

        // Create MusicBrainz client
        let mb_client = MusicBrainzClient::new(
            self.http_client.clone(),
            self.rate_limiter.clone(),
        );

        // Fetch album artwork
        for album in albums {
            if self.is_cancelled() {
                break;
            }

            let album_name = format!("{} - {}", album.artist_name, album.title);

            // Check if artwork already exists (skip if already downloaded during this session)
            if self.has_album_artwork(album.id) {
                albums_successful += 1;
                let mut progress = self.fetch_progress.lock().unwrap();
                if let Some(ref mut p) = *progress {
                    p.update(album_name, true);
                }
                continue;
            }

            let success = self.fetch_album_artwork_internal(&mb_client, album.id, &album.title, &album.artist_name).await;

            if success {
                albums_successful += 1;
            } else {
                albums_failed.push((album_name.clone(), "Not found or download failed".to_string()));
            }

            let mut progress = self.fetch_progress.lock().unwrap();
            if let Some(ref mut p) = *progress {
                p.update(album_name, success);
            }
        }

        // Track artist failures
        let mut artists_failed = Vec::new();
        let mut artists_successful = 0;
        let mut total_artists = 0;

        // Fetch artist photos if requested
        if fetch_artists {
            let artists = self.get_artists_without_photos()?;
            total_artists = artists.len();

            for artist in artists {
                if self.is_cancelled() {
                    break;
                }

                // Check if photo already exists (skip if already downloaded during this session)
                if self.has_artist_photo(artist.id) {
                    artists_successful += 1;
                    let mut progress = self.fetch_progress.lock().unwrap();
                    if let Some(ref mut p) = *progress {
                        p.update(artist.name, true);
                    }
                    continue;
                }

                let success = self.fetch_artist_photo_internal(&mb_client, artist.id, &artist.name).await;

                if success {
                    artists_successful += 1;
                } else {
                    artists_failed.push((artist.name.clone(), "Not found or download failed".to_string()));
                }

                let mut progress = self.fetch_progress.lock().unwrap();
                if let Some(ref mut p) = *progress {
                    p.update(artist.name, success);
                }
            }
        }

        // Generate report
        let mut report = ArtworkReport::new(start_time);
        report.end_time = Local::now();
        report.total_albums = total_albums;
        report.albums_successful = albums_successful;
        report.albums_failed = albums_failed;
        report.total_artists = total_artists;
        report.artists_successful = artists_successful;
        report.artists_failed = artists_failed;

        // Try to save report (non-fatal if it fails)
        match report.save(&self.app_paths.reports_dir()) {
            Ok(path) => log::info!("Artwork scraping report saved to: {:?}", path),
            Err(e) => log::warn!("Failed to save artwork report: {}", e),
        }

        Ok(())
    }

    /// Fetch artwork for a specific album
    pub async fn fetch_album_artwork(&self, album_id: i64) -> Result<(), String> {
        let album = self.get_album_info(album_id)?;

        let mb_client = MusicBrainzClient::new(
            self.http_client.clone(),
            self.rate_limiter.clone(),
        );

        self.fetch_album_artwork_internal(&mb_client, album_id, &album.title, &album.artist_name).await;
        Ok(())
    }

    /// Fetch photo for a specific artist
    pub async fn fetch_artist_photo(&self, artist_id: i64) -> Result<(), String> {
        let artist = self.get_artist_info(artist_id)?;

        let mb_client = MusicBrainzClient::new(
            self.http_client.clone(),
            self.rate_limiter.clone(),
        );

        self.fetch_artist_photo_internal(&mb_client, artist_id, &artist.name).await;
        Ok(())
    }

    /// Internal method to fetch album artwork
    async fn fetch_album_artwork_internal(
        &self,
        mb_client: &MusicBrainzClient,
        album_id: i64,
        album_title: &str,
        artist_name: &str,
    ) -> bool {
        match mb_client.search_album_artwork(album_title, artist_name).await {
            Ok(Some(image_data)) => {
                // Save image
                let file_path = self.app_paths.album_artwork_dir().join(format!("{}.jpg", album_id));
                if let Err(e) = std::fs::write(&file_path, &image_data) {
                    log::error!("Failed to save album artwork for {}: {}", album_id, e);
                    return false;
                }

                // Update database
                if let Err(e) = self.update_album_artwork(album_id, file_path.to_string_lossy().to_string()) {
                    log::error!("Failed to update database for album {}: {}", album_id, e);
                    return false;
                }

                log::info!("Fetched artwork for album {} - {}", artist_name, album_title);
                true
            }
            Ok(None) => {
                log::warn!("No artwork found for album {} - {}", artist_name, album_title);
                false
            }
            Err(e) => {
                log::error!("Error fetching artwork for album {} - {}: {}", artist_name, album_title, e);
                false
            }
        }
    }

    /// Internal method to fetch artist photo
    async fn fetch_artist_photo_internal(
        &self,
        mb_client: &MusicBrainzClient,
        artist_id: i64,
        artist_name: &str,
    ) -> bool {
        match mb_client.search_artist_photo(artist_name).await {
            Ok(Some(image_data)) => {
                // Save image
                let file_path = self.app_paths.artist_photo_dir().join(format!("{}.jpg", artist_id));
                if let Err(e) = std::fs::write(&file_path, &image_data) {
                    log::error!("Failed to save artist photo for {}: {}", artist_id, e);
                    return false;
                }

                // Update database
                if let Err(e) = self.update_artist_photo(artist_id, file_path.to_string_lossy().to_string()) {
                    log::error!("Failed to update database for artist {}: {}", artist_id, e);
                    return false;
                }

                log::info!("Fetched photo for artist {}", artist_name);
                true
            }
            Ok(None) => {
                log::warn!("No photo found for artist {}", artist_name);
                false
            }
            Err(e) => {
                log::error!("Error fetching photo for artist {}: {}", artist_name, e);
                false
            }
        }
    }

    /// Get albums without online artwork
    fn get_albums_without_artwork(&self) -> Result<Vec<AlbumInfo>, String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT a.id, a.title, ar.name as artist_name
                 FROM albums a
                 JOIN artists ar ON a.artist_id = ar.id
                 WHERE a.online_artwork_path IS NULL
                 ORDER BY ar.name, a.title",
            )
            .map_err(|e| e.to_string())?;

        let albums = stmt
            .query_map([], |row| {
                Ok(AlbumInfo {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist_name: row.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(albums)
    }

    /// Get artists without photos
    fn get_artists_without_photos(&self) -> Result<Vec<ArtistInfo>, String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name
                 FROM artists
                 WHERE photo_path IS NULL
                 ORDER BY name",
            )
            .map_err(|e| e.to_string())?;

        let artists = stmt
            .query_map([], |row| {
                Ok(ArtistInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(artists)
    }

    /// Check if an album already has artwork
    fn has_album_artwork(&self, album_id: i64) -> bool {
        let conn = match self.pool.get() {
            Ok(conn) => conn,
            Err(_) => return false,
        };

        let result: Result<Option<String>, _> = conn.query_row(
            "SELECT online_artwork_path FROM albums WHERE id = ?1",
            [album_id],
            |row| row.get(0),
        );

        matches!(result, Ok(Some(_)))
    }

    /// Check if an artist already has a photo
    fn has_artist_photo(&self, artist_id: i64) -> bool {
        let conn = match self.pool.get() {
            Ok(conn) => conn,
            Err(_) => return false,
        };

        let result: Result<Option<String>, _> = conn.query_row(
            "SELECT photo_path FROM artists WHERE id = ?1",
            [artist_id],
            |row| row.get(0),
        );

        matches!(result, Ok(Some(_)))
    }

    /// Get album info
    fn get_album_info(&self, album_id: i64) -> Result<AlbumInfo, String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        let result = conn
            .query_row(
                "SELECT a.id, a.title, ar.name as artist_name
                 FROM albums a
                 JOIN artists ar ON a.artist_id = ar.id
                 WHERE a.id = ?1",
                [album_id],
                |row| {
                    Ok(AlbumInfo {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        artist_name: row.get(2)?,
                    })
                },
            )
            .map_err(|e| e.to_string())?;

        Ok(result)
    }

    /// Get artist info
    fn get_artist_info(&self, artist_id: i64) -> Result<ArtistInfo, String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        let result = conn
            .query_row(
                "SELECT id, name FROM artists WHERE id = ?1",
                [artist_id],
                |row| {
                    Ok(ArtistInfo {
                        id: row.get(0)?,
                        name: row.get(1)?,
                    })
                },
            )
            .map_err(|e| e.to_string())?;

        Ok(result)
    }

    /// Update album artwork in database
    fn update_album_artwork(&self, album_id: i64, path: String) -> Result<(), String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE albums SET online_artwork_path = ?1, artwork_source = 'musicbrainz', artwork_fetched_at = CURRENT_TIMESTAMP WHERE id = ?2",
            rusqlite::params![path, album_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Update artist photo in database
    fn update_artist_photo(&self, artist_id: i64, path: String) -> Result<(), String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE artists SET photo_path = ?1, photo_source = 'musicbrainz', photo_fetched_at = CURRENT_TIMESTAMP WHERE id = ?2",
            rusqlite::params![path, artist_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[derive(Debug)]
struct AlbumInfo {
    id: i64,
    title: String,
    artist_name: String,
}

#[derive(Debug)]
struct ArtistInfo {
    id: i64,
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TestEnv;
    use crate::db::queries;

    // ArtworkFetchProgress tests

    #[test]
    fn test_progress_new() {
        let p = ArtworkFetchProgress::new(10);
        assert_eq!(p.total_items, 10);
        assert_eq!(p.processed_items, 0);
        assert_eq!(p.successful, 0);
        assert_eq!(p.failed, 0);
        assert!(p.current_item.is_empty());
    }

    #[test]
    fn test_progress_update_success() {
        let mut p = ArtworkFetchProgress::new(5);
        p.update("Album A".to_string(), true);
        assert_eq!(p.processed_items, 1);
        assert_eq!(p.successful, 1);
        assert_eq!(p.failed, 0);
        assert_eq!(p.current_item, "Album A");
    }

    #[test]
    fn test_progress_update_failure() {
        let mut p = ArtworkFetchProgress::new(5);
        p.update("Album B".to_string(), false);
        assert_eq!(p.processed_items, 1);
        assert_eq!(p.successful, 0);
        assert_eq!(p.failed, 1);
    }

    #[test]
    fn test_progress_multiple_updates() {
        let mut p = ArtworkFetchProgress::new(3);
        p.update("A".to_string(), true);
        p.update("B".to_string(), false);
        p.update("C".to_string(), true);
        assert_eq!(p.processed_items, 3);
        assert_eq!(p.successful, 2);
        assert_eq!(p.failed, 1);
        assert_eq!(p.current_item, "C");
    }

    // ArtworkService DB method tests

    fn setup_artwork_env() -> (TestEnv, i64, i64) {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist_id = queries::insert_artist(&conn, "Pink Floyd", None).unwrap();
        let album_id = queries::insert_album(&conn, "The Wall", artist_id, Some(1979)).unwrap();
        (env, artist_id, album_id)
    }

    #[test]
    fn test_get_albums_without_artwork() {
        let (env, _artist_id, _album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        let albums = service.get_albums_without_artwork().unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].title, "The Wall");
    }

    #[test]
    fn test_get_albums_excludes_fetched() {
        let (env, _artist_id, album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());

        // Set artwork
        service.update_album_artwork(album_id, "/path/to/art.jpg".to_string()).unwrap();

        let albums = service.get_albums_without_artwork().unwrap();
        assert_eq!(albums.len(), 0);
    }

    #[test]
    fn test_has_album_artwork_false() {
        let (env, _artist_id, album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        assert!(!service.has_album_artwork(album_id));
    }

    #[test]
    fn test_has_album_artwork_true() {
        let (env, _artist_id, album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        service.update_album_artwork(album_id, "/art.jpg".to_string()).unwrap();
        assert!(service.has_album_artwork(album_id));
    }

    #[test]
    fn test_update_album_artwork() {
        let (env, _artist_id, album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        service.update_album_artwork(album_id, "/path/art.jpg".to_string()).unwrap();

        let conn = env.pool.get().unwrap();
        let path: String = conn
            .query_row("SELECT online_artwork_path FROM albums WHERE id = ?1", [album_id], |r| r.get(0))
            .unwrap();
        assert_eq!(path, "/path/art.jpg");

        let source: String = conn
            .query_row("SELECT artwork_source FROM albums WHERE id = ?1", [album_id], |r| r.get(0))
            .unwrap();
        assert_eq!(source, "musicbrainz");
    }

    #[test]
    fn test_get_artists_without_photos() {
        let (env, _artist_id, _album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        let artists = service.get_artists_without_photos().unwrap();
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].name, "Pink Floyd");
    }

    #[test]
    fn test_has_artist_photo_false() {
        let (env, artist_id, _album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        assert!(!service.has_artist_photo(artist_id));
    }

    #[test]
    fn test_has_artist_photo_true() {
        let (env, artist_id, _album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        service.update_artist_photo(artist_id, "/photo.jpg".to_string()).unwrap();
        assert!(service.has_artist_photo(artist_id));
    }

    #[test]
    fn test_update_artist_photo() {
        let (env, artist_id, _album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        service.update_artist_photo(artist_id, "/path/photo.jpg".to_string()).unwrap();

        let conn = env.pool.get().unwrap();
        let path: String = conn
            .query_row("SELECT photo_path FROM artists WHERE id = ?1", [artist_id], |r| r.get(0))
            .unwrap();
        assert_eq!(path, "/path/photo.jpg");

        let source: String = conn
            .query_row("SELECT photo_source FROM artists WHERE id = ?1", [artist_id], |r| r.get(0))
            .unwrap();
        assert_eq!(source, "musicbrainz");
    }
}
