//! Minimal example: scan a music directory and play the first track.
//!
//! ```
//! cargo run --example scan_and_play -- /path/to/music
//! ```

use std::path::Path;
use tornade_core::db;
use tornade_core::services::{LibraryService, PlayerService};
use tornade_core::utils::AppPaths;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let music_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| String::from("/tmp/music"));

    // 1. Initialise application paths and database
    let paths = AppPaths::new()?;
    let pool = db::create_pool(paths.database_path())?;
    db::initialize_database(&pool)?;

    // 2. Register the directory as a library source and scan it
    let library = LibraryService::new(pool.clone(), paths);
    let source = library.add_source("Example Library", Path::new(&music_dir))?;
    println!("Scanning {} …", music_dir);
    let result = library.scan_directory(Path::new(&music_dir), source.id)?;
    println!(
        "Scan complete: {} added, {} skipped",
        result.tracks_added, result.tracks_skipped
    );

    // 3. Retrieve the first track and play it
    let tracks = library.get_album_tracks(1).unwrap_or_default();
    if let Some(track) = tracks.first() {
        println!("Playing: {} — {}", track.artist_id, track.title);
        let player = PlayerService::new(pool)?;
        player.play(track.id)?;
        std::thread::sleep(std::time::Duration::from_secs(5));
        player.stop()?;
    } else {
        println!("No tracks found in {music_dir}");
    }

    Ok(())
}
