// FFI Bridge for Swift/Rust interop
// This module exposes Rust functions to Swift via swift-bridge

use std::path::PathBuf;
use std::sync::Mutex;
use once_cell::sync::Lazy;

use crate::db;
use crate::services::*;
use crate::utils::AppPaths;

// Global database pool - the only shared state we need
static DB_POOL: Lazy<Mutex<Option<db::DbPool>>> = Lazy::new(|| Mutex::new(None));

// Unsafe wrapper to make PlayerService Send+Sync (needed for rodio's OutputStream)
// SAFETY: All access is protected by Mutex, ensuring no concurrent access
struct SendSyncPlayerService(player::PlayerService);
unsafe impl Send for SendSyncPlayerService {}
unsafe impl Sync for SendSyncPlayerService {}

// Global player service (wrapped for thread safety)
static PLAYER_SERVICE: Lazy<Mutex<Option<SendSyncPlayerService>>> = Lazy::new(|| Mutex::new(None));

// Global library service (for accessing scan progress)
static LIBRARY_SERVICE: Lazy<Mutex<Option<library::LibraryService>>> = Lazy::new(|| Mutex::new(None));

// Global artwork service
static ARTWORK_SERVICE: Lazy<Mutex<Option<artwork::ArtworkService>>> = Lazy::new(|| Mutex::new(None));

// Global tokio runtime for async operations
static TOKIO_RUNTIME: Lazy<tokio::runtime::Runtime> = Lazy::new(|| {
    tokio::runtime::Runtime::new().expect("Failed to create tokio runtime")
});

fn get_or_init_pool() -> Result<db::DbPool, String> {
    let mut pool_opt = DB_POOL.lock().unwrap();

    if pool_opt.is_none() {
        // Initialize application paths
        let app_paths = AppPaths::new()
            .map_err(|e| format!("Failed to initialize app paths: {}", e))?;

        // Create database connection pool
        let pool = db::create_pool(app_paths.database_path())
            .map_err(|e| format!("Failed to create database pool: {}", e))?;

        // Initialize database schema
        db::initialize_database(&pool)
            .map_err(|e| format!("Failed to initialize database: {}", e))?;

        *pool_opt = Some(pool);
    }

    Ok(pool_opt.as_ref().unwrap().clone())
}

fn get_or_init_player() -> Result<(), String> {
    let mut player_opt = PLAYER_SERVICE.lock().unwrap();

    if player_opt.is_none() {
        // Get database pool first
        let pool = get_or_init_pool()?;

        // Create player service
        let player = player::PlayerService::new(pool)
            .map_err(|e| format!("Failed to create player service: {}", e))?;

        *player_opt = Some(SendSyncPlayerService(player));
    }

    Ok(())
}

fn get_or_init_library() -> Result<library::LibraryService, String> {
    let mut library_opt = LIBRARY_SERVICE.lock().unwrap();

    if library_opt.is_none() {
        // Get database pool first
        let pool = get_or_init_pool()?;

        // Initialize app paths
        let app_paths = AppPaths::new()
            .map_err(|e| format!("Failed to initialize app paths: {}", e))?;

        // Create library service
        let library = library::LibraryService::new(pool, app_paths);

        *library_opt = Some(library);
    }

    Ok(library_opt.as_ref().unwrap().clone())
}

fn get_or_init_artwork() -> Result<(), String> {
    let mut artwork_opt = ARTWORK_SERVICE.lock().unwrap_or_else(|e| e.into_inner());

    if artwork_opt.is_none() {
        // Get database pool first
        let pool = get_or_init_pool()?;

        // Initialize app paths
        let app_paths = AppPaths::new()
            .map_err(|e| format!("Failed to initialize app paths: {}", e))?;

        // Create artwork service
        let artwork = artwork::ArtworkService::new(pool, app_paths);

        *artwork_opt = Some(artwork);
    }

    Ok(())
}

#[swift_bridge::bridge]
mod ffi {
    extern "Rust" {
        // Library Management Functions
        fn get_library_stats() -> String;
        fn scan_library(folder_path: &str) -> String;
        fn get_scan_progress() -> String;

        // Track Functions
        fn get_tracks_page(offset: u32, limit: u32, sort_by: &str, sort_dir: &str) -> String;
        fn get_track_by_id(track_id: i64) -> String;
        fn get_random_tracks(count: u32) -> String;
        fn search_tracks(query: &str, limit: u32) -> String;
        fn delete_tracks(track_ids_json: &str) -> String;

        // Album Functions
        fn get_albums_page(offset: u32, limit: u32) -> String;
        fn get_album_by_id(album_id: i64) -> String;
        fn get_album_tracks(album_id: i64) -> String;
        fn get_album_artwork(album_id: i64) -> Vec<u8>;

        // Artist Functions
        fn get_artists_page(offset: u32, limit: u32) -> String;
        fn get_artist_by_id(artist_id: i64) -> String;

        // Genre Functions
        fn get_genres() -> String;
        fn get_genre_tracks(genre_id: i64) -> String;
        fn get_genre_artists(genre_id: i64) -> String;
        fn get_genre_albums(genre_id: i64) -> String;
        fn get_album_genres(album_id: i64) -> String;
        fn get_artist_genres(artist_id: i64) -> String;

        // Playlist Functions
        fn get_playlists() -> String;
        fn create_playlist(name: &str) -> String;
        fn rename_playlist(playlist_id: i64, name: &str) -> String;
        fn delete_playlist(playlist_id: i64) -> String;
        fn add_track_to_playlist(playlist_id: i64, track_id: i64) -> String;
        fn remove_track_from_playlist(playlist_id: i64, position: i64) -> String;

        // Playback Control Functions
        fn play_track(track_id: i64) -> String;
        fn pause_playback() -> String;
        fn resume_playback() -> String;
        fn stop_playback() -> String;
        fn next_track() -> String;
        fn previous_track() -> String;
        fn jump_to_queue_index(index: i64) -> String;
        fn get_player_state() -> String;

        // Queue Management Functions
        fn get_queue() -> String;
        fn add_to_queue(track_id: i64) -> String;
        fn add_tracks_to_queue(track_ids: &str) -> String;
        fn set_queue(track_ids: &str) -> String;
        fn clear_queue() -> String;
        fn reorder_queue(track_ids: &str) -> String;

        // Audio Control Functions
        fn set_volume(volume: f64) -> String;
        fn seek_to_position(position: f64) -> String;

        // Playback Mode Functions
        fn toggle_shuffle() -> String;
        fn set_shuffle(enabled: bool) -> String;
        fn toggle_repeat() -> String;
        fn set_repeat_mode(mode: &str) -> String;

        // Import Functions
        fn import_files(paths_json: &str) -> String;

        // Artwork Fetching Functions
        fn fetch_all_artwork(fetch_artists: bool) -> String;
        fn fetch_album_artwork(album_id: i64) -> String;
        fn get_artwork_fetch_progress() -> String;
        fn cancel_artwork_fetch() -> String;
        fn get_album_artwork_with_online(album_id: i64) -> Vec<u8>;
        fn get_artist_photo(artist_id: i64) -> Vec<u8>;
    }
}

// Function implementations will be added in subsequent tasks (T012-T036)
// Each function returns JSON-serialized results for cross-language data transfer

fn get_library_stats() -> String {
    // T012: Get library statistics (album count, artist count, track count)
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            // Query counts and totals in one pass
            let album_count: Result<i64, _> = conn.query_row(
                "SELECT COUNT(*) FROM albums", [], |row| row.get(0)
            );
            let artist_count: Result<i64, _> = conn.query_row(
                "SELECT COUNT(*) FROM artists", [], |row| row.get(0)
            );
            let track_count: Result<i64, _> = conn.query_row(
                "SELECT COUNT(*) FROM tracks", [], |row| row.get(0)
            );
            let total_duration: Result<i64, _> = conn.query_row(
                "SELECT COALESCE(SUM(duration), 0) / 1000 FROM tracks", [], |row| row.get(0)
            );
            let artwork_count: Result<i64, _> = conn.query_row(
                "SELECT COUNT(*) FROM albums WHERE online_artwork_path IS NOT NULL", [], |row| row.get(0)
            );

            match (album_count, artist_count, track_count, total_duration, artwork_count) {
                (Ok(albums), Ok(artists), Ok(tracks), Ok(duration), Ok(artworks)) => {
                    serde_json::json!({
                        "success": true,
                        "data": {
                            "album_count": albums,
                            "artist_count": artists,
                            "track_count": tracks,
                            "total_duration_seconds": duration,
                            "artwork_count": artworks,
                        }
                    }).to_string()
                }
                _ => {
                    serde_json::json!({
                        "success": false,
                        "error": "Failed to query library statistics"
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn scan_library(folder_path: &str) -> String {
    // T013: Scan a folder for music files and add to library
    match get_or_init_library() {
        Ok(library_service) => {
            let path = PathBuf::from(folder_path);

            // First, add the source
            match library_service.add_source("Music Library", &path) {
                Ok(source) => {
                    // Then scan it
                    match library_service.scan_directory(&path, source.id) {
                        Ok(result) => {
                            serde_json::json!({
                                "success": true,
                                "data": {
                                    "source_id": source.id,
                                    "tracks_added": result.tracks_added,
                                    "tracks_updated": result.tracks_updated,
                                    "tracks_skipped": result.tracks_skipped,
                                    "errors_count": result.errors.len(),
                                    "duration_ms": result.duration.as_millis()
                                }
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to scan directory: {}", e)
                            }).to_string()
                        }
                    }
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to add source: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_scan_progress() -> String {
    // Get the current scan progress from the global library service
    match get_or_init_library() {
        Ok(library_service) => {
            match library_service.scan_progress() {
                Some(progress) => {
                    serde_json::json!({
                        "success": true,
                        "data": {
                            "total_files": progress.total_files,
                            "processed_files": progress.processed_files,
                            "tracks_added": progress.tracks_added,
                            "current_file": progress.current_file.as_ref().map(|p| p.to_string_lossy().to_string())
                        }
                    }).to_string()
                }
                None => {
                    serde_json::json!({
                        "success": true,
                        "data": null
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("Failed to get scan progress: {}", e)
            }).to_string()
        }
    }
}

fn get_tracks_page(offset: u32, limit: u32, sort_by: &str, sort_dir: &str) -> String {
    // T014: Get paginated list of tracks (limit capped at 100)
    match get_or_init_pool() {
        Ok(pool) => {
            let limit_capped = std::cmp::min(limit, 100) as usize;
            let offset_val = offset as usize;

            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            // Map sort_by to the appropriate SQL expression
            let order_col = match sort_by {
                "artist" => "ar.name",
                "album"  => "al.title",
                "duration" => "t.duration",
                "track_number" => "t.disc_number * 1000 + COALESCE(t.track_number, 0)",
                "file_size" => "t.file_size",
                "play_count" => "t.play_count",
                "rating" => "t.rating",
                "file_path" => "t.file_path",
                _ => "t.title",
            };
            let order_dir = if sort_dir.eq_ignore_ascii_case("desc") { "DESC" } else { "ASC" };

            // Query tracks with pagination and dynamic ORDER BY
            // LEFT JOINs allow sorting by artist name or album title
            let query = format!(
                "SELECT t.id, t.title, t.artist_id, t.album_id, t.source_id, t.file_path, t.duration, \
                 t.track_number, t.disc_number, t.sample_rate, t.bit_depth, t.file_type, t.file_size, \
                 t.rating, t.fingerprint, t.is_duplicate, t.duplicate_of, t.last_played_at, t.play_count, \
                 t.created_at \
                 FROM tracks t \
                 LEFT JOIN artists ar ON t.artist_id = ar.id \
                 LEFT JOIN albums al ON t.album_id = al.id \
                 ORDER BY {} {} LIMIT {} OFFSET {}",
                order_col, order_dir, limit_capped, offset_val
            );

            match conn.prepare(&query) {
                Ok(mut stmt) => {
                    let tracks_iter = stmt.query_map([], |row| {
                        Ok(serde_json::json!({
                            "id": row.get::<_, i64>(0)?,
                            "title": row.get::<_, String>(1)?,
                            "artist_id": row.get::<_, i64>(2)?,
                            "album_id": row.get::<_, Option<i64>>(3)?,
                            "source_id": row.get::<_, i64>(4)?,
                            "file_path": row.get::<_, String>(5)?,
                            "duration": row.get::<_, i64>(6)?,  // Duration in milliseconds
                            "track_number": row.get::<_, Option<u32>>(7)?,
                            "disc_number": row.get::<_, u32>(8)?,
                            "sample_rate": row.get::<_, Option<u32>>(9)?,
                            "bit_depth": row.get::<_, Option<u8>>(10)?,
                            "file_type": row.get::<_, String>(11)?,
                            "file_size": row.get::<_, i64>(12)?,
                            "rating": row.get::<_, u8>(13)?,
                            "fingerprint": row.get::<_, Option<String>>(14)?,
                            "is_duplicate": row.get::<_, bool>(15)?,
                            "duplicate_of": row.get::<_, Option<i64>>(16)?,
                            "last_played_at": row.get::<_, Option<String>>(17)?,
                            "play_count": row.get::<_, u32>(18)?,
                            "created_at": row.get::<_, Option<String>>(19)?,
                        }))
                    });

                    match tracks_iter {
                        Ok(tracks) => {
                            let tracks_vec: Result<Vec<_>, _> = tracks.collect();
                            match tracks_vec {
                                Ok(tracks_data) => {
                                    serde_json::json!({
                                        "success": true,
                                        "data": {
                                            "tracks": tracks_data,
                                            "offset": offset,
                                            "limit": limit_capped
                                        }
                                    }).to_string()
                                }
                                Err(e) => {
                                    serde_json::json!({
                                        "success": false,
                                        "error": format!("Failed to fetch tracks: {}", e)
                                    }).to_string()
                                }
                            }
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to query tracks: {}", e)
                            }).to_string()
                        }
                    }
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to prepare query: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_random_tracks(count: u32) -> String {
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            let count_capped = std::cmp::min(count, 200) as usize;

            match conn.prepare(&format!(
                "SELECT id FROM tracks ORDER BY RANDOM() LIMIT {}",
                count_capped
            )) {
                Ok(mut stmt) => {
                    let ids_iter = stmt.query_map([], |row| row.get::<_, i64>(0));
                    match ids_iter {
                        Ok(rows) => {
                            let track_ids: Result<Vec<i64>, _> = rows.collect();
                            match track_ids {
                                Ok(ids) => {
                                    serde_json::json!({
                                        "success": true,
                                        "data": ids
                                    }).to_string()
                                }
                                Err(e) => {
                                    serde_json::json!({
                                        "success": false,
                                        "error": format!("Failed to fetch random tracks: {}", e)
                                    }).to_string()
                                }
                            }
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to query random tracks: {}", e)
                            }).to_string()
                        }
                    }
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to prepare query: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_track_by_id(track_id: i64) -> String {
    // T015: Get a single track by ID
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            match crate::db::queries::get_track(&conn, track_id) {
                Ok(Some(track)) => {
                    serde_json::json!({
                        "success": true,
                        "data": track
                    }).to_string()
                }
                Ok(None) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Track {} not found", track_id)
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get track: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn delete_tracks(track_ids_json: &str) -> String {
    let track_ids: Vec<i64> = match serde_json::from_str(track_ids_json) {
        Ok(ids) => ids,
        Err(e) => {
            return serde_json::json!({
                "success": false,
                "error": format!("Invalid track IDs JSON: {}", e)
            }).to_string();
        }
    };

    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            let mut deleted = 0usize;
            for track_id in &track_ids {
                match crate::db::queries::delete_track(&conn, *track_id) {
                    Ok(_) => deleted += 1,
                    Err(e) => {
                        return serde_json::json!({
                            "success": false,
                            "error": format!("Failed to delete track {}: {}", track_id, e)
                        }).to_string();
                    }
                }
            }

            serde_json::json!({
                "success": true,
                "data": { "deleted": deleted }
            }).to_string()
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn search_tracks(query: &str, limit: u32) -> String {
    // T016: FTS5 search
    match get_or_init_pool() {
        Ok(pool) => {
            let search_service = SearchService::new(pool.clone());
            match search_service.search(query) {
                Ok(results) => {
                    // Apply limit to results
                    let limit_val = limit as usize;
                    let tracks: Vec<_> = results.tracks.into_iter().take(limit_val).collect();
                    let albums: Vec<_> = results.albums.into_iter().take(limit_val).collect();
                    let artists: Vec<_> = results.artists.into_iter().take(limit_val).collect();

                    serde_json::json!({
                        "success": true,
                        "data": {
                            "tracks": tracks,
                            "albums": albums,
                            "artists": artists
                        }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Search failed: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_album_by_id(album_id: i64) -> String {
    // Get a single album by ID
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            match crate::db::queries::get_album(&conn, album_id) {
                Ok(Some(album)) => {
                    serde_json::json!({
                        "success": true,
                        "data": album
                    }).to_string()
                }
                Ok(None) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Album with ID {} not found", album_id)
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get album: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("Database pool error: {}", e)
            }).to_string()
        }
    }
}

fn get_albums_page(offset: u32, limit: u32) -> String {
    // T017: Get paginated list of albums
    match get_or_init_pool() {
        Ok(pool) => {
            let limit_capped = std::cmp::min(limit, 100) as usize;
            let offset_val = offset as usize;

            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            match crate::db::queries::list_albums(
                &conn,
                None,  // artist_id
                None,  // genre_id
                None,  // min_rating
                Some(limit_capped),
                Some(offset_val),
            ) {
                Ok(albums) => {
                    serde_json::json!({
                        "success": true,
                        "data": {
                            "albums": albums,
                            "offset": offset,
                            "limit": limit_capped
                        }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to list albums: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_artist_by_id(artist_id: i64) -> String {
    // Get a single artist by ID
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            match crate::db::queries::get_artist(&conn, artist_id) {
                Ok(Some(artist)) => {
                    serde_json::json!({
                        "success": true,
                        "data": artist
                    }).to_string()
                }
                Ok(None) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Artist with ID {} not found", artist_id)
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get artist: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("Database pool error: {}", e)
            }).to_string()
        }
    }
}

fn get_artists_page(offset: u32, limit: u32) -> String {
    // T018: Get paginated list of artists
    match get_or_init_pool() {
        Ok(pool) => {
            let limit_capped = std::cmp::min(limit, 100) as usize;
            let offset_val = offset as usize;

            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            match crate::db::queries::list_artists(&conn) {
                Ok(all_artists) => {
                    // Apply pagination manually
                    let artists: Vec<_> = all_artists.into_iter()
                        .skip(offset_val)
                        .take(limit_capped)
                        .collect();

                    serde_json::json!({
                        "success": true,
                        "data": {
                            "artists": artists,
                            "offset": offset,
                            "limit": limit_capped
                        }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to list artists: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_album_artwork(album_id: i64) -> Vec<u8> {
    // T019: Get album artwork as raw bytes
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(_) => return Vec::new(),
            };

            match crate::db::queries::get_album(&conn, album_id) {
                Ok(Some(album)) => {
                    if let Some(artwork_path) = album.artwork_path {
                        if let Ok(bytes) = std::fs::read(&artwork_path) {
                            return bytes;
                        }
                    }
                    Vec::new()
                }
                _ => Vec::new(),
            }
        }
        Err(_) => Vec::new(),
    }
}

fn get_album_tracks(album_id: i64) -> String {
    // Get all tracks for a specific album
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            match crate::db::queries::get_album_tracks(&conn, album_id) {
                Ok(tracks) => {
                    serde_json::json!({
                        "success": true,
                        "data": { "tracks": tracks }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get album tracks: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("Database pool error: {}", e)
            }).to_string()
        }
    }
}

fn get_playlists() -> String {
    // T020: Get all playlists
    match get_or_init_pool() {
        Ok(pool) => {
            let playlist_service = PlaylistService::new(pool.clone());
            match playlist_service.list_playlists() {
                Ok(playlists) => {
                    serde_json::json!({
                        "success": true,
                        "data": { "playlists": playlists }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to list playlists: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn create_playlist(name: &str) -> String {
    // T021: Create a new playlist
    match get_or_init_pool() {
        Ok(pool) => {
            let playlist_service = PlaylistService::new(pool.clone());
            match playlist_service.create_playlist(name, None) {
                Ok(playlist) => {
                    serde_json::json!({
                        "success": true,
                        "data": playlist
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to create playlist: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn rename_playlist(playlist_id: i64, name: &str) -> String {
    // Rename an existing playlist
    match get_or_init_pool() {
        Ok(pool) => {
            let playlist_service = PlaylistService::new(pool.clone());
            match playlist_service.rename_playlist(playlist_id, name) {
                Ok(()) => {
                    serde_json::json!({
                        "success": true,
                        "data": { "playlist_id": playlist_id, "name": name }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to rename playlist: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn delete_playlist(playlist_id: i64) -> String {
    // T022: Delete a playlist
    match get_or_init_pool() {
        Ok(pool) => {
            let playlist_service = PlaylistService::new(pool.clone());
            match playlist_service.delete_playlist(playlist_id) {
                Ok(_) => {
                    serde_json::json!({
                        "success": true,
                        "data": { "playlist_id": playlist_id }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to delete playlist: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn add_track_to_playlist(playlist_id: i64, track_id: i64) -> String {
    // T023: Add a track to a playlist
    match get_or_init_pool() {
        Ok(pool) => {
            let playlist_service = PlaylistService::new(pool.clone());
            match playlist_service.add_tracks(playlist_id, vec![track_id]) {
                Ok(_) => {
                    serde_json::json!({
                        "success": true,
                        "data": {
                            "playlist_id": playlist_id,
                            "track_id": track_id
                        }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to add track to playlist: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn remove_track_from_playlist(playlist_id: i64, position: i64) -> String {
    // Remove a track from a playlist at the given position
    match get_or_init_pool() {
        Ok(pool) => {
            let playlist_service = PlaylistService::new(pool.clone());
            match playlist_service.remove_track(playlist_id, position as usize) {
                Ok(()) => {
                    serde_json::json!({
                        "success": true,
                        "data": {
                            "playlist_id": playlist_id,
                            "position": position
                        }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to remove track from playlist: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn play_track(track_id: i64) -> String {
    // T024: Play a specific track
    // Sets queue to just this track and starts playback
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    // Set queue to this single track
                    if let Err(e) = player_service.set_queue(vec![track_id]) {
                        return serde_json::json!({
                            "success": false,
                            "error": format!("Failed to set queue: {}", e)
                        }).to_string();
                    }

                    // Now play it
                    match player_service.play(track_id) {
                        Ok(()) => {
                            serde_json::json!({
                                "success": true,
                                "data": "Track playing"
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to play track: {}", e)
                            }).to_string()
                        }
                    }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn pause_playback() -> String {
    // T025: Pause playback
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    match player_service.pause() {
                        Ok(()) => {
                            serde_json::json!({
                                "success": true,
                                "data": "Playback paused"
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to pause: {}", e)
                            }).to_string()
                        }
                    }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn resume_playback() -> String {
    // T026: Resume playback
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    match player_service.resume() {
                        Ok(()) => {
                            serde_json::json!({
                                "success": true,
                                "data": "Playback resumed"
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to resume: {}", e)
                            }).to_string()
                        }
                    }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn stop_playback() -> String {
    // T027: Stop playback
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    match player_service.stop() {
                        Ok(()) => {
                            serde_json::json!({
                                "success": true,
                                "data": "Playback stopped"
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to stop: {}", e)
                            }).to_string()
                        }
                    }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn next_track() -> String {
    // T028: Skip to next track
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    match player_service.next() {
                        Ok(()) => {
                            serde_json::json!({
                                "success": true,
                                "data": "Skipped to next track"
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to skip to next: {}", e)
                            }).to_string()
                        }
                    }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn previous_track() -> String {
    // T029: Go to previous track
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    match player_service.previous() {
                        Ok(()) => {
                            serde_json::json!({
                                "success": true,
                                "data": "Skipped to previous track"
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to skip to previous: {}", e)
                            }).to_string()
                        }
                    }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn jump_to_queue_index(index: i64) -> String {
    // Jump to specific index in queue
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    match player_service.jump_to_index(index as usize) {
                        Ok(()) => {
                            serde_json::json!({
                                "success": true,
                                "data": "Jumped to queue index"
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to jump to index: {}", e)
                            }).to_string()
                        }
                    }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_player_state() -> String {
    // T030: Get current player state
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;

                    // Auto-advance: this is called every ~250 ms from Swift,
                    // making it the heartbeat for detecting end-of-track.
                    if player_service.is_track_finished() {
                        log::info!("[auto-advance] track finished, advancing to next");
                        if let Err(e) = player_service.next() {
                            log::warn!("[auto-advance] next() failed: {}", e);
                        }
                    }

                    use crate::services::events::PlaybackState;

                    let current_track = player_service.get_current_track();
                    let playback_state = player_service.get_state();
                    let volume = player_service.get_volume();
                    let shuffle = player_service.is_shuffle_enabled();
                    let repeat_mode = player_service.get_repeat_mode();
                    let skipped_track_ids = player_service.get_skipped_track_ids();

                    // Convert PlaybackState to is_playing boolean
                    let is_playing = matches!(playback_state, PlaybackState::Playing);

                    // Extract track ID, duration, and position
                    let current_track_id = current_track.as_ref().map(|t| t.id);
                    let duration = current_track.as_ref()
                        .map(|t| t.duration.as_secs_f64())
                        .unwrap_or(0.0);
                    let position = player_service.get_position();

                    let json_result = serde_json::json!({
                        "success": true,
                        "data": {
                            "is_playing": is_playing,
                            "current_track_id": current_track_id,
                            "position": position,
                            "duration": duration,
                            "volume": volume as f64,  // Cast f32 to f64 for JSON
                            "shuffle": shuffle,
                            "repeat_mode": repeat_mode,
                            "skipped_track_ids": skipped_track_ids
                        }
                    });
                    log::debug!("get_player_state JSON: {}", json_result);
                    json_result.to_string()
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_queue() -> String {
    // T031: Get current playback queue
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    let queue = player_service.get_queue();
                    let current_index = player_service.get_queue_index();
                    let shuffle = player_service.is_shuffle_enabled();
                    let shuffle_order = player_service.get_shuffle_order();
                    let repeat_mode = player_service.get_repeat_mode();

                    serde_json::json!({
                        "success": true,
                        "data": {
                            "items": queue,  // Changed from "queue" to "items" to match Swift model
                            "current_index": current_index,
                            "shuffle": shuffle,
                            "shuffle_order": shuffle_order,
                            "repeat_mode": repeat_mode
                        }
                    }).to_string()
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn add_to_queue(track_id: i64) -> String {
    // T032: Add track to queue
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    match player_service.add_to_queue(vec![track_id]) {
                        Ok(()) => {
                            serde_json::json!({
                                "success": true,
                                "data": "Track added to queue"
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to add to queue: {}", e)
                            }).to_string()
                        }
                    }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn add_tracks_to_queue(track_ids: &str) -> String {
    // Add multiple tracks to queue
    match get_or_init_player() {
        Ok(()) => {
            // Parse JSON array of track IDs
            let ids: Vec<i64> = match serde_json::from_str(track_ids) {
                Ok(ids) => ids,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to parse track IDs: {}", e)
                    }).to_string();
                }
            };

            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                match player_service.add_to_queue(ids.clone()) {
                    Ok(()) => {
                        serde_json::json!({
                            "success": true,
                            "data": format!("{} tracks added to queue", ids.len())
                        }).to_string()
                    }
                    Err(e) => {
                        serde_json::json!({
                            "success": false,
                            "error": format!("Failed to add tracks to queue: {}", e)
                        }).to_string()
                    }
                }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn set_queue(track_ids: &str) -> String {
    // Replace the entire queue with new tracks
    match get_or_init_player() {
        Ok(()) => {
            // Parse JSON array of track IDs
            let ids: Vec<i64> = match serde_json::from_str(track_ids) {
                Ok(ids) => ids,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to parse track IDs: {}", e)
                    }).to_string();
                }
            };

            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                match player_service.set_queue(ids.clone()) {
                    Ok(()) => {
                        serde_json::json!({
                            "success": true,
                            "data": format!("Queue set with {} tracks", ids.len())
                        }).to_string()
                    }
                    Err(e) => {
                        serde_json::json!({
                            "success": false,
                            "error": format!("Failed to set queue: {}", e)
                        }).to_string()
                    }
                }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn clear_queue() -> String {
    // T033: Clear the playback queue
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    match player_service.clear_queue() {
                        Ok(()) => {
                            serde_json::json!({
                                "success": true,
                                "data": "Queue cleared"
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to clear queue: {}", e)
                            }).to_string()
                        }
                    }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn reorder_queue(track_ids: &str) -> String {
    // T034: Reorder queue with provided track IDs (JSON array string)
    match get_or_init_player() {
        Ok(()) => {
            // Parse track IDs from JSON array
            let track_ids_vec: Vec<i64> = match serde_json::from_str(track_ids) {
                Ok(ids) => ids,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to parse track IDs: {}", e)
                    }).to_string();
                }
            };

            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    match player_service.set_queue(track_ids_vec) {
                        Ok(()) => {
                            serde_json::json!({
                                "success": true,
                                "data": "Queue reordered"
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to reorder queue: {}", e)
                            }).to_string()
                        }
                    }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn set_volume(volume: f64) -> String {
    // T035: Set playback volume (0.0 - 1.0)
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    match player_service.set_volume(volume as f32) {
                        Ok(()) => {
                            serde_json::json!({
                                "success": true,
                                "data": format!("Volume set to {}", volume)
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Failed to set volume: {}", e)
                            }).to_string()
                        }
                    }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn seek_to_position(position: f64) -> String {
    // T036: Seek to specific position in track (in seconds)
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                    use std::time::Duration;
                    let duration = Duration::from_secs_f64(position);
                    match player_service.seek(duration) {
                        Ok(()) => {
                            serde_json::json!({
                                "success": true,
                                "data": format!("Seeked to {}", position)
                            }).to_string()
                        }
                        Err(e) => {
                            serde_json::json!({
                                "success": false,
                                "error": format!("Seek not supported: {}", e)
                            }).to_string()
                        }
                    }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn toggle_shuffle() -> String {
    // Toggle shuffle mode on/off
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                let current = player_service.is_shuffle_enabled();
                match player_service.set_shuffle(!current) {
                    Ok(()) => {
                        serde_json::json!({
                            "success": true,
                            "data": format!("Shuffle {}", if !current { "enabled" } else { "disabled" })
                        }).to_string()
                    }
                    Err(e) => {
                        serde_json::json!({
                            "success": false,
                            "error": format!("Failed to toggle shuffle: {}", e)
                        }).to_string()
                    }
                }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn set_shuffle(enabled: bool) -> String {
    // Set shuffle mode to specific state
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                match player_service.set_shuffle(enabled) {
                    Ok(()) => {
                        serde_json::json!({
                            "success": true,
                            "data": format!("Shuffle {}", if enabled { "enabled" } else { "disabled" })
                        }).to_string()
                    }
                    Err(e) => {
                        serde_json::json!({
                            "success": false,
                            "error": format!("Failed to set shuffle: {}", e)
                        }).to_string()
                    }
                }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn toggle_repeat() -> String {
    // Cycle through repeat modes: Off -> All -> One -> Off
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                use crate::models::RepeatMode;
                let current = player_service.get_repeat_mode();
                let next = match current {
                    RepeatMode::Off => RepeatMode::All,
                    RepeatMode::All => RepeatMode::One,
                    RepeatMode::One => RepeatMode::Off,
                };
                match player_service.set_repeat(next) {
                    Ok(()) => {
                        serde_json::json!({
                            "success": true,
                            "data": format!("Repeat mode: {:?}", next)
                        }).to_string()
                    }
                    Err(e) => {
                        serde_json::json!({
                            "success": false,
                            "error": format!("Failed to toggle repeat: {}", e)
                        }).to_string()
                    }
                }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn set_repeat_mode(mode: &str) -> String {
    // Set specific repeat mode: "off", "all", or "one"
    match get_or_init_player() {
        Ok(()) => {
            let player = PLAYER_SERVICE.lock().unwrap();
            if let Some(ref wrapped) = *player {
                let player_service = &wrapped.0;
                use crate::models::RepeatMode;
                let repeat_mode = match mode.to_lowercase().as_str() {
                    "off" => RepeatMode::Off,
                    "all" => RepeatMode::All,
                    "one" => RepeatMode::One,
                    _ => {
                        return serde_json::json!({
                            "success": false,
                            "error": format!("Invalid repeat mode: {}. Use 'off', 'all', or 'one'", mode)
                        }).to_string();
                    }
                };
                match player_service.set_repeat(repeat_mode) {
                    Ok(()) => {
                        serde_json::json!({
                            "success": true,
                            "data": format!("Repeat mode: {:?}", repeat_mode)
                        }).to_string()
                    }
                    Err(e) => {
                        serde_json::json!({
                            "success": false,
                            "error": format!("Failed to set repeat mode: {}", e)
                        }).to_string()
                    }
                }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Player service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_genres() -> String {
    // Get all genres with track and album counts
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            match crate::db::queries::list_genres_with_count(&conn) {
                Ok(genres_with_counts) => {
                    // Collect up to 4 representative album IDs per genre
                    let mut genre_albums: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
                    if let Ok(mut stmt) = conn.prepare(
                        "SELECT tg.genre_id, a.id
                         FROM track_genres tg
                         JOIN tracks t ON tg.track_id = t.id
                         JOIN albums a ON t.album_id = a.id
                         GROUP BY tg.genre_id, a.id
                         ORDER BY tg.genre_id, a.id"
                    ) {
                        let _ = stmt.query_map([], |row| {
                            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                        }).map(|rows| {
                            for pair in rows.flatten() {
                                let entry = genre_albums.entry(pair.0).or_default();
                                if entry.len() < 4 {
                                    entry.push(pair.1);
                                }
                            }
                        });
                    }

                    let genres: Vec<_> = genres_with_counts.into_iter()
                        .map(|(genre, track_count, album_count)| {
                            let album_ids = genre_albums.get(&genre.id).cloned().unwrap_or_default();
                            serde_json::json!({
                                "id": genre.id,
                                "name": genre.name,
                                "track_count": track_count,
                                "album_count": album_count,
                                "album_ids": album_ids
                            })
                        })
                        .collect();

                    serde_json::json!({
                        "success": true,
                        "data": {
                            "genres": genres
                        }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to list genres: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_genre_tracks(genre_id: i64) -> String {
    // Get all tracks for a specific genre
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            match crate::db::queries::get_genre_tracks(&conn, genre_id) {
                Ok(tracks) => {
                    serde_json::json!({
                        "success": true,
                        "data": { "tracks": tracks }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get genre tracks: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_genre_artists(genre_id: i64) -> String {
    // Get all artists for a specific genre
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            match crate::db::queries::get_genre_artists(&conn, genre_id) {
                Ok(artists) => {
                    serde_json::json!({
                        "success": true,
                        "data": { "artists": artists }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get genre artists: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_genre_albums(genre_id: i64) -> String {
    // Get all albums for a specific genre
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            match crate::db::queries::get_genre_albums(&conn, genre_id) {
                Ok(albums) => {
                    serde_json::json!({
                        "success": true,
                        "data": { "albums": albums }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get genre albums: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_album_genres(album_id: i64) -> String {
    // Get all genres for a specific album
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            match crate::db::queries::get_album_genres(&conn, album_id) {
                Ok(genres) => {
                    serde_json::json!({
                        "success": true,
                        "data": { "genres": genres }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get album genres: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

fn get_artist_genres(artist_id: i64) -> String {
    // Get all genres for a specific artist (from all their tracks)
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    return serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get database connection: {}", e)
                    }).to_string();
                }
            };

            match crate::db::queries::get_artist_genres(&conn, artist_id) {
                Ok(genres) => {
                    serde_json::json!({
                        "success": true,
                        "data": { "genres": genres }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to get artist genres: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

// Import Functions

fn import_files(paths_json: &str) -> String {
    // Parse JSON array of path strings
    let path_strings: Vec<String> = match serde_json::from_str(paths_json) {
        Ok(v) => v,
        Err(e) => {
            return serde_json::json!({
                "success": false,
                "error": format!("Failed to parse paths JSON: {}", e)
            }).to_string();
        }
    };

    let path_bufs: Vec<PathBuf> = path_strings.iter().map(PathBuf::from).collect();

    match get_or_init_library() {
        Ok(library_service) => {
            match library_service.import_paths(&path_bufs) {
                Ok(track_ids) => {
                    serde_json::json!({
                        "success": true,
                        "data": {
                            "tracks_imported": track_ids.len()
                        }
                    }).to_string()
                }
                Err(e) => {
                    serde_json::json!({
                        "success": false,
                        "error": format!("Failed to import paths: {}", e)
                    }).to_string()
                }
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("FFI initialization failed: {}", e)
            }).to_string()
        }
    }
}

// Artwork Fetching Functions

fn fetch_all_artwork(fetch_artists: bool) -> String {
    // Fetch artwork for all albums and optionally artists
    match get_or_init_artwork() {
        Ok(_) => {
            // Spawn async task in background
            let artwork_opt = ARTWORK_SERVICE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(artwork_service) = artwork_opt.as_ref() {
                let service_clone = artwork_service.clone();
                TOKIO_RUNTIME.spawn(async move {
                    if let Err(e) = service_clone.fetch_all_artwork(fetch_artists).await {
                        log::error!("Artwork fetch failed: {}", e);
                    }
                });

                serde_json::json!({
                    "success": true,
                    "data": {
                        "message": "Artwork fetch started"
                    }
                }).to_string()
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Artwork service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("Failed to initialize artwork service: {}", e)
            }).to_string()
        }
    }
}

fn fetch_album_artwork(album_id: i64) -> String {
    // Fetch artwork for a specific album
    match get_or_init_artwork() {
        Ok(_) => {
            let artwork_opt = ARTWORK_SERVICE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(artwork_service) = artwork_opt.as_ref() {
                let service_clone = artwork_service.clone();
                TOKIO_RUNTIME.spawn(async move {
                    if let Err(e) = service_clone.fetch_album_artwork(album_id).await {
                        log::error!("Failed to fetch artwork for album {}: {}", album_id, e);
                    }
                });

                serde_json::json!({
                    "success": true,
                    "data": {
                        "message": format!("Fetching artwork for album {}", album_id)
                    }
                }).to_string()
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Artwork service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("Failed to initialize artwork service: {}", e)
            }).to_string()
        }
    }
}

fn get_artwork_fetch_progress() -> String {
    // Get current artwork fetch progress
    match get_or_init_artwork() {
        Ok(_) => {
            let artwork_opt = ARTWORK_SERVICE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(artwork_service) = artwork_opt.as_ref() {
                match artwork_service.get_progress() {
                    Some(progress) => {
                        serde_json::json!({
                            "success": true,
                            "data": {
                                "totalItems": progress.total_items,
                                "processedItems": progress.processed_items,
                                "currentItem": progress.current_item,
                                "successful": progress.successful,
                                "failed": progress.failed
                            }
                        }).to_string()
                    }
                    None => {
                        serde_json::json!({
                            "success": true,
                            "data": null
                        }).to_string()
                    }
                }
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Artwork service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("Failed to initialize artwork service: {}", e)
            }).to_string()
        }
    }
}

fn cancel_artwork_fetch() -> String {
    // Cancel ongoing artwork fetch
    match get_or_init_artwork() {
        Ok(_) => {
            let artwork_opt = ARTWORK_SERVICE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(artwork_service) = artwork_opt.as_ref() {
                artwork_service.cancel_fetch();
                serde_json::json!({
                    "success": true,
                    "data": {
                        "message": "Artwork fetch cancelled"
                    }
                }).to_string()
            } else {
                serde_json::json!({
                    "success": false,
                    "error": "Artwork service not initialized"
                }).to_string()
            }
        }
        Err(e) => {
            serde_json::json!({
                "success": false,
                "error": format!("Failed to initialize artwork service: {}", e)
            }).to_string()
        }
    }
}

fn get_album_artwork_with_online(album_id: i64) -> Vec<u8> {
    // Get album artwork, preferring online artwork over embedded
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(_) => return Vec::new(),
            };

            match crate::db::queries::get_album(&conn, album_id) {
                Ok(Some(album)) => {
                    // Try online artwork first
                    if let Some(online_path) = album.online_artwork_path {
                        if let Ok(bytes) = std::fs::read(&online_path) {
                            return bytes;
                        }
                    }

                    // Fall back to embedded artwork
                    if let Some(artwork_path) = album.artwork_path {
                        if let Ok(bytes) = std::fs::read(&artwork_path) {
                            return bytes;
                        }
                    }

                    Vec::new()
                }
                _ => Vec::new(),
            }
        }
        Err(_) => Vec::new(),
    }
}

fn get_artist_photo(artist_id: i64) -> Vec<u8> {
    // Get artist photo
    match get_or_init_pool() {
        Ok(pool) => {
            let conn = match pool.get() {
                Ok(conn) => conn,
                Err(_) => return Vec::new(),
            };

            // Query artist photo path
            let result: Result<Option<String>, _> = conn.query_row(
                "SELECT photo_path FROM artists WHERE id = ?1",
                [artist_id],
                |row| row.get(0),
            );

            match result {
                Ok(Some(photo_path)) => {
                    if let Ok(bytes) = std::fs::read(&photo_path) {
                        return bytes;
                    }
                    Vec::new()
                }
                _ => Vec::new(),
            }
        }
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TestEnv;
    use crate::services::{LibraryService, PlaylistService};

    const MINIMAL_FLAC: &[u8] =
        include_bytes!("../tests/fixtures/minimal.flac");

    /// Given a base name and a set of names already in use, return the first
    /// available name: `base`, then `base-2`, `base-3`, etc.
    fn unique_playlist_name(base: &str, existing: &std::collections::HashSet<String>) -> String {
        if !existing.contains(base) {
            return base.to_string();
        }
        let mut suffix = 2usize;
        loop {
            let candidate = format!("{}-{}", base, suffix);
            if !existing.contains(&candidate) {
                return candidate;
            }
            suffix += 1;
        }
    }

    // ── unique_playlist_name ─────────────────────────────────────────────────

    #[test]
    fn test_unique_name_no_collision() {
        let existing: std::collections::HashSet<String> = std::collections::HashSet::new();
        assert_eq!(unique_playlist_name("import-2026-02-17", &existing), "import-2026-02-17");
    }

    #[test]
    fn test_unique_name_first_collision() {
        let existing: std::collections::HashSet<String> =
            ["import-2026-02-17".to_string()].into_iter().collect();
        assert_eq!(unique_playlist_name("import-2026-02-17", &existing), "import-2026-02-17-2");
    }

    #[test]
    fn test_unique_name_multiple_collisions() {
        let existing: std::collections::HashSet<String> = [
            "import-2026-02-17".to_string(),
            "import-2026-02-17-2".to_string(),
            "import-2026-02-17-3".to_string(),
        ].into_iter().collect();
        assert_eq!(unique_playlist_name("import-2026-02-17", &existing), "import-2026-02-17-4");
    }

    #[test]
    fn test_unique_name_gap_in_suffixes() {
        // -2 exists but -3 does not: should return -2 wait, no — it tries 2 first
        let existing: std::collections::HashSet<String> = [
            "import-2026-02-17".to_string(),
            "import-2026-02-17-3".to_string(),
        ].into_iter().collect();
        // -2 is free, so we get -2 (gap is irrelevant)
        assert_eq!(unique_playlist_name("import-2026-02-17", &existing), "import-2026-02-17-2");
    }

    // ── import_files JSON parsing ────────────────────────────────────────────

    #[test]
    fn test_import_files_invalid_json_returns_error() {
        let result = import_files("not-json");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["success"], false);
        assert!(v["error"].as_str().unwrap().contains("Failed to parse"));
    }

    #[test]
    fn test_import_files_empty_array_returns_zero_tracks() {
        let result = import_files("[]");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["success"], true);
        assert_eq!(v["data"]["tracks_imported"], 0);
        assert!(v["data"]["playlist"].is_null());
    }

    #[test]
    fn test_import_files_nonexistent_paths_returns_zero() {
        let paths = serde_json::json!(["/no/such/file.flac"]).to_string();
        let result = import_files(&paths);
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["success"], true);
        assert_eq!(v["data"]["tracks_imported"], 0);
        assert!(v["data"]["playlist"].is_null());
    }

    /// Helper: run the import business logic against a TestEnv's pool (bypasses global singleton).
    fn import_with_env(env: &TestEnv, file_paths: &[std::path::PathBuf]) -> (usize, Option<(i64, String)>) {
        let library_service = LibraryService::new(env.pool.clone(), env.app_paths.clone());
        let playlist_service = PlaylistService::new(env.pool.clone());

        let track_ids = library_service.import_paths(file_paths).expect("import_paths failed");
        if track_ids.is_empty() {
            return (0, None);
        }
        let tracks_imported = track_ids.len();

        let base_name = chrono::Local::now().format("import-%Y-%m-%d").to_string();
        let existing_names: std::collections::HashSet<String> = playlist_service
            .list_playlists()
            .unwrap_or_default()
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let playlist_name = unique_playlist_name(&base_name, &existing_names);

        let playlist = playlist_service.create_playlist(&playlist_name, None).expect("create_playlist failed");
        playlist_service.add_tracks(playlist.id, track_ids).expect("add_tracks failed");

        (tracks_imported, Some((playlist.id, playlist.name)))
    }

    #[test]
    fn test_import_files_valid_flac_creates_playlist() {
        let env = TestEnv::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let flac = tmp.path().join("song.flac");
        std::fs::write(&flac, MINIMAL_FLAC).unwrap();

        let (count, playlist) = import_with_env(&env, &[flac]);

        assert_eq!(count, 1, "should import exactly 1 track");
        let (id, name) = playlist.expect("playlist should be created");
        assert!(id > 0, "playlist id must be positive");
        assert!(name.starts_with("import-"), "playlist name must start with 'import-', got: {}", name);
    }

    // ── get_tracks_page — sorting ────────────────────────────────────────────

    /// Runs the same ORDER BY logic as `get_tracks_page` but against a given pool,
    /// returning the ordered list of track IDs.
    fn query_track_ids_sorted(pool: &crate::db::DbPool, sort_by: &str, sort_dir: &str) -> Vec<i64> {
        let conn = pool.get().unwrap();

        let order_col = match sort_by {
            "artist"       => "ar.name",
            "album"        => "al.title",
            "duration"     => "t.duration",
            "track_number" => "t.disc_number * 1000 + COALESCE(t.track_number, 0)",
            "file_size"    => "t.file_size",
            "play_count"   => "t.play_count",
            "rating"       => "t.rating",
            "file_path"    => "t.file_path",
            _              => "t.title",
        };
        let order_dir = if sort_dir.eq_ignore_ascii_case("desc") { "DESC" } else { "ASC" };

        let query = format!(
            "SELECT t.id FROM tracks t \
             LEFT JOIN artists ar ON t.artist_id = ar.id \
             LEFT JOIN albums al ON t.album_id = al.id \
             ORDER BY {} {}",
            order_col, order_dir
        );

        let mut stmt = conn.prepare(&query).unwrap();
        stmt.query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn test_get_tracks_page_sort_by_duration_asc() {
        let env = TestEnv::new();
        let (_, _, _, _, track1_id, track2_id) = env.seed_basic_library();
        // track1: duration=240_000ms (longer), track2: duration=180_000ms (shorter)
        let ids = query_track_ids_sorted(&env.pool, "duration", "asc");
        assert_eq!(ids, vec![track2_id, track1_id], "shorter duration must come first in ASC order");
    }

    #[test]
    fn test_get_tracks_page_sort_by_duration_desc() {
        let env = TestEnv::new();
        let (_, _, _, _, track1_id, track2_id) = env.seed_basic_library();
        let ids = query_track_ids_sorted(&env.pool, "duration", "desc");
        assert_eq!(ids, vec![track1_id, track2_id], "longer duration must come first in DESC order");
    }

    #[test]
    fn test_get_tracks_page_sort_dir_case_insensitive() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let ids_upper = query_track_ids_sorted(&env.pool, "duration", "DESC");
        let ids_lower = query_track_ids_sorted(&env.pool, "duration", "desc");
        assert_eq!(ids_upper, ids_lower, "sort_dir must be case-insensitive");
    }

    #[test]
    fn test_get_tracks_page_unknown_sort_by_falls_back_to_title() {
        let env = TestEnv::new();
        env.seed_basic_library();
        // "Track One" < "Track Two" alphabetically → track1 comes first ASC
        let ids_unknown = query_track_ids_sorted(&env.pool, "unknown_field", "asc");
        let ids_title   = query_track_ids_sorted(&env.pool, "title", "asc");
        // Both should use t.title order (which is not a named sort_by key, so also falls to default)
        // We just verify the fallback doesn't crash and returns both tracks
        assert_eq!(ids_unknown.len(), 2);
        assert_eq!(ids_unknown, ids_title);
    }

    #[test]
    fn test_get_tracks_page_sort_by_file_size_asc() {
        let env = TestEnv::new();
        let (_, _, _, _, track1_id, track2_id) = env.seed_basic_library();
        // track1: file_size=30_000_000, track2: file_size=25_000_000
        let ids = query_track_ids_sorted(&env.pool, "file_size", "asc");
        assert_eq!(ids, vec![track2_id, track1_id], "smaller file_size must come first in ASC order");
    }

    #[test]
    fn test_get_tracks_page_empty_library_returns_empty() {
        let env = TestEnv::new();
        // No tracks seeded
        let ids = query_track_ids_sorted(&env.pool, "duration", "asc");
        assert!(ids.is_empty());
    }

    // ── get_library_stats ────────────────────────────────────────────────

    /// Runs the same stats queries as `get_library_stats` against a provided pool.
    fn query_library_stats(pool: &crate::db::DbPool) -> serde_json::Value {
        let conn = pool.get().unwrap();
        let albums:   i64 = conn.query_row("SELECT COUNT(*) FROM albums",  [], |r| r.get(0)).unwrap();
        let artists:  i64 = conn.query_row("SELECT COUNT(*) FROM artists", [], |r| r.get(0)).unwrap();
        let tracks:   i64 = conn.query_row("SELECT COUNT(*) FROM tracks",  [], |r| r.get(0)).unwrap();
        let duration: i64 = conn.query_row(
            "SELECT COALESCE(SUM(duration), 0) / 1000 FROM tracks", [], |r| r.get(0),
        ).unwrap();
        let artworks: i64 = conn.query_row(
            "SELECT COUNT(*) FROM albums WHERE online_artwork_path IS NOT NULL", [], |r| r.get(0),
        ).unwrap();
        serde_json::json!({
            "success": true,
            "data": {
                "album_count":            albums,
                "artist_count":           artists,
                "track_count":            tracks,
                "total_duration_seconds": duration,
                "artwork_count":          artworks,
            }
        })
    }

    #[test]
    fn test_library_stats_empty_db_returns_all_zeros() {
        let env = TestEnv::new();
        let v = query_library_stats(&env.pool);
        assert_eq!(v["data"]["album_count"],   0);
        assert_eq!(v["data"]["artist_count"],  0);
        assert_eq!(v["data"]["track_count"],   0);
        assert_eq!(v["data"]["artwork_count"], 0);
    }

    #[test]
    fn test_library_stats_total_duration_is_zero_not_null_on_empty_db() {
        // COALESCE(SUM(duration), 0) must return integer 0, not JSON null.
        // A regression would cause JSON serialisation to fail or produce null.
        let env = TestEnv::new();
        let v = query_library_stats(&env.pool);
        assert!(!v["data"]["total_duration_seconds"].is_null());
        assert_eq!(v["data"]["total_duration_seconds"], 0);
    }

    #[test]
    fn test_library_stats_counts_match_seeded_data() {
        let env = TestEnv::new();
        env.seed_basic_library(); // 1 artist, 1 album, 2 tracks (240s + 180s = 420s)
        let v = query_library_stats(&env.pool);
        assert_eq!(v["data"]["album_count"],            1);
        assert_eq!(v["data"]["artist_count"],           1);
        assert_eq!(v["data"]["track_count"],            2);
        assert_eq!(v["data"]["total_duration_seconds"], 420);
    }

    // ── delete_tracks ────────────────────────────────────────────────────

    #[test]
    fn test_delete_tracks_invalid_json_returns_error() {
        let result = delete_tracks("not-json");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["success"], false);
        assert!(v["error"].as_str().unwrap().contains("Invalid track IDs JSON"));
    }

    #[test]
    fn test_delete_tracks_wrong_json_type_returns_error() {
        let result = delete_tracks("{\"id\": 1}");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["success"], false);
    }

    #[test]
    fn test_delete_tracks_empty_array_returns_zero_deleted() {
        let result = delete_tracks("[]");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["success"], true);
        assert_eq!(v["data"]["deleted"], 0);
    }

    /// Helper: run the delete-tracks logic against a TestEnv pool (bypasses global singleton).
    fn delete_tracks_with_pool(pool: &crate::db::DbPool, ids: &[i64]) -> (bool, usize) {
        let conn = pool.get().unwrap();
        let mut deleted = 0usize;
        for &id in ids {
            match crate::db::queries::delete_track(&conn, id) {
                Ok(_)  => deleted += 1,
                Err(_) => return (false, deleted),
            }
        }
        (true, deleted)
    }

    #[test]
    fn test_delete_tracks_nonexistent_id_counted_as_deleted() {
        // SQLite DELETE never errors on a missing row (returns Ok(0) rows affected).
        // The loop in delete_tracks counts every Ok(_) as a deletion, so a nonexistent
        // ID still increments the counter.  This is the current behaviour; a future
        // fix might check rows_affected instead.
        let env = TestEnv::new();
        let (success, deleted) = delete_tracks_with_pool(&env.pool, &[99999999]);
        assert!(success, "deleting a nonexistent ID must not return an error");
        assert_eq!(deleted, 1,
            "current behaviour: nonexistent delete is counted (SQLite returns Ok with 0 rows affected)");
    }

    #[test]
    fn test_delete_tracks_existing_track_is_removed() {
        let env = TestEnv::new();
        let (_, _, _, _, track_id, _) = env.seed_basic_library();
        let conn = env.pool.get().unwrap();

        let (success, deleted) = delete_tracks_with_pool(&env.pool, &[track_id]);
        assert!(success);
        assert_eq!(deleted, 1);

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tracks WHERE id = ?1", [track_id], |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 0, "track must be absent from the database after deletion");
    }

    // ── get_tracks_page — pagination edge cases ──────────────────────────

    fn query_track_ids_paged(pool: &crate::db::DbPool, limit: u32, offset: u32) -> Vec<i64> {
        let conn = pool.get().unwrap();
        let limit_capped = std::cmp::min(limit, 100) as usize;
        let query = format!(
            "SELECT id FROM tracks ORDER BY title LIMIT {} OFFSET {}",
            limit_capped, offset
        );
        let mut stmt = conn.prepare(&query).unwrap();
        stmt.query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn test_get_tracks_page_offset_beyond_total_returns_empty() {
        let env = TestEnv::new();
        env.seed_basic_library(); // 2 tracks
        let ids = query_track_ids_paged(&env.pool, 10, 999);
        assert!(ids.is_empty(), "offset beyond total must return an empty list");
    }

    #[test]
    fn test_get_tracks_page_limit_zero_returns_empty() {
        let env = TestEnv::new();
        env.seed_basic_library();
        let ids = query_track_ids_paged(&env.pool, 0, 0);
        assert!(ids.is_empty(), "limit=0 must return an empty list");
    }

    #[test]
    fn test_get_tracks_page_all_named_sort_fields_do_not_crash() {
        let env = TestEnv::new();
        env.seed_basic_library();
        for sort_by in &["artist", "album", "duration", "track_number",
                         "file_size", "play_count", "rating", "file_path", "unknown_field"] {
            let ids = query_track_ids_sorted(&env.pool, sort_by, "asc");
            assert_eq!(ids.len(), 2, "sort_by='{}' must return all seeded tracks", sort_by);
        }
    }

    #[test]
    fn test_import_files_second_import_same_day_gets_suffix() {
        let env = TestEnv::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let flac1 = tmp.path().join("a.flac");
        let flac2 = tmp.path().join("b.flac");
        std::fs::write(&flac1, MINIMAL_FLAC).unwrap();
        std::fs::write(&flac2, MINIMAL_FLAC).unwrap();

        let (_, p1) = import_with_env(&env, &[flac1]);
        let (_, p2) = import_with_env(&env, &[flac2]);

        let name1 = p1.expect("first import should create a playlist").1;
        let name2 = p2.expect("second import should create a playlist").1;

        assert_ne!(name1, name2, "two imports on the same day must get distinct names");
        assert!(name2.ends_with("-2"), "second import should get '-2' suffix, got: {}", name2);
    }
}
