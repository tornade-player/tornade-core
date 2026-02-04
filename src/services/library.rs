// Library management service

use crate::db::{queries, DbPool};
use crate::models::{Track, Album, Artist, Source, AudioFormat};
use crate::services::error::LibraryError;
use crate::services::events::{ScanProgress, ScanResult, ScanError};
use crate::services::metadata::MetadataService;
use crate::utils::AppPaths;
use log::{info, warn, error};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use walkdir::WalkDir;

type Result<T> = std::result::Result<T, LibraryError>;

#[derive(Clone)]
pub struct LibraryService {
    pool: DbPool,
    metadata_service: MetadataService,
    scan_progress: Arc<Mutex<Option<ScanProgress>>>,
    scan_cancelled: Arc<Mutex<bool>>,
}

impl LibraryService {
    pub fn new(pool: DbPool, app_paths: AppPaths) -> Self {
        LibraryService {
            pool,
            metadata_service: MetadataService::new(app_paths),
            scan_progress: Arc::new(Mutex::new(None)),
            scan_cancelled: Arc::new(Mutex::new(false)),
        }
    }

    // ========================================================================
    // Scanning
    // ========================================================================

    /// Scan a directory and add tracks to the library
    pub fn scan_directory(&self, path: &Path, source_id: i64) -> Result<ScanResult> {
        info!("Starting library scan: {:?}", path);
        let start_time = Instant::now();

        // Validate directory exists and is accessible (T135)
        if !path.exists() {
            error!("Library folder does not exist: {:?}", path);
            return Err(LibraryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Library folder not found: {:?}", path)
            )));
        }

        if !path.is_dir() {
            error!("Library path is not a directory: {:?}", path);
            return Err(LibraryError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Path is not a directory: {:?}", path)
            )));
        }

        // Reset cancellation flag
        *self.scan_cancelled.lock().unwrap() = false;

        let mut tracks_added = 0u32;
        let mut tracks_updated = 0u32;
        let mut tracks_skipped = 0u32;
        let mut errors = Vec::new();

        // Collect all audio files
        let audio_files: Vec<PathBuf> = WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                if let Some(ext) = e.path().extension() {
                    matches!(
                        ext.to_str().unwrap_or("").to_lowercase().as_str(),
                        "flac" | "mp3" | "aac" | "m4a" | "alac"
                    )
                } else {
                    false
                }
            })
            .map(|e| e.path().to_path_buf())
            .collect();

        let total_files = audio_files.len() as u32;
        info!("Found {} audio files to process", total_files);

        // Update initial progress
        {
            let mut progress = self.scan_progress.lock().unwrap();
            *progress = Some(ScanProgress {
                total_files,
                processed_files: 0,
                current_file: None,
                tracks_added: 0,
            });
        }

        // T130: Process files in batches with transactions for better performance
        const BATCH_SIZE: usize = 50;

        for (batch_idx, batch) in audio_files.chunks(BATCH_SIZE).enumerate() {
            // Check for cancellation
            if *self.scan_cancelled.lock().unwrap() {
                warn!("Library scan cancelled by user");
                return Err(LibraryError::ScanCancelled);
            }

            // Get connection for this batch
            let conn = self.pool.get().map_err(|e| {
                LibraryError::Database(rusqlite::Error::InvalidPath(
                    PathBuf::from(format!("Pool error: {}", e))
                ))
            })?;

            // Process batch within a transaction
            let tx = conn.unchecked_transaction().map_err(LibraryError::Database)?;

            for (file_idx, file_path) in batch.iter().enumerate() {
                let overall_idx = batch_idx * BATCH_SIZE + file_idx;

                // Update progress
                {
                    let mut progress = self.scan_progress.lock().unwrap();
                    if let Some(ref mut p) = *progress {
                        p.processed_files = overall_idx as u32;
                        p.current_file = Some(file_path.clone());
                        p.tracks_added = tracks_added;
                    }
                }

                // T134: Process the file with proper error handling for corrupted files
                match self.process_audio_file_with_conn(&tx, file_path, source_id) {
                    Ok(true) => tracks_added += 1,
                    Ok(false) => tracks_updated += 1,
                    Err(e) => {
                        // Log corrupted/invalid files and continue processing
                        warn!("Skipping file {:?}: {}", file_path, e);
                        error!("Corrupted or invalid file: {:?} - {}", file_path, e);
                        errors.push(ScanError {
                            path: file_path.clone(),
                            error: format!("Corrupted/invalid file: {}", e),
                        });
                        tracks_skipped += 1;
                    }
                }
            }

            // Commit the batch transaction
            tx.commit().map_err(LibraryError::Database)?;
        }

        // Clear progress
        {
            let mut progress = self.scan_progress.lock().unwrap();
            *progress = None;
        }

        let duration = start_time.elapsed();
        info!(
            "Library scan complete: {} added, {} updated, {} skipped in {:?}",
            tracks_added, tracks_updated, tracks_skipped, duration
        );

        if !errors.is_empty() {
            info!("Encountered {} errors during scan (see log for details)", errors.len());
        }

        Ok(ScanResult {
            tracks_added,
            tracks_updated,
            tracks_skipped,
            errors,
            duration,
        })
    }

    /// Process a single audio file (legacy method, uses pool connection)
    #[allow(dead_code)]
    fn _process_audio_file(&self, path: &Path, source_id: i64) -> Result<bool> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        self.process_audio_file_with_conn(&conn, path, source_id)
    }

    /// Process a single audio file with provided connection/transaction (T130, T134)
    fn process_audio_file_with_conn<C: std::ops::Deref<Target = rusqlite::Connection>>(
        &self,
        conn: &C,
        path: &Path,
        source_id: i64
    ) -> Result<bool> {
        // T134: Validate file exists and is readable
        if !path.exists() {
            return Err(LibraryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "File not found"
            )));
        }

        // T134: Read metadata with proper error handling for corrupted files
        let metadata = match self.metadata_service.read_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                // Log and skip corrupted files instead of stopping the scan
                warn!("Failed to read metadata from {:?}: {}", path, e);
                return Err(LibraryError::Metadata(format!(
                    "Corrupted or invalid audio file: {}",
                    e
                )));
            }
        };

        // Get or create artist
        let artist_id = queries::insert_artist(
            conn,
            &metadata.artist,
            None, // name_sort can be computed later
        )?;

        // Get or create album if present
        let album_id = if let Some(ref album_title) = metadata.album {
            Some(queries::insert_album(
                conn,
                album_title,
                artist_id,
                metadata.year,
            )?)
        } else {
            None
        };

        // Determine audio format
        let file_format = MetadataService::get_file_format(path)
            .and_then(|ext| AudioFormat::from_str(&ext))
            .ok_or_else(|| LibraryError::Metadata("Unknown file format".to_string()))?;

        // Get file size
        let file_size = std::fs::metadata(path)
            .map_err(LibraryError::Io)?
            .len();

        // Insert/update track
        let track_id = queries::insert_track(
            conn,
            &metadata.title,
            album_id,
            artist_id,
            source_id,
            &path.to_path_buf(),
            metadata.duration.as_millis() as i64,
            metadata.track_number,
            metadata.sample_rate,
            metadata.bit_depth,
            file_format,
            file_size,
        )?;

        // Add genre if present
        if let Some(ref genre_name) = metadata.genre {
            let genre_id = queries::insert_genre(conn, genre_name)?;
            queries::link_track_genre(conn, track_id, genre_id)?;
        }

        Ok(true) // Track was added/updated
    }

    pub fn scan_progress(&self) -> Option<ScanProgress> {
        self.scan_progress.lock().unwrap().clone()
    }

    pub fn cancel_scan(&self) {
        *self.scan_cancelled.lock().unwrap() = true;
    }

    // ========================================================================
    // Sources
    // ========================================================================

    pub fn list_sources(&self) -> Result<Vec<Source>> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::list_sources(&conn).map_err(LibraryError::Database)
    }

    pub fn add_source(&self, name: &str, path: &Path) -> Result<Source> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        // Check if a source with this path already exists
        if let Some(existing_source) = self.find_source_by_path(path)? {
            info!("Source with path {:?} already exists (id: {}), returning existing source", path, existing_source.id);
            return Ok(existing_source);
        }

        use crate::models::source::SourceType;
        let source_id = queries::insert_source(
            &conn,
            name,
            SourceType::Disk,
            Some(&path.to_path_buf()),
        )?;

        queries::get_source(&conn, source_id)?
            .ok_or(LibraryError::SourceNotFound(source_id))
    }

    /// Find a source by its path (to avoid duplicate sources)
    pub fn find_source_by_path(&self, path: &Path) -> Result<Option<Source>> {
        let sources = self.list_sources()?;
        for source in sources {
            if let Some(ref source_path) = source.path {
                // Compare canonical paths to handle symlinks and relative paths
                let source_canonical = source_path.canonicalize().unwrap_or_else(|_| source_path.clone());
                let path_canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

                if source_canonical == path_canonical {
                    return Ok(Some(source));
                }
            }
        }
        Ok(None)
    }

    /// Validate all sources and detect moved/deleted folders (T135)
    pub fn validate_sources(&self) -> Result<Vec<(Source, bool)>> {
        let sources = self.list_sources()?;
        let mut results = Vec::new();

        for source in sources {
            let is_valid = if let Some(ref path) = source.path {
                path.exists() && path.is_dir()
            } else {
                false // No path means invalid
            };

            if !is_valid {
                warn!(
                    "Source '{}' (ID: {}) path is invalid or missing: {:?}",
                    source.name, source.id, source.path
                );
            }

            results.push((source, is_valid));
        }

        Ok(results)
    }

    // ========================================================================
    // Tracks
    // ========================================================================

    pub fn get_track(&self, id: i64) -> Result<Option<Track>> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::get_track(&conn, id).map_err(LibraryError::Database)
    }

    pub fn get_album_tracks(&self, album_id: i64) -> Result<Vec<Track>> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::get_album_tracks(&conn, album_id).map_err(LibraryError::Database)
    }

    pub fn rate_track(&self, track_id: i64, rating: u8) -> Result<()> {
        if rating > 5 {
            return Err(LibraryError::InvalidRating(rating));
        }

        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::update_track_rating(&conn, track_id, rating)
            .map_err(LibraryError::Database)
    }

    // ========================================================================
    // Albums
    // ========================================================================

    pub fn get_album(&self, id: i64) -> Result<Option<Album>> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::get_album(&conn, id).map_err(LibraryError::Database)
    }

    pub fn list_albums(
        &self,
        artist_id: Option<i64>,
        genre_id: Option<i64>,
        min_rating: Option<u8>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<Album>> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::list_albums(&conn, artist_id, genre_id, min_rating, limit, offset)
            .map_err(LibraryError::Database)
    }

    pub fn get_artist_albums(&self, artist_id: i64) -> Result<Vec<Album>> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::get_artist_albums(&conn, artist_id)
            .map_err(LibraryError::Database)
    }

    pub fn rate_album(&self, album_id: i64, rating: u8) -> Result<()> {
        if rating > 5 {
            return Err(LibraryError::InvalidRating(rating));
        }

        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::update_album_rating(&conn, album_id, rating)
            .map_err(LibraryError::Database)
    }

    // ========================================================================
    // Artists
    // ========================================================================

    pub fn get_artist(&self, id: i64) -> Result<Option<Artist>> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::get_artist(&conn, id).map_err(LibraryError::Database)
    }

    pub fn list_artists(&self) -> Result<Vec<Artist>> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::list_artists(&conn).map_err(LibraryError::Database)
    }

    pub fn get_genre_artists(&self, genre_id: i64) -> Result<Vec<Artist>> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::get_genre_artists(&conn, genre_id)
            .map_err(LibraryError::Database)
    }

    // ========================================================================
    // Genres
    // ========================================================================

    pub fn list_genres(&self) -> Result<Vec<(crate::models::Genre, u32, u32)>> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::list_genres_with_count(&conn)
            .map_err(LibraryError::Database)
    }

    pub fn get_genre_tracks(&self, genre_id: i64) -> Result<Vec<Track>> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::get_genre_tracks(&conn, genre_id)
            .map_err(LibraryError::Database)
    }

    pub fn get_source_tracks(&self, source_id: i64) -> Result<Vec<Track>> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::get_source_tracks(&conn, source_id)
            .map_err(LibraryError::Database)
    }

    // ========================================================================
    // Search
    // ========================================================================

    pub fn search(&self, query: &str, limit: usize) -> Result<(Vec<Track>, Vec<Album>, Vec<Artist>)> {
        let conn = self.pool.get().map_err(|e| {
            LibraryError::Database(rusqlite::Error::InvalidPath(
                PathBuf::from(format!("Pool error: {}", e))
            ))
        })?;

        queries::search_library(&conn, query, limit)
            .map_err(LibraryError::Database)
    }
}
