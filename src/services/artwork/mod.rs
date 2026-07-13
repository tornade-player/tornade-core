//! Artwork fetching service — downloads album and artist images from MusicBrainz
//! and Cover Art Archive at 1 request/second to respect API rate limits.

mod client;
mod matching;

pub use client::{ArtistSearchResult, ArtworkSearchResult, MusicBrainzClient, RateLimiter};
pub use matching::fuzzy_match;

use crate::db::DbPool;
use crate::services::reports::ArtworkReport;
use crate::utils::MutexExt;
use crate::utils::paths::AppPaths;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Live progress state for an in-progress artwork fetch operation.
///
/// Retrieved via `ArtworkService::get_fetch_progress` from a background thread
/// while the fetch runs on the main service thread.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArtworkFetchProgress {
    /// Total number of albums/artists to process in this batch.
    pub total_items: u32,
    /// Number of items processed so far (including failures).
    pub processed_items: u32,
    /// Human-readable label of the item currently being fetched.
    pub current_item: String,
    /// Number of items for which artwork was successfully downloaded.
    pub successful: u32,
    /// Number of items for which the download failed or no artwork was found.
    pub failed: u32,
}

impl ArtworkFetchProgress {
    /// Create a new progress tracker for a batch of `total_items` items.
    pub fn new(total_items: u32) -> Self {
        Self {
            total_items,
            ..Default::default()
        }
    }

    /// Record the outcome of processing one item and advance the progress counter.
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

/// Downloads and caches album artwork and artist photos from MusicBrainz / Cover Art Archive.
///
/// All network calls respect a 1 req/s rate limit imposed by the MusicBrainz API
/// terms of service. Long-running fetch operations can be cancelled via
/// `ArtworkService::cancel_fetch` and monitored via `ArtworkService::get_fetch_progress`.
///
/// `ArtworkService` is `Clone` — all mutable state is `Arc<Mutex<…>>` so clones
/// share the same rate limiter and progress tracking.
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
            .timeout(std::time::Duration::from_secs(15)) // Hard timeout per request
            .connect_timeout(std::time::Duration::from_secs(5)) // Connection timeout
            .pool_max_idle_per_host(2) // Limit connections per host
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
        self.fetch_progress.lock_infallible().clone()
    }

    /// Cancel ongoing fetch operation
    pub fn cancel_fetch(&self) {
        *self.fetch_cancelled.lock_infallible() = true;
    }

    /// Check if fetch was cancelled
    fn is_cancelled(&self) -> bool {
        *self.fetch_cancelled.lock_infallible()
    }

    /// Reset cancellation flag
    fn reset_cancel(&self) {
        *self.fetch_cancelled.lock_infallible() = false;
    }

    /// Fetch artwork for all albums (and optionally artists).
    ///
    /// `force = true` resets all previous failure markers and re-scrapes everything,
    /// including albums that already have artwork.
    /// `force = false` (default) skips albums whose scrape was already attempted
    /// and failed, as well as albums that already have artwork.
    pub async fn fetch_all_artwork(&self, fetch_artists: bool, force: bool) -> Result<(), String> {
        self.reset_cancel();
        // One-shot: delete legacy {album_db_id}.jpg files created before MBID-based naming
        self.cleanup_legacy_artwork_if_needed();
        let start_time = Local::now();

        if force {
            self.reset_artwork_fetch_attempts()?;
        }

        // Get albums to scrape
        let albums = self.get_albums_for_scrape(force)?;
        let total_albums = albums.len();
        let total_items = albums.len() as u32
            + if fetch_artists {
                self.get_artists_for_scrape(force)?.len() as u32
            } else {
                0
            };

        // Initialize progress
        {
            let mut progress = self.fetch_progress.lock_infallible();
            *progress = Some(ArtworkFetchProgress::new(total_items));
        }

        // Track failures
        let mut albums_failed = Vec::new();
        let mut albums_successful = 0;

        // Create MusicBrainz client
        let mb_client = MusicBrainzClient::new(self.http_client.clone(), self.rate_limiter.clone());

        // Fetch album artwork
        for album in albums {
            if self.is_cancelled() {
                break;
            }

            let album_name = format!("{} - {}", album.artist_name, album.title);

            // Check if artwork already exists (skip if already downloaded during this session)
            if self.has_album_artwork(album.id) {
                albums_successful += 1;
                let mut progress = self.fetch_progress.lock_infallible();
                if let Some(ref mut p) = *progress {
                    p.update(album_name, true);
                }
                continue;
            }

            let success = self
                .fetch_album_artwork_internal(
                    &mb_client,
                    album.id,
                    &album.title,
                    &album.artist_name,
                )
                .await;

            if success {
                albums_successful += 1;
            } else {
                albums_failed.push((
                    album_name.clone(),
                    "Not found or download failed".to_string(),
                ));
            }

            let mut progress = self.fetch_progress.lock_infallible();
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
            let artists = self.get_artists_for_scrape(force)?;
            total_artists = artists.len();

            for artist in artists {
                if self.is_cancelled() {
                    break;
                }

                // Check if photo already exists (skip if already downloaded during this session)
                if self.has_artist_photo(artist.id) {
                    artists_successful += 1;
                    let mut progress = self.fetch_progress.lock_infallible();
                    if let Some(ref mut p) = *progress {
                        p.update(artist.name, true);
                    }
                    continue;
                }

                let success = self
                    .fetch_artist_photo_internal(&mb_client, artist.id, &artist.name)
                    .await;

                if success {
                    artists_successful += 1;
                } else {
                    artists_failed.push((
                        artist.name.clone(),
                        "Not found or download failed".to_string(),
                    ));
                }

                let mut progress = self.fetch_progress.lock_infallible();
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
            Ok(path) => log::info!("Artwork scraping report saved to: {path:?}"),
            Err(e) => log::warn!("Failed to save artwork report: {e}"),
        }

        Ok(())
    }

    /// Fetch artwork for a specific album
    pub async fn fetch_album_artwork(&self, album_id: i64) -> Result<(), String> {
        let album = self.get_album_info(album_id)?;

        let mb_client = MusicBrainzClient::new(self.http_client.clone(), self.rate_limiter.clone());

        self.fetch_album_artwork_internal(&mb_client, album_id, &album.title, &album.artist_name)
            .await;
        Ok(())
    }

    /// Fetch photo for a specific artist
    pub async fn fetch_artist_photo(&self, artist_id: i64) -> Result<(), String> {
        let artist = self.get_artist_info(artist_id)?;

        let mb_client = MusicBrainzClient::new(self.http_client.clone(), self.rate_limiter.clone());

        self.fetch_artist_photo_internal(&mb_client, artist_id, &artist.name)
            .await;
        Ok(())
    }

    /// Internal method to fetch album artwork and MB metadata.
    ///
    /// The artwork file is saved as `{mbid}.jpg` so re-scraping the same release
    /// never produces a duplicate file (stable across DB resets and rescans).
    async fn fetch_album_artwork_internal(
        &self,
        mb_client: &MusicBrainzClient,
        album_id: i64,
        album_title: &str,
        artist_name: &str,
    ) -> bool {
        match mb_client
            .search_album_artwork(album_title, artist_name)
            .await
        {
            Ok(Some(result)) => {
                let file_path = self
                    .app_paths
                    .album_artwork_dir()
                    .join(format!("{}.jpg", result.musicbrainz_id));

                // Only write the file if it doesn't already exist (e.g. another album shares
                // the same release, or the DB was reset but files were kept).
                if !file_path.exists()
                    && let Err(e) = std::fs::write(&file_path, &result.image_data)
                {
                    log::error!("Failed to save album artwork for {album_id}: {e}");
                    return false;
                }

                let path_str = file_path.to_string_lossy().to_string();
                if let Err(e) = self.update_album_mb_info(album_id, &path_str, &result) {
                    log::error!("Failed to update DB for album {album_id}: {e}");
                    return false;
                }

                log::info!(
                    "Fetched artwork for album {} - {} (mbid: {})",
                    artist_name,
                    album_title,
                    result.musicbrainz_id
                );
                true
            }
            Ok(None) => {
                log::warn!("No artwork found for album {artist_name} - {album_title}");
                let _ = self.mark_album_fetch_attempted(album_id);
                false
            }
            Err(e) => {
                log::error!("Error fetching artwork for album {artist_name} - {album_title}: {e}");
                let _ = self.mark_album_fetch_attempted(album_id);
                false
            }
        }
    }

    /// Internal method to fetch artist photo and metadata
    async fn fetch_artist_photo_internal(
        &self,
        mb_client: &MusicBrainzClient,
        artist_id: i64,
        artist_name: &str,
    ) -> bool {
        match mb_client.search_artist_photo(artist_name).await {
            Ok(Some(result)) => {
                // Save photo if available
                if let Some(ref image_data) = result.image_data {
                    let file_path = self
                        .app_paths
                        .artist_photo_dir()
                        .join(format!("{artist_id}.jpg"));
                    if let Err(e) = std::fs::write(&file_path, image_data) {
                        log::error!("Failed to save artist photo for {artist_id}: {e}");
                        // Continue to save metadata even if image write fails
                    } else if let Err(e) =
                        self.update_artist_photo(artist_id, &file_path.to_string_lossy())
                    {
                        log::error!("Failed to update photo path for artist {artist_id}: {e}");
                    }
                }

                // Always save metadata (also sets photo_fetched_at so we don't retry)
                if let Err(e) = self.update_artist_metadata(artist_id, &result) {
                    log::error!("Failed to update metadata for artist {artist_id}: {e}");
                }

                if result.image_data.is_some() {
                    log::info!("Fetched photo + metadata for artist {artist_name}");
                } else {
                    log::info!("Fetched metadata (no photo) for artist {artist_name}");
                }
                true // success = we found the artist (metadata is the value)
            }
            Ok(None) => {
                // Artist not in TheAudioDB — mark as attempted so we don't retry on every run
                log::warn!("Artist not found in TheAudioDB: {artist_name}");
                let _ = self.mark_artist_fetch_attempted(artist_id);
                false
            }
            Err(e) => {
                // Network/parse error — also mark as attempted to avoid hammering the API
                log::error!("Error fetching data for artist {artist_name}: {e}");
                let _ = self.mark_artist_fetch_attempted(artist_id);
                false
            }
        }
    }

    /// Mark an artist as "fetch attempted" without saving any data.
    /// Used when the artist is not found or an error occurs, to prevent endless retries.
    fn mark_artist_fetch_attempted(&self, artist_id: i64) -> Result<(), String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE artists SET photo_fetched_at = CURRENT_TIMESTAMP WHERE id = ?1",
            [artist_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Get albums to scrape.
    ///
    /// `force = true`: all albums (no filter — re-scrape even those with artwork).
    /// `force = false`: only albums where `online_artwork_path IS NULL`
    ///   AND `artwork_fetch_attempted_at IS NULL` (never tried or not yet failed).
    fn get_albums_for_scrape(&self, force: bool) -> Result<Vec<AlbumInfo>, String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        // force=true  → failed + new (no artwork, regardless of prior attempt)
        // force=false → new only (no artwork AND never attempted)
        let sql = if force {
            "SELECT a.id, a.title, ar.name as artist_name
             FROM albums a
             JOIN artists ar ON a.artist_id = ar.id
             WHERE a.online_artwork_path IS NULL
             ORDER BY ar.name, a.title"
        } else {
            "SELECT a.id, a.title, ar.name as artist_name
             FROM albums a
             JOIN artists ar ON a.artist_id = ar.id
             WHERE a.online_artwork_path IS NULL
               AND a.artwork_fetch_attempted_at IS NULL
             ORDER BY ar.name, a.title"
        };
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;

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

    /// Reset artwork fetch attempt markers for elements that still have no artwork.
    /// Elements that already succeeded keep their data untouched.
    fn reset_artwork_fetch_attempts(&self) -> Result<(), String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        conn.execute_batch(
            "UPDATE albums SET artwork_fetch_attempted_at = NULL WHERE online_artwork_path IS NULL;
             UPDATE artists SET photo_fetched_at = NULL WHERE photo_path IS NULL;",
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Mark an album as "fetch attempted" without saving artwork.
    /// Prevents endless retries on albums not found in MusicBrainz.
    fn mark_album_fetch_attempted(&self, album_id: i64) -> Result<(), String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE albums SET artwork_fetch_attempted_at = CURRENT_TIMESTAMP WHERE id = ?1",
            [album_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Get artists to scrape.
    ///
    /// `force = true`: failed + new (no photo, regardless of prior attempt).
    /// `force = false`: new only (never attempted).
    fn get_artists_for_scrape(&self, force: bool) -> Result<Vec<ArtistInfo>, String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        let sql = if force {
            "SELECT id, name FROM artists WHERE photo_path IS NULL ORDER BY name"
        } else {
            "SELECT id, name FROM artists WHERE photo_fetched_at IS NULL ORDER BY name"
        };
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;

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

    /// Check if an artist has already been attempted (photo_fetched_at is set)
    fn has_artist_photo(&self, artist_id: i64) -> bool {
        let conn = match self.pool.get() {
            Ok(conn) => conn,
            Err(_) => return false,
        };

        let result: Result<Option<String>, _> = conn.query_row(
            "SELECT photo_fetched_at FROM artists WHERE id = ?1",
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

    /// Write all MusicBrainz metadata + artwork path to the database.
    fn update_album_mb_info(
        &self,
        album_id: i64,
        artwork_path: &str,
        result: &ArtworkSearchResult,
    ) -> Result<(), String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE albums SET
                online_artwork_path = ?1,
                artwork_source      = 'musicbrainz',
                artwork_fetched_at  = CURRENT_TIMESTAMP,
                musicbrainz_id      = ?2,
                label               = ?3,
                country             = ?4,
                barcode             = ?5,
                album_type          = ?6,
                release_status      = ?7
             WHERE id = ?8",
            rusqlite::params![
                artwork_path,
                result.musicbrainz_id,
                result.label,
                result.country,
                result.barcode,
                result.album_type,
                result.release_status,
                album_id,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// One-shot cleanup: delete artwork files using the old `{album_db_id}.jpg` naming
    /// scheme (pure-digit filenames) that were created before MBID-based naming.
    /// Triggered by the `pending_legacy_artwork_cleanup` flag set by migration 3.
    fn cleanup_legacy_artwork_if_needed(&self) {
        let conn = match self.pool.get() {
            Ok(c) => c,
            Err(_) => return,
        };

        let needs_cleanup: bool = conn
            .query_row(
                "SELECT value FROM app_state WHERE key = 'pending_legacy_artwork_cleanup'",
                [],
                |row| row.get::<_, String>(0),
            )
            .map(|v| v == "true")
            .unwrap_or(false);

        if !needs_cleanup {
            return;
        }

        let artwork_dir = self.app_paths.album_artwork_dir();
        if let Ok(entries) = std::fs::read_dir(&artwork_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "jpg") {
                    let is_legacy = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|stem| stem.chars().all(|c| c.is_ascii_digit()));
                    if is_legacy {
                        if let Err(e) = std::fs::remove_file(&path) {
                            log::warn!("Could not remove legacy artwork file {path:?}: {e}");
                        } else {
                            log::info!("Removed legacy artwork file: {path:?}");
                        }
                    }
                }
            }
        }

        let _ = conn.execute(
            "DELETE FROM app_state WHERE key = 'pending_legacy_artwork_cleanup'",
            [],
        );
        log::info!("Legacy artwork cleanup complete");
    }

    /// Update album artwork in database (test helper only)
    #[cfg(test)]
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
    fn update_artist_photo(&self, artist_id: i64, path: &str) -> Result<(), String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE artists SET photo_path = ?1, photo_source = 'theaudiodb', photo_fetched_at = CURRENT_TIMESTAMP WHERE id = ?2",
            rusqlite::params![path, artist_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Update rich artist metadata from TheAudioDB, using COALESCE to avoid overwriting existing data.
    fn update_artist_metadata(
        &self,
        artist_id: i64,
        result: &crate::services::artwork::client::ArtistSearchResult,
    ) -> Result<(), String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE artists SET
                bio              = COALESCE(?1, bio),
                country          = COALESCE(?2, country),
                genre            = COALESCE(?3, genre),
                style            = COALESCE(?4, style),
                mood             = COALESCE(?5, mood),
                formed_year      = COALESCE(?6, formed_year),
                born_year        = COALESCE(?7, born_year),
                died_year        = COALESCE(?8, died_year),
                disbanded        = COALESCE(?9, disbanded),
                musicbrainz_id   = COALESCE(?10, musicbrainz_id),
                theaudiodb_id    = COALESCE(?11, theaudiodb_id),
                photo_fetched_at = CURRENT_TIMESTAMP
             WHERE id = ?12",
            rusqlite::params![
                result.bio,
                result.country,
                result.genre,
                result.style,
                result.mood,
                result.formed_year,
                result.born_year,
                result.died_year,
                result.disbanded,
                result.musicbrainz_id,
                result.theaudiodb_id,
                artist_id,
            ],
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
    use crate::db::queries;
    use crate::test_helpers::TestEnv;

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
        let albums = service.get_albums_for_scrape(false).unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].title, "The Wall");
    }

    #[test]
    fn test_get_albums_excludes_fetched() {
        let (env, _artist_id, album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());

        // Set artwork
        service
            .update_album_artwork(album_id, "/path/to/art.jpg".to_string())
            .unwrap();

        let albums = service.get_albums_for_scrape(false).unwrap();
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
        service
            .update_album_artwork(album_id, "/art.jpg".to_string())
            .unwrap();
        assert!(service.has_album_artwork(album_id));
    }

    #[test]
    fn test_update_album_artwork() {
        let (env, _artist_id, album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        service
            .update_album_artwork(album_id, "/path/art.jpg".to_string())
            .unwrap();

        let conn = env.pool.get().unwrap();
        let path: String = conn
            .query_row(
                "SELECT online_artwork_path FROM albums WHERE id = ?1",
                [album_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(path, "/path/art.jpg");

        let source: String = conn
            .query_row(
                "SELECT artwork_source FROM albums WHERE id = ?1",
                [album_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(source, "musicbrainz");
    }

    #[test]
    fn test_get_artists_without_photos() {
        let (env, _artist_id, _album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        let artists = service.get_artists_for_scrape(false).unwrap();
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
        service
            .update_artist_photo(artist_id, "/photo.jpg")
            .unwrap();
        assert!(service.has_artist_photo(artist_id));
    }

    #[test]
    fn test_update_artist_photo() {
        let (env, artist_id, _album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        service
            .update_artist_photo(artist_id, "/path/photo.jpg")
            .unwrap();

        let conn = env.pool.get().unwrap();
        let path: String = conn
            .query_row(
                "SELECT photo_path FROM artists WHERE id = ?1",
                [artist_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(path, "/path/photo.jpg");

        let source: String = conn
            .query_row(
                "SELECT photo_source FROM artists WHERE id = ?1",
                [artist_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(source, "theaudiodb");
    }

    // ── get_progress / cancel_fetch ──────────────────────────────────────────

    #[test]
    fn test_get_progress_returns_none_initially() {
        let (env, _, _) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        assert!(service.get_progress().is_none());
    }

    #[test]
    fn test_cancel_fetch_sets_cancelled_flag() {
        let (env, _, _) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        assert!(!service.is_cancelled());
        service.cancel_fetch();
        assert!(service.is_cancelled());
    }

    #[test]
    fn test_reset_cancel_clears_flag() {
        let (env, _, _) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        service.cancel_fetch();
        assert!(service.is_cancelled());
        service.reset_cancel();
        assert!(!service.is_cancelled());
    }

    // ── get_albums_without_artwork (multiple / sort) ─────────────────────────

    #[test]
    fn test_get_albums_without_artwork_multiple_sorted_by_artist() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let artist_z = queries::insert_artist(&conn, "Zappa, Frank", None).unwrap();
        let artist_a = queries::insert_artist(&conn, "ABBA", None).unwrap();
        queries::insert_album(&conn, "Apostrophe", artist_z, None).unwrap();
        queries::insert_album(&conn, "The Album", artist_a, None).unwrap();
        drop(conn);

        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        let albums = service.get_albums_for_scrape(false).unwrap();
        assert_eq!(albums.len(), 2);
        // Ordered by artist name: ABBA before Zappa
        assert_eq!(albums[0].artist_name, "ABBA");
        assert_eq!(albums[1].artist_name, "Zappa, Frank");
    }

    #[test]
    fn test_get_albums_without_artwork_empty_library() {
        let env = TestEnv::new();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        let albums = service.get_albums_for_scrape(false).unwrap();
        assert!(albums.is_empty());
    }

    // ── get_artists_without_photos (multiple / sort) ─────────────────────────

    #[test]
    fn test_get_artists_without_photos_multiple_sorted_by_name() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        queries::insert_artist(&conn, "Miles Davis", None).unwrap();
        queries::insert_artist(&conn, "Alice Coltrane", None).unwrap();
        queries::insert_artist(&conn, "John Coltrane", None).unwrap();
        drop(conn);

        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        let artists = service.get_artists_for_scrape(false).unwrap();
        assert_eq!(artists.len(), 3);
        assert_eq!(artists[0].name, "Alice Coltrane");
        assert_eq!(artists[1].name, "John Coltrane");
        assert_eq!(artists[2].name, "Miles Davis");
    }

    #[test]
    fn test_get_artists_without_photos_excludes_artists_with_photo() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();
        let with_photo = queries::insert_artist(&conn, "Known Artist", None).unwrap();
        queries::insert_artist(&conn, "Unknown Artist", None).unwrap();
        drop(conn);

        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        service
            .update_artist_photo(with_photo, "/photo.jpg")
            .unwrap();

        let artists = service.get_artists_for_scrape(false).unwrap();
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].name, "Unknown Artist");
    }

    // ── update_album_artwork sets artwork_fetched_at ─────────────────────────

    #[test]
    fn test_update_album_artwork_sets_fetched_at() {
        let (env, _artist_id, album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        service
            .update_album_artwork(album_id, "/art.jpg".to_string())
            .unwrap();

        let conn = env.pool.get().unwrap();
        let fetched_at: Option<String> = conn
            .query_row(
                "SELECT artwork_fetched_at FROM albums WHERE id = ?1",
                [album_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            fetched_at.is_some(),
            "artwork_fetched_at should be set after update"
        );
    }

    #[test]
    fn test_update_artist_photo_sets_fetched_at() {
        let (env, artist_id, _album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        service
            .update_artist_photo(artist_id, "/photo.jpg")
            .unwrap();

        let conn = env.pool.get().unwrap();
        let fetched_at: Option<String> = conn
            .query_row(
                "SELECT photo_fetched_at FROM artists WHERE id = ?1",
                [artist_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            fetched_at.is_some(),
            "photo_fetched_at should be set after update"
        );
    }

    // ── has_album_artwork / has_artist_photo for unknown ids ─────────────────

    #[test]
    fn test_has_album_artwork_returns_false_for_unknown_id() {
        let (env, _, _) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        assert!(!service.has_album_artwork(9999));
    }

    #[test]
    fn test_has_artist_photo_returns_false_for_unknown_id() {
        let (env, _, _) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());
        assert!(!service.has_artist_photo(9999));
    }

    #[test]
    fn test_mark_artist_fetch_attempted_excludes_from_future_runs() {
        let (env, artist_id, _album_id) = setup_artwork_env();
        let service = ArtworkService::new(env.pool.clone(), env.app_paths.clone());

        // Before: artist should appear in "without photos" list
        assert_eq!(service.get_artists_for_scrape(false).unwrap().len(), 1);

        // Mark as attempted (not found in TADB, no photo saved)
        service.mark_artist_fetch_attempted(artist_id).unwrap();

        // After: artist should no longer appear (photo_fetched_at is set)
        assert_eq!(service.get_artists_for_scrape(false).unwrap().len(), 0);
        // But photo_path should still be NULL
        let conn = env.pool.get().unwrap();
        let photo_path: Option<String> = conn
            .query_row(
                "SELECT photo_path FROM artists WHERE id = ?1",
                [artist_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(photo_path.is_none());
    }

    // =========================================================================
    // ArtworkFetchProgress::new — T006
    // =========================================================================

    #[test]
    fn test_artwork_fetch_progress_new_sets_total_items() {
        let p = ArtworkFetchProgress::new(42);
        assert_eq!(p.total_items, 42);
    }

    #[test]
    fn test_artwork_fetch_progress_new_zeroes_counters() {
        let p = ArtworkFetchProgress::new(10);
        assert_eq!(p.processed_items, 0);
        assert_eq!(p.successful, 0);
        assert_eq!(p.failed, 0);
    }

    #[test]
    fn test_artwork_fetch_progress_new_empty_current_item() {
        let p = ArtworkFetchProgress::new(5);
        assert!(p.current_item.is_empty());
    }

    #[test]
    fn test_artwork_fetch_progress_new_zero_total() {
        let p = ArtworkFetchProgress::new(0);
        assert_eq!(p.total_items, 0);
        assert_eq!(p.processed_items, 0);
        assert_eq!(p.successful, 0);
        assert_eq!(p.failed, 0);
        assert!(p.current_item.is_empty());
    }
}
