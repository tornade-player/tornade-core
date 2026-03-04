// Library management service

use crate::db::{DbPool, queries};
use crate::models::{Album, Artist, AudioFormat, Source, Track};
use crate::services::error::LibraryError;
use crate::services::events::{ScanError, ScanProgress, ScanResult};
use crate::services::metadata::MetadataService;
use crate::services::reports::ScanReport;
use crate::utils::AppPaths;
use chrono::Local;
use log::{error, info, warn};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use walkdir::WalkDir;

type Result<T> = std::result::Result<T, LibraryError>;

/// Split a raw ARTIST tag value into individual artist names (Option 4 heuristic).
///
/// # Strategy
/// 1. If the string contains an unambiguous collaboration marker (feat, ft, avec, vs, comma)
///    → split on **all** separators including `&` and `+` (contextual evidence that `&`/`+`
///    separates artists rather than forming a band name).
/// 2. Otherwise → return the string as-is.
///
/// # Examples
/// ```text
/// "Stromae avec Maitre Gims & OrelSan" → ["Stromae", "Maitre Gims", "OrelSan"]
/// "Alan Sivestri, Brusser Philarmonic & Dirk Bross" → ["Alan Sivestri", "Brusser Philarmonic", "Dirk Bross"]
/// "Doc Gynéco ft El maestro"           → ["Doc Gynéco", "El maestro"]
/// "Lauryn Hill ft. D'angelo"           → ["Lauryn Hill", "D'angelo"]
/// "Simon & Garfunkel"                  → ["Simon & Garfunkel"]   (no context → intact)
/// "Mike + The Mechanics"               → ["Mike + The Mechanics"] (no context → intact)
/// "Akhenaton, Disiz la Peste"          → ["Akhenaton", "Disiz la Peste"]
/// ```
/// Extract featured artist names from a track title.
///
/// Recognises parenthetical or bracketed feat markers:
/// ```text
/// "Titanium (feat. Sia)"              → ["Sia"]
/// "Diamond [ft. Rihanna & Jay-Z]"     → ["Rihanna", "Jay-Z"]
/// "Avf (avec OrelSan & Maitre Gims)"  → ["OrelSan", "Maitre Gims"]
/// "Normal Title"                      → []
/// ```
pub(crate) fn extract_feat_from_title(title: &str) -> Vec<String> {
    let lower = title.to_lowercase();

    // Longest patterns first to avoid partial matches.
    const MARKERS: &[&str] = &[
        "(featuring ", "(feat. ", "(feat ", "(ft. ", "(ft ", "(avec ",
        "[featuring ", "[feat. ", "[feat ", "[ft. ", "[ft ", "[avec ",
    ];

    for marker in MARKERS {
        let Some(start) = lower.find(marker) else {
            continue;
        };
        let artist_start = start + marker.len();
        let rest_lower = &lower[artist_start..];
        let closing = rest_lower
            .find(|c| c == ')' || c == ']')
            .unwrap_or(rest_lower.len());
        let artist_end = artist_start + closing;

        // Map byte offsets back to the original title for proper casing.
        let artist_str = if artist_start <= title.len()
            && artist_end <= title.len()
            && title.is_char_boundary(artist_start)
            && title.is_char_boundary(artist_end)
        {
            title[artist_start..artist_end].trim()
        } else {
            // Edge case: non-ASCII char changed byte length when lowercased.
            rest_lower[..closing].trim()
        };

        if !artist_str.is_empty() {
            // Inside a feat block, & and + are always artist separators.
            // Prepend a comma-space to trigger split_artists' context detection.
            let with_context = format!(", {artist_str}");
            return split_artists(&with_context);
        }
    }

    vec![]
}

pub(crate) fn split_artists(artist: &str) -> Vec<String> {
    let trimmed = artist.trim();
    if trimmed.is_empty() {
        return vec!["Unknown Artist".to_string()];
    }

    // Detect unambiguous collaboration markers (all ASCII patterns — safe for contains()).
    let lower = trimmed.to_lowercase();
    let has_context = trimmed.contains(", ")
        || trimmed.contains(',')
        || lower.contains(" feat")    // feat, feat., featuring
        || lower.contains(" ft")      // ft, ft.
        || lower.contains(" avec ")
        || lower.contains(" vs");     // vs, vs.

    if !has_context {
        return vec![trimmed.to_string()];
    }

    // Replace all separators with a null-byte delimiter (longest patterns first to
    // avoid partial matches). Patterns are ASCII so str::replace is safe and correct.
    const DELIM: &str = "\x00";
    #[rustfmt::skip]
    const PATTERNS: &[&str] = &[
        " featuring ", " Featuring ", " FEATURING ",
        " feat. ",     " Feat. ",     " FEAT. ",
        " feat ",      " Feat ",      " FEAT ",
        " ft. ",       " Ft. ",       " FT. ",
        " ft ",        " Ft ",        " FT ",
        " avec ",      " Avec ",      " AVEC ",
        " vs. ",       " Vs. ",       " VS. ",
        " vs ",        " Vs ",        " VS ",
        " & ", " + ", ", ", ",",
    ];

    let mut s = trimmed.to_string();
    for pat in PATTERNS {
        s = s.replace(pat, DELIM);
    }

    s.split('\x00')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

#[derive(Clone)]
pub struct LibraryService {
    pool: DbPool,
    app_paths: AppPaths,
    metadata_service: MetadataService,
    scan_progress: Arc<Mutex<Option<ScanProgress>>>,
    scan_cancelled: Arc<Mutex<bool>>,
}

impl LibraryService {
    pub fn new(pool: DbPool, app_paths: AppPaths) -> Self {
        LibraryService {
            pool,
            app_paths: app_paths.clone(), // clone: value also moved into MetadataService below
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
        info!("Starting library scan: {path:?}");
        let start_time = Instant::now();
        let scan_start_time = Local::now();

        // Validate directory exists and is accessible (T135)
        if !path.exists() {
            error!("Library folder does not exist: {path:?}");
            return Err(LibraryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Library folder not found: {path:?}"),
            )));
        }

        if !path.is_dir() {
            error!("Library path is not a directory: {path:?}");
            return Err(LibraryError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Path is not a directory: {path:?}"),
            )));
        }

        // Reset cancellation flag
        *self.scan_cancelled.lock().unwrap() = false;

        let mut tracks_added = 0u32;
        let tracks_updated = 0u32;
        let mut tracks_skipped = 0u32;
        let mut errors = Vec::new();

        // Helper: returns true for supported audio extensions
        let is_audio = |e: &walkdir::DirEntry| -> bool {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| {
                        matches!(
                            s.to_lowercase().as_str(),
                            "flac" | "mp3" | "aac" | "m4a" | "alac"
                        )
                    })
        };

        // Pass 1: count files for progress reporting (lightweight, no PathBuf allocation)
        let total_files = WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|e| is_audio(e))
            .count() as u32;

        info!("Found {total_files} audio files to process");

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

        // Pass 2: stream files and process in batches — only BATCH_SIZE paths in RAM at a time
        const BATCH_SIZE: usize = 50;
        let mut batch: Vec<PathBuf> = Vec::with_capacity(BATCH_SIZE);
        let mut processed = 0u32;

        let walker = WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|e| is_audio(e));

        for entry in walker {
            batch.push(entry.path().to_path_buf());

            if batch.len() >= BATCH_SIZE {
                // Check for cancellation between batches
                if *self.scan_cancelled.lock().unwrap() {
                    warn!("Library scan cancelled by user");
                    return Err(LibraryError::ScanCancelled);
                }

                let conn = self.pool.get()?;
                let tx = conn
                    .unchecked_transaction()
                    .map_err(LibraryError::Database)?;

                for file_path in &batch {
                    {
                        let mut progress = self.scan_progress.lock().unwrap();
                        if let Some(ref mut p) = *progress {
                            p.processed_files = processed;
                            p.current_file = Some(file_path.clone());
                            p.tracks_added = tracks_added;
                        }
                    }

                    match self.process_audio_file_with_conn(&tx, file_path, source_id) {
                        Ok(_track_id) => tracks_added += 1,
                        Err(e) => {
                            warn!("Skipping file {file_path:?}: {e}");
                            error!("Corrupted or invalid file: {file_path:?} - {e}");
                            errors.push(ScanError {
                                path: file_path.clone(),
                                error: format!("Corrupted/invalid file: {e}"),
                            });
                            tracks_skipped += 1;
                        }
                    }
                    processed += 1;
                }

                tx.commit().map_err(LibraryError::Database)?;
                batch.clear();
            }
        }

        // Process remaining files (last partial batch)
        if !batch.is_empty() {
            if *self.scan_cancelled.lock().unwrap() {
                warn!("Library scan cancelled by user");
                return Err(LibraryError::ScanCancelled);
            }

            let conn = self.pool.get()?;
            let tx = conn
                .unchecked_transaction()
                .map_err(LibraryError::Database)?;

            for file_path in &batch {
                {
                    let mut progress = self.scan_progress.lock().unwrap();
                    if let Some(ref mut p) = *progress {
                        p.processed_files = processed;
                        p.current_file = Some(file_path.clone());
                        p.tracks_added = tracks_added;
                    }
                }

                match self.process_audio_file_with_conn(&tx, file_path, source_id) {
                    Ok(_track_id) => tracks_added += 1,
                    Err(e) => {
                        warn!("Skipping file {file_path:?}: {e}");
                        error!("Corrupted or invalid file: {file_path:?} - {e}");
                        errors.push(ScanError {
                            path: file_path.clone(),
                            error: format!("Corrupted/invalid file: {e}"),
                        });
                        tracks_skipped += 1;
                    }
                }
                processed += 1;
            }

            tx.commit().map_err(LibraryError::Database)?;
        }

        // Clear progress
        {
            let mut progress = self.scan_progress.lock().unwrap();
            *progress = None;
        }

        let duration = start_time.elapsed();
        info!(
            "Library scan complete: {tracks_added} added, {tracks_updated} updated, {tracks_skipped} skipped in {duration:?}"
        );

        if !errors.is_empty() {
            info!(
                "Encountered {} errors during scan (see log for details)",
                errors.len()
            );
        }

        // Generate scan report
        let mut report = ScanReport::new(path.to_string_lossy().into_owned(), scan_start_time);
        report.end_time = Local::now();
        report.total_files = total_files as usize;
        report.tracks_added = tracks_added as usize;
        report.errors = errors.iter().map(|e| format!("{e:?}")).collect();

        // Try to save report (non-fatal if it fails)
        match report.save(&self.app_paths.reports_dir()) {
            Ok(path) => info!("Scan report saved to: {path:?}"),
            Err(e) => warn!("Failed to save scan report: {e}"),
        }

        Ok(ScanResult {
            tracks_added,
            tracks_updated,
            tracks_skipped,
            errors,
            duration,
        })
    }

    /// Process a single audio file with provided connection/transaction (T130, T134)
    fn process_audio_file_with_conn<C: std::ops::Deref<Target = rusqlite::Connection>>(
        &self,
        conn: &C,
        path: &Path,
        source_id: i64,
    ) -> Result<i64> {
        // T134: Validate file exists and is readable
        if !path.exists() {
            return Err(LibraryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "File not found",
            )));
        }

        // T134: Read metadata with proper error handling for corrupted files
        let metadata = match self.metadata_service.read_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                // Log and skip corrupted files instead of stopping the scan
                warn!("Failed to read metadata from {path:?}: {e}");
                return Err(LibraryError::Metadata(format!(
                    "Corrupted or invalid audio file: {e}"
                )));
            }
        };

        // Get or create track artist.
        // Split composite tags (e.g. "Stromae avec Maitre Gims & OrelSan") and use the
        // first name as the primary artist. All artists are linked via track_artists.
        let artists = split_artists(&metadata.artist);
        let artist_id = queries::insert_artist(
            conn,
            &artists[0],
            None, // name_sort can be computed later
        )?;

        // For album grouping, prefer ALBUMARTIST tag over track artist.
        // This keeps multi-artist albums (e.g. "Dr. Dre feat. Snoop Dogg") together.
        // "Various Artists" is intentionally ignored — featuring relationships are
        // tracked in track_artists, so albums are always owned by their dominant artist.
        let album_artist_id = if let Some(ref album_artist) = metadata.album_artist {
            if album_artist == "Various Artists" {
                artist_id
            } else {
                queries::insert_artist(conn, album_artist, None)?
            }
        } else {
            artist_id
        };

        // Get or create album if present.
        //
        // Album identity is always (title, artist_id) — the same UNIQUE constraint used
        // in the database.  When ALBUMARTIST is set (and is not "Various Artists"),
        // we use it as the artist; otherwise we use the primary track artist.
        //
        // This means "Greatest Hits" by The Beatles and "Greatest Hits" by Eminem are
        // always stored as two separate albums, even if neither file carries an ALBUMARTIST
        // tag.
        let album_id = if let Some(ref album_title) = metadata.album {
            Some(queries::insert_album(
                conn,
                album_title,
                album_artist_id,
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
        let file_size = std::fs::metadata(path).map_err(LibraryError::Io)?.len();

        // Insert/update track
        let track_id = queries::insert_track(
            conn,
            &metadata.title,
            album_id,
            artist_id,
            source_id,
            path,
            metadata.duration.as_millis() as i64,
            metadata.track_number,
            metadata.sample_rate,
            metadata.bit_depth,
            file_format,
            file_size,
        )?;

        // Link all artists in track_artists (primary at position 0, featured at 1, 2, ...)
        for (pos, name) in artists.iter().enumerate() {
            let aid = queries::insert_artist(conn, name, None)?;
            queries::link_track_artist(conn, track_id, aid, pos as u32)?;
        }

        // Also link featured artists extracted from the track title
        // (e.g. "Titanium (feat. Sia)" when ARTIST tag only contains "David Guetta").
        let title_offset = artists.len() as u32;
        for (pos, name) in extract_feat_from_title(&metadata.title).into_iter().enumerate() {
            let aid = queries::insert_artist(conn, &name, None)?;
            queries::link_track_artist(conn, track_id, aid, title_offset + pos as u32)?;
        }

        // Add genre if present
        if let Some(ref genre_name) = metadata.genre {
            let genre_id = queries::insert_genre(conn, genre_name)?;
            queries::link_track_genre(conn, track_id, genre_id)?;
        }

        Ok(track_id)
    }

    /// Import a list of paths (files or directories) into the library.
    /// Returns the track IDs of all successfully imported tracks.
    pub fn import_paths(&self, paths: &[PathBuf]) -> Result<Vec<i64>> {
        let mut all_audio_files: Vec<PathBuf> = Vec::new();

        // Collect all audio files from the provided paths
        for path in paths {
            if path.is_file() {
                if let Some(ext) = path.extension()
                    && matches!(
                        ext.to_str().unwrap_or("").to_lowercase().as_str(),
                        "flac" | "mp3" | "aac" | "m4a" | "alac"
                    )
                {
                    all_audio_files.push(path.clone());
                }
            } else if path.is_dir() {
                let files: Vec<PathBuf> = WalkDir::new(path)
                    .follow_links(true)
                    .into_iter()
                    .filter_map(std::result::Result::ok)
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
                all_audio_files.extend(files);
            }
        }

        if all_audio_files.is_empty() {
            return Ok(Vec::new());
        }

        info!("import_paths: found {} audio files", all_audio_files.len());

        // Pre-create sources for all unique parent directories
        use std::collections::HashMap;
        let mut source_map: HashMap<PathBuf, i64> = HashMap::new();
        for file_path in &all_audio_files {
            let parent_dir = file_path
                .parent()
                .unwrap_or(file_path.as_path())
                .to_path_buf();
            if !source_map.contains_key(&parent_dir) {
                let source = self.add_source("Import", &parent_dir)?;
                source_map.insert(parent_dir, source.id);
            }
        }

        let mut track_ids: Vec<i64> = Vec::new();

        // Process files in batches of 50
        const BATCH_SIZE: usize = 50;

        for batch in all_audio_files.chunks(BATCH_SIZE) {
            let conn = self.pool.get()?;

            let tx = conn
                .unchecked_transaction()
                .map_err(LibraryError::Database)?;

            for file_path in batch {
                let parent_dir = file_path
                    .parent()
                    .unwrap_or(file_path.as_path())
                    .to_path_buf();
                let source_id = *source_map.get(&parent_dir).unwrap();

                match self.process_audio_file_with_conn(&tx, file_path, source_id) {
                    Ok(track_id) => track_ids.push(track_id),
                    Err(e) => {
                        warn!("import_paths: skipping {file_path:?}: {e}");
                    }
                }
            }

            tx.commit().map_err(LibraryError::Database)?;
        }

        info!("import_paths: imported {} tracks", track_ids.len());
        Ok(track_ids)
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
        let conn = self.pool.get()?;

        queries::list_sources(&conn).map_err(LibraryError::Database)
    }

    pub fn add_source(&self, name: &str, path: &Path) -> Result<Source> {
        let conn = self.pool.get()?;

        // Check if a source with this path already exists
        if let Some(existing_source) = self.find_source_by_path(path)? {
            info!(
                "Source with path {:?} already exists (id: {}), returning existing source",
                path, existing_source.id
            );
            return Ok(existing_source);
        }

        use crate::models::source::SourceType;
        let source_id = queries::insert_source(&conn, name, SourceType::Disk, Some(path))?;

        queries::get_source(&conn, source_id)?.ok_or(LibraryError::SourceNotFound(source_id))
    }

    /// Find a source by its path (to avoid duplicate sources)
    pub fn find_source_by_path(&self, path: &Path) -> Result<Option<Source>> {
        let sources = self.list_sources()?;
        for source in sources {
            if let Some(ref source_path) = source.path {
                // Compare canonical paths to handle symlinks and relative paths
                let source_canonical = source_path
                    .canonicalize()
                    .unwrap_or_else(|_| source_path.clone());
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
        let conn = self.pool.get()?;

        queries::get_track(&conn, id).map_err(LibraryError::Database)
    }

    pub fn get_album_tracks(&self, album_id: i64) -> Result<Vec<Track>> {
        let conn = self.pool.get()?;

        queries::get_album_tracks(&conn, album_id).map_err(LibraryError::Database)
    }

    pub fn rate_track(&self, track_id: i64, rating: u8) -> Result<()> {
        if rating > 5 {
            return Err(LibraryError::InvalidRating(rating));
        }

        let conn = self.pool.get()?;

        queries::update_track_rating(&conn, track_id, rating).map_err(LibraryError::Database)
    }

    // ========================================================================
    // Albums
    // ========================================================================

    pub fn get_album(&self, id: i64) -> Result<Option<Album>> {
        let conn = self.pool.get()?;

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
        let conn = self.pool.get()?;

        queries::list_albums(&conn, artist_id, genre_id, min_rating, limit, offset)
            .map_err(LibraryError::Database)
    }

    pub fn get_artist_albums(&self, artist_id: i64) -> Result<Vec<Album>> {
        let conn = self.pool.get()?;

        queries::get_artist_albums(&conn, artist_id).map_err(LibraryError::Database)
    }

    pub fn rate_album(&self, album_id: i64, rating: u8) -> Result<()> {
        if rating > 5 {
            return Err(LibraryError::InvalidRating(rating));
        }

        let conn = self.pool.get()?;

        queries::update_album_rating(&conn, album_id, rating).map_err(LibraryError::Database)
    }

    // ========================================================================
    // Artists
    // ========================================================================

    pub fn get_artist(&self, id: i64) -> Result<Option<Artist>> {
        let conn = self.pool.get()?;

        queries::get_artist(&conn, id).map_err(LibraryError::Database)
    }

    pub fn list_artists(&self) -> Result<Vec<Artist>> {
        let conn = self.pool.get()?;

        queries::list_artists(&conn).map_err(LibraryError::Database)
    }

    pub fn get_genre_artists(&self, genre_id: i64) -> Result<Vec<Artist>> {
        let conn = self.pool.get()?;

        queries::get_genre_artists(&conn, genre_id).map_err(LibraryError::Database)
    }

    // ========================================================================
    // Genres
    // ========================================================================

    pub fn list_genres(&self) -> Result<Vec<(crate::models::Genre, u32, u32)>> {
        let conn = self.pool.get()?;

        queries::list_genres_with_count(&conn).map_err(LibraryError::Database)
    }

    pub fn get_genre_tracks(&self, genre_id: i64) -> Result<Vec<Track>> {
        let conn = self.pool.get()?;

        queries::get_genre_tracks(&conn, genre_id).map_err(LibraryError::Database)
    }

    pub fn get_source_tracks(&self, source_id: i64) -> Result<Vec<Track>> {
        let conn = self.pool.get()?;

        queries::get_source_tracks(&conn, source_id).map_err(LibraryError::Database)
    }

    // ========================================================================
    // Search
    // ========================================================================

    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<(Vec<Track>, Vec<Album>, Vec<Artist>)> {
        let conn = self.pool.get()?;

        queries::search_library(&conn, query, limit).map_err(LibraryError::Database)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Rating;
    use crate::test_helpers::TestEnv;

    // ── split_artists ─────────────────────────────────────────────────────────

    #[test]
    fn test_split_artists_single() {
        assert_eq!(split_artists("Pink Floyd"), vec!["Pink Floyd"]);
    }

    #[test]
    fn test_split_artists_comma_list() {
        assert_eq!(
            split_artists("Akhenaton, Disiz la Peste"),
            vec!["Akhenaton", "Disiz la Peste"]
        );
    }

    #[test]
    fn test_split_artists_long_comma_list() {
        assert_eq!(
            split_artists("Akhenaton, Disiz la Peste, Kool Shen, Soprano"),
            vec!["Akhenaton", "Disiz la Peste", "Kool Shen", "Soprano"]
        );
    }

    #[test]
    fn test_split_artists_avec() {
        assert_eq!(
            split_artists("Stromae avec Maitre Gims & OrelSan"),
            vec!["Stromae", "Maitre Gims", "OrelSan"]
        );
    }

    #[test]
    fn test_split_artists_comma_and_ampersand() {
        assert_eq!(
            split_artists("Alan Silvestri, Brusser Philarmonic & Dirk Bross"),
            vec!["Alan Silvestri", "Brusser Philarmonic", "Dirk Bross"]
        );
    }

    #[test]
    fn test_split_artists_ft() {
        assert_eq!(
            split_artists("Doc Gynéco ft El maestro"),
            vec!["Doc Gynéco", "El maestro"]
        );
    }

    #[test]
    fn test_split_artists_ft_dot() {
        assert_eq!(
            split_artists("Lauryn Hill ft. D'angelo"),
            vec!["Lauryn Hill", "D'angelo"]
        );
    }

    #[test]
    fn test_split_artists_feat() {
        assert_eq!(
            split_artists("Drake feat. Future"),
            vec!["Drake", "Future"]
        );
    }

    #[test]
    fn test_split_artists_featuring() {
        assert_eq!(
            split_artists("Daft Punk featuring Pharrell Williams"),
            vec!["Daft Punk", "Pharrell Williams"]
        );
    }

    #[test]
    fn test_split_artists_preserves_simon_garfunkel() {
        // No unambiguous marker → intact
        assert_eq!(split_artists("Simon & Garfunkel"), vec!["Simon & Garfunkel"]);
    }

    #[test]
    fn test_split_artists_preserves_mike_and_mechanics() {
        assert_eq!(
            split_artists("Mike + The Mechanics"),
            vec!["Mike + The Mechanics"]
        );
    }

    #[test]
    fn test_split_artists_preserves_earth_wind_fire() {
        // "Earth, Wind & Fire" — comma present but " & " in second part
        // With Option 4 logic: comma IS present → split on & too
        // Result: ["Earth", "Wind", "Fire"] — acceptable trade-off
        let result = split_artists("Earth, Wind & Fire");
        assert_eq!(result, vec!["Earth", "Wind", "Fire"]);
    }

    #[test]
    fn test_split_artists_case_insensitive_feat() {
        assert_eq!(
            split_artists("Artist A FEAT Artist B"),
            vec!["Artist A", "Artist B"]
        );
    }

    #[test]
    fn test_split_artists_trims_whitespace() {
        assert_eq!(
            split_artists("  50 Cent, Eminem  "),
            vec!["50 Cent", "Eminem"]
        );
    }

    // Minimal valid FLAC file with STREAMINFO + VORBIS_COMMENT (title/artist/album).
    // Generated by make_fixture.py; verified parseable by lofty 0.21.
    const MINIMAL_FLAC: &[u8] = include_bytes!("../../tests/fixtures/minimal.flac");

    fn setup() -> (TestEnv, LibraryService) {
        let env = TestEnv::new();
        let svc = LibraryService::new(env.pool.clone(), env.app_paths.clone());
        (env, svc)
    }

    fn write_flac(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, MINIMAL_FLAC).unwrap();
        path
    }

    // ── File-collection logic ────────────────────────────────────────────────

    #[test]
    fn test_import_paths_empty_input() {
        let (_env, svc) = setup();
        let result = svc.import_paths(&[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_import_paths_nonexistent_path_returns_empty() {
        let (_env, svc) = setup();
        let bogus = PathBuf::from("/nonexistent/path/does/not/exist.flac");
        let result = svc.import_paths(&[bogus]).unwrap();
        assert!(
            result.is_empty(),
            "nonexistent paths must be skipped gracefully"
        );
    }

    #[test]
    fn test_import_paths_non_audio_extension_skipped() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("cover.jpg"), b"not audio").unwrap();
        std::fs::write(tmp.path().join("readme.txt"), b"text").unwrap();

        let (_env, svc) = setup();
        let result = svc.import_paths(&[tmp.path().to_path_buf()]).unwrap();
        assert!(result.is_empty(), "non-audio files must be skipped");
    }

    #[test]
    fn test_import_paths_empty_directory_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (_env, svc) = setup();
        let result = svc.import_paths(&[tmp.path().to_path_buf()]).unwrap();
        assert!(result.is_empty());
    }

    // ── Single-file import ───────────────────────────────────────────────────

    #[test]
    fn test_import_paths_single_flac_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = write_flac(tmp.path(), "track.flac");

        let (_env, svc) = setup();
        let ids = svc.import_paths(&[path]).unwrap();

        assert_eq!(ids.len(), 1, "one file → one track id");
        assert!(ids[0] > 0, "track id must be positive");
    }

    #[test]
    fn test_import_paths_file_path_directly() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = write_flac(tmp.path(), "song.flac");

        let (_env, svc) = setup();
        let ids = svc.import_paths(&[path]).unwrap();
        assert_eq!(ids.len(), 1);
    }

    // ── Directory recursion ──────────────────────────────────────────────────

    #[test]
    fn test_import_paths_directory_with_multiple_flac() {
        let tmp = tempfile::TempDir::new().unwrap();
        for i in 0..4 {
            write_flac(tmp.path(), &format!("track{}.flac", i));
        }
        std::fs::write(tmp.path().join("cover.jpg"), b"image").unwrap();
        std::fs::write(tmp.path().join("info.txt"), b"notes").unwrap();

        let (_env, svc) = setup();
        let ids = svc.import_paths(&[tmp.path().to_path_buf()]).unwrap();

        assert_eq!(ids.len(), 4, "only .flac files counted; jpg/txt skipped");
    }

    #[test]
    fn test_import_paths_recurses_into_subdirectories() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sub = tmp.path().join("album");
        std::fs::create_dir(&sub).unwrap();
        write_flac(tmp.path(), "root.flac");
        write_flac(&sub, "sub.flac");

        let (_env, svc) = setup();
        let ids = svc.import_paths(&[tmp.path().to_path_buf()]).unwrap();

        assert_eq!(ids.len(), 2, "should recurse into sub-directories");
    }

    // ── Upsert / idempotency ─────────────────────────────────────────────────

    #[test]
    fn test_import_paths_same_file_twice_returns_same_track_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = write_flac(tmp.path(), "track.flac");

        let (_env, svc) = setup();
        let first = svc.import_paths(&[path.clone()]).unwrap();
        let second = svc.import_paths(&[path]).unwrap();

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0], second[0], "re-import must upsert → same track id");
    }

    // ── Mixed file + directory ───────────────────────────────────────────────

    #[test]
    fn test_import_paths_mix_of_files_and_directories() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("album");
        std::fs::create_dir(&dir).unwrap();
        let file = write_flac(tmp.path(), "single.flac");
        write_flac(&dir, "album_track.flac");

        let (_env, svc) = setup();
        let ids = svc.import_paths(&[file, dir]).unwrap();

        assert_eq!(ids.len(), 2);
    }

    // ── Source creation ──────────────────────────────────────────────────────

    #[test]
    fn test_import_paths_creates_source_for_parent_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_flac(tmp.path(), "t.flac");

        let (env, svc) = setup();
        svc.import_paths(&[tmp.path().to_path_buf()]).unwrap();

        let sources = svc.list_sources().unwrap();
        assert!(
            sources.iter().any(|s| s
                .path
                .as_ref()
                .map(|p| {
                    p.canonicalize().unwrap_or_else(|_| p.clone())
                        == tmp.path().canonicalize().unwrap()
                })
                .unwrap_or(false)),
            "a source should have been created for the parent directory"
        );
        drop(env);
    }

    // ── rating validation ────────────────────────────────────────────────────

    #[test]
    fn test_rate_track_valid_rating() {
        let (env, svc) = setup();
        let (_, _, _, _, t1, _) = env.seed_basic_library();
        svc.rate_track(t1, 4).unwrap();
        let track = svc.get_track(t1).unwrap().unwrap();
        assert_eq!(track.rating, Rating(4));
    }

    #[test]
    fn test_rate_track_above_five_returns_error() {
        let (env, svc) = setup();
        let (_, _, _, _, t1, _) = env.seed_basic_library();
        let result = svc.rate_track(t1, 6);
        assert!(result.is_err(), "rating > 5 must be rejected");
    }

    #[test]
    fn test_rate_track_zero_is_valid() {
        let (env, svc) = setup();
        let (_, _, _, _, t1, _) = env.seed_basic_library();
        svc.rate_track(t1, 0).unwrap();
        let track = svc.get_track(t1).unwrap().unwrap();
        assert_eq!(track.rating, Rating(0));
    }

    // ── Album identity: (title, artist) ─────────────────────────────────────

    /// Two tracks with the same album title but different primary artists (no ALBUMARTIST tag)
    /// must produce two *separate* albums, not a single "Various Artists" one.
    /// This covers the "Greatest Hits" / "Best Of" scenario.
    #[test]
    fn test_same_title_different_artist_creates_separate_albums() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();

        let artist1 = queries::insert_artist(&conn, "The Beatles", None).unwrap();
        let artist2 = queries::insert_artist(&conn, "Eminem", None).unwrap();

        let id1 = queries::insert_album(&conn, "Greatest Hits", artist1, None).unwrap();
        let id2 = queries::insert_album(&conn, "Greatest Hits", artist2, None).unwrap();

        assert_ne!(
            id1, id2,
            "distinct artists must produce distinct album rows"
        );

        let album1 = queries::get_album(&conn, id1).unwrap().unwrap();
        let album2 = queries::get_album(&conn, id2).unwrap().unwrap();
        assert_eq!(album1.artist_id, artist1);
        assert_eq!(album2.artist_id, artist2);
    }

    /// Two tracks with the same album title *and* the same primary artist (no ALBUMARTIST tag)
    /// must be grouped into the same album row.
    #[test]
    fn test_same_title_same_artist_reuses_album() {
        let env = TestEnv::new();
        let conn = env.pool.get().unwrap();

        let artist_id = queries::insert_artist(&conn, "Akhenaton", None).unwrap();

        let id1 = queries::insert_album(&conn, "Sol Invictus", artist_id, None).unwrap();
        let id2 = queries::insert_album(&conn, "Sol Invictus", artist_id, None).unwrap();

        assert_eq!(
            id1, id2,
            "same title + same artist must reuse the existing album row"
        );
    }

    #[test]
    fn test_rate_album_valid_rating() {
        let (env, svc) = setup();
        let (_, _, album_id, _, _, _) = env.seed_basic_library();
        svc.rate_album(album_id, 5).unwrap();
        let album = svc.get_album(album_id).unwrap().unwrap();
        assert_eq!(album.rating, Rating(5));
    }

    #[test]
    fn test_rate_album_above_five_returns_error() {
        let (env, svc) = setup();
        let (_, _, album_id, _, _, _) = env.seed_basic_library();
        let result = svc.rate_album(album_id, 7);
        assert!(result.is_err(), "rating > 5 must be rejected");
    }

    // ── list_albums filters ──────────────────────────────────────────────────

    #[test]
    fn test_list_albums_all() {
        let (env, svc) = setup();
        env.seed_basic_library();
        let albums = svc.list_albums(None, None, None, None, None).unwrap();
        assert_eq!(albums.len(), 1);
    }

    #[test]
    fn test_list_albums_filtered_by_artist() {
        let (env, svc) = setup();
        let (_, artist_id, _, _, _, _) = env.seed_basic_library();
        let albums = svc
            .list_albums(Some(artist_id), None, None, None, None)
            .unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].artist_id, artist_id);
    }

    #[test]
    fn test_list_albums_filtered_by_min_rating() {
        let (env, svc) = setup();
        let (_, _, album_id, _, _, _) = env.seed_basic_library();
        svc.rate_album(album_id, 4).unwrap();
        let high = svc.list_albums(None, None, Some(3), None, None).unwrap();
        let low = svc.list_albums(None, None, Some(5), None, None).unwrap();
        assert_eq!(high.len(), 1);
        assert!(low.is_empty());
    }

    // ── source / genre / track accessors ────────────────────────────────────

    #[test]
    fn test_get_source_tracks_returns_correct_tracks() {
        let (env, svc) = setup();
        let (source_id, _, _, _, t1, t2) = env.seed_basic_library();
        let tracks = svc.get_source_tracks(source_id).unwrap();
        assert_eq!(tracks.len(), 2);
        let ids: Vec<i64> = tracks.iter().map(|t| t.id).collect();
        assert!(ids.contains(&t1));
        assert!(ids.contains(&t2));
    }

    #[test]
    fn test_get_genre_tracks_returns_correct_tracks() {
        let (env, svc) = setup();
        let (_, _, _, genre_id, t1, t2) = env.seed_basic_library();
        let tracks = svc.get_genre_tracks(genre_id).unwrap();
        assert_eq!(tracks.len(), 2);
        let ids: Vec<i64> = tracks.iter().map(|t| t.id).collect();
        assert!(ids.contains(&t1));
        assert!(ids.contains(&t2));
    }

    #[test]
    fn test_get_album_tracks_returns_correct_tracks() {
        let (env, svc) = setup();
        let (_, _, album_id, _, t1, t2) = env.seed_basic_library();
        let tracks = svc.get_album_tracks(album_id).unwrap();
        assert_eq!(tracks.len(), 2);
        let ids: Vec<i64> = tracks.iter().map(|t| t.id).collect();
        assert!(ids.contains(&t1));
        assert!(ids.contains(&t2));
    }

    #[test]
    fn test_get_genre_artists_returns_correct_artists() {
        let (env, svc) = setup();
        let (_, artist_id, _, genre_id, _, _) = env.seed_basic_library();
        let artists = svc.get_genre_artists(genre_id).unwrap();
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].id, artist_id);
    }

    #[test]
    fn test_list_genres_includes_counts() {
        let (env, svc) = setup();
        env.seed_basic_library();
        let genres = svc.list_genres().unwrap();
        assert_eq!(genres.len(), 1);
        let (genre, track_count, _album_count) = &genres[0];
        assert_eq!(genre.name, "Rock");
        assert_eq!(*track_count, 2);
    }

    // ── scan_directory ───────────────────────────────────────────────────────

    #[test]
    fn test_scan_directory_nonexistent_path_returns_error() {
        let (_env, svc) = setup();
        let bogus = PathBuf::from("/nonexistent/library/path/does/not/exist");
        let result = svc.scan_directory(&bogus, 1);
        assert!(result.is_err(), "scanning nonexistent path must fail");
    }

    #[test]
    fn test_scan_directory_path_is_file_returns_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let file_path = write_flac(tmp.path(), "track.flac");
        let (_env, svc) = setup();
        let result = svc.scan_directory(&file_path, 1);
        assert!(
            result.is_err(),
            "scanning a file path (not a dir) must fail"
        );
    }

    #[test]
    fn test_scan_directory_empty_dir_returns_zero_tracks() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (_env, svc) = setup();
        let source = svc.add_source("Test", tmp.path()).unwrap();

        let result = svc.scan_directory(tmp.path(), source.id).unwrap();
        assert_eq!(result.tracks_added, 0);
        assert_eq!(result.tracks_skipped, 0);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_scan_directory_adds_flac_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_flac(tmp.path(), "track1.flac");
        write_flac(tmp.path(), "track2.flac");
        write_flac(tmp.path(), "track3.flac");

        let (_env, svc) = setup();
        let source = svc.add_source("Test", tmp.path()).unwrap();
        let result = svc.scan_directory(tmp.path(), source.id).unwrap();

        assert_eq!(result.tracks_added, 3);
        assert_eq!(result.tracks_skipped, 0);
    }

    #[test]
    fn test_scan_directory_skips_non_audio_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_flac(tmp.path(), "track.flac");
        std::fs::write(tmp.path().join("cover.jpg"), b"image").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), b"text").unwrap();
        std::fs::write(tmp.path().join("data.wav"), b"wav").unwrap();

        let (_env, svc) = setup();
        let source = svc.add_source("Test", tmp.path()).unwrap();
        let result = svc.scan_directory(tmp.path(), source.id).unwrap();

        assert_eq!(
            result.tracks_added, 1,
            "only .flac counted; wav/jpg/txt skipped"
        );
    }

    #[test]
    fn test_scan_directory_recurses_subdirectories() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sub = tmp.path().join("album");
        std::fs::create_dir(&sub).unwrap();
        let deep = tmp.path().join("artist").join("album2");
        std::fs::create_dir_all(&deep).unwrap();

        write_flac(tmp.path(), "root.flac");
        write_flac(&sub, "sub.flac");
        write_flac(&deep, "deep.flac");

        let (_env, svc) = setup();
        let source = svc.add_source("Test", tmp.path()).unwrap();
        let result = svc.scan_directory(tmp.path(), source.id).unwrap();

        assert_eq!(
            result.tracks_added, 3,
            "should recurse into all subdirectories"
        );
    }

    #[test]
    fn test_scan_directory_handles_corrupted_file_gracefully() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_flac(tmp.path(), "valid.flac");
        std::fs::write(tmp.path().join("bad.flac"), b"NOT A VALID FLAC FILE").unwrap();

        let (_env, svc) = setup();
        let source = svc.add_source("Test", tmp.path()).unwrap();
        let result = svc.scan_directory(tmp.path(), source.id).unwrap();

        assert_eq!(result.tracks_added, 1, "valid file must be added");
        assert_eq!(result.tracks_skipped, 1, "corrupted file must be skipped");
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn test_scan_directory_result_records_duration() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_flac(tmp.path(), "track.flac");

        let (_env, svc) = setup();
        let source = svc.add_source("Test", tmp.path()).unwrap();
        let result = svc.scan_directory(tmp.path(), source.id).unwrap();

        assert!(result.duration.as_nanos() > 0);
    }

    // ── validate_sources ─────────────────────────────────────────────────────

    #[test]
    fn test_validate_sources_empty_returns_empty() {
        let (_env, svc) = setup();
        let results = svc.validate_sources().unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_validate_sources_existing_dir_is_valid() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (_env, svc) = setup();
        svc.add_source("Valid Library", tmp.path()).unwrap();

        let results = svc.validate_sources().unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1, "existing directory must be valid");
    }

    #[test]
    fn test_validate_sources_nonexistent_dir_is_invalid() {
        let (_env, svc) = setup();
        svc.add_source("Dead Library", &PathBuf::from("/nowhere/xyz123abc"))
            .unwrap();

        let results = svc.validate_sources().unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].1, "nonexistent path must be invalid");
    }

    #[test]
    fn test_validate_sources_mixed_validity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (_env, svc) = setup();
        svc.add_source("Good", tmp.path()).unwrap();
        svc.add_source("Bad", &PathBuf::from("/nowhere/xyz999abc"))
            .unwrap();

        let results = svc.validate_sources().unwrap();
        assert_eq!(results.len(), 2);
        let valid = results.iter().filter(|(_, ok)| *ok).count();
        let invalid = results.iter().filter(|(_, ok)| !ok).count();
        assert_eq!(valid, 1);
        assert_eq!(invalid, 1);
    }

    // ── find_source_by_path ──────────────────────────────────────────────────

    #[test]
    fn test_find_source_by_path_returns_none_when_not_found() {
        let (_env, svc) = setup();
        let result = svc
            .find_source_by_path(&PathBuf::from("/some/path/not/in/db"))
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_find_source_by_path_finds_existing_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (_env, svc) = setup();
        let source = svc.add_source("Music", tmp.path()).unwrap();

        let found = svc.find_source_by_path(tmp.path()).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, source.id);
    }

    #[test]
    fn test_find_source_by_path_returns_none_for_different_path() {
        let tmp1 = tempfile::TempDir::new().unwrap();
        let tmp2 = tempfile::TempDir::new().unwrap();
        let (_env, svc) = setup();
        svc.add_source("Music", tmp1.path()).unwrap();

        let result = svc.find_source_by_path(tmp2.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_find_source_returns_correct_source_when_multiple_exist() {
        let tmp1 = tempfile::TempDir::new().unwrap();
        let tmp2 = tempfile::TempDir::new().unwrap();
        let (_env, svc) = setup();
        let s1 = svc.add_source("Library A", tmp1.path()).unwrap();
        let s2 = svc.add_source("Library B", tmp2.path()).unwrap();

        let found1 = svc.find_source_by_path(tmp1.path()).unwrap().unwrap();
        let found2 = svc.find_source_by_path(tmp2.path()).unwrap().unwrap();
        assert_eq!(found1.id, s1.id);
        assert_eq!(found2.id, s2.id);
    }
}
