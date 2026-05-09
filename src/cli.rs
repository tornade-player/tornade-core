// Interactive CLI for Tornade Music Player

use crate::db::{self, DbPool};
use crate::services::{
    ArtworkService, DuplicateService, LibraryService, PlayerService, PlaylistService,
};
use crate::utils::AppPaths;
use std::io::{self, Write};
use std::path::PathBuf;

pub struct TornadeCli {
    library: LibraryService,
    player: PlayerService,
    playlist: PlaylistService,
    duplicate: DuplicateService,
    artwork: ArtworkService,
    pool: DbPool,
}

impl TornadeCli {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let app_paths = AppPaths::new()?;
        let pool = db::create_pool(app_paths.database_path())?;
        db::initialize_database(&pool)?;

        let library = LibraryService::new(pool.clone(), app_paths.clone());
        let player = PlayerService::new(pool.clone())?;
        let playlist = PlaylistService::new(pool.clone());
        let duplicate = DuplicateService::new(pool.clone());
        let artwork = ArtworkService::new(pool.clone(), app_paths);

        Ok(TornadeCli {
            library,
            player,
            playlist,
            duplicate,
            artwork,
            pool,
        })
    }

    pub fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🎵 Tornade Music Player - Interactive CLI");
        println!("=========================================\n");

        loop {
            Self::print_menu();

            let choice = Self::read_input("Enter your choice: ")?;

            match choice.trim() {
                "1" => self.add_source()?,
                "2" => self.scan_library()?,
                "3" => self.list_sources()?,
                "4" => self.browse_tracks()?,
                "5" => self.browse_albums()?,
                "6" => self.browse_artists()?,
                "7" => self.browse_genres()?,
                "8" => self.search_library()?,
                "9" => self.play_track()?,
                "10" => self.playback_controls()?,
                "11" => self.queue_management()?,
                "12" => self.playlist_management()?,
                "13" => self.show_stats()?,
                "14" => self.find_duplicates()?,
                "15" => self.fetch_artwork()?,
                "16" => self.reset_library()?,
                "17" => {
                    println!("\n👋 Goodbye!");
                    break;
                }
                _ => println!("❌ Invalid choice, please try again."),
            }

            println!();
        }

        Ok(())
    }

    fn print_menu() {
        println!("Main Menu:");
        println!("  1. Add music source");
        println!("  2. Scan library");
        println!("  3. List sources");
        println!("  4. Browse tracks");
        println!("  5. Browse albums");
        println!("  6. Browse artists");
        println!("  7. Browse genres");
        println!("  8. Search library");
        println!("  9. Play track");
        println!(" 10. Playback controls");
        println!(" 11. Queue management");
        println!(" 12. Playlist management");
        println!(" 13. Show statistics");
        println!(" 14. Find duplicates");
        println!(" 15. Fetch artwork from online sources");
        println!(" 16. Reset library (⚠️  deletes all data)");
        println!(" 17. Quit");
        println!();
    }

    fn add_source(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n📁 Add Music Source");
        println!("==================");

        let name = Self::read_input("Source name: ")?;
        let path = Self::read_input("Path to music folder: ")?;

        let path = PathBuf::from(path.trim());

        if !path.exists() {
            println!("❌ Path does not exist: {path:?}");
            return Ok(());
        }

        match self.library.add_source(&name, &path) {
            Ok(source) => {
                println!("✅ Source added successfully!");
                println!("   ID: {}", source.id);
                println!("   Name: {}", source.name);
                println!("   Path: {:?}", source.path);
            }
            Err(e) => println!("❌ Failed to add source: {e}"),
        }

        Ok(())
    }

    fn scan_library(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🔍 Scan Library");
        println!("===============");

        // List sources
        let sources = self.library.list_sources()?;
        if sources.is_empty() {
            println!("❌ No sources configured. Please add a source first.");
            return Ok(());
        }

        println!("Available sources:");
        for source in &sources {
            println!("  {}. {} - {:?}", source.id, source.name, source.path);
        }

        let source_id_str = Self::read_input("\nEnter source ID to scan: ")?;
        let source_id: i64 = source_id_str.trim().parse()?;

        // Find source
        let source = sources.iter().find(|s| s.id == source_id);
        if source.is_none() {
            println!("❌ Source not found");
            return Ok(());
        }

        let source = source.unwrap();
        if source.path.is_none() {
            println!("❌ Source has no path");
            return Ok(());
        }

        println!("\n⏳ Scanning directory: {:?}", source.path);
        println!("This may take a while...\n");

        match self
            .library
            .scan_directory(source.path.as_ref().unwrap(), source_id)
        {
            Ok(result) => {
                println!("✅ Scan complete!");
                println!("   Tracks added: {}", result.tracks_added);
                println!("   Tracks updated: {}", result.tracks_updated);
                println!("   Tracks skipped: {}", result.tracks_skipped);
                println!("   Duration: {:?}", result.duration);

                if !result.errors.is_empty() {
                    println!("\n⚠️  Errors encountered: {}", result.errors.len());
                    println!("   (Use verbose mode to see details)");
                }
            }
            Err(e) => println!("❌ Scan failed: {e}"),
        }

        Ok(())
    }

    fn list_sources(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n📚 Music Sources");
        println!("================");

        let sources = self.library.list_sources()?;

        if sources.is_empty() {
            println!("No sources configured yet.");
            return Ok(());
        }

        for source in &sources {
            println!("\nSource ID: {}", source.id);
            println!("  Name: {}", source.name);
            println!("  Type: {:?}", source.source_type);
            println!("  Path: {:?}", source.path.clone().unwrap_or_default());
            if let Some(ref scanned) = source.last_scanned_at {
                println!("  Last scanned: {scanned}");
            }
        }

        // Option to filter by source
        let choice =
            Self::read_input("\nEnter source ID to view tracks (or press Enter to go back): ")?;
        if !choice.trim().is_empty() {
            let source_id: i64 = choice.trim().parse()?;
            self.show_source_tracks(source_id)?;
        }

        Ok(())
    }

    fn browse_tracks(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🎵 Browse Tracks");
        println!("================");

        // Get track count
        let conn = self.pool.get()?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))?;

        println!("Total tracks in library: {count}\n");

        if count == 0 {
            println!("No tracks in library. Please scan a source first.");
            return Ok(());
        }

        // Show recent tracks
        let mut stmt = conn.prepare(
            "SELECT t.id, t.title, a.name as artist, al.title as album
             FROM tracks t
             JOIN artists a ON a.id = t.artist_id
             LEFT JOIN albums al ON al.id = t.album_id
             ORDER BY t.id DESC
             LIMIT 20",
        )?;

        let tracks = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;

        println!("Recent tracks:");
        println!(
            "{:<6} {:<40} {:<30} {:<30}",
            "ID", "Title", "Artist", "Album"
        );
        println!("{}", "-".repeat(110));

        for track in tracks {
            let (id, title, artist, album) = track?;
            let album_str = album.unwrap_or_else(|| "Unknown".to_string());
            println!(
                "{:<6} {:<40} {:<30} {:<30}",
                id,
                truncate(&title, 40),
                truncate(&artist, 30),
                truncate(&album_str, 30)
            );
        }

        Ok(())
    }

    fn play_track(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n▶️  Play Track");
        println!("=============");

        let track_id_str = Self::read_input("Enter track ID to play: ")?;
        let track_id: i64 = track_id_str.trim().parse()?;

        match self.library.get_track(track_id)? {
            Some(track) => {
                println!("\n🎵 Now playing:");
                println!("   Title: {}", track.title);
                println!("   Artist: (ID {})", track.artist_id);
                println!("   Duration: {:?}", track.duration);
                println!("   Format: {:?}", track.file_type);
                if let Some(sr) = track.sample_rate {
                    println!("   Sample rate: {sr} Hz");
                }

                self.player.set_queue(vec![track_id])?;
                self.player.play(track_id)?;

                println!("\n✅ Playback started!");
            }
            None => println!("❌ Track not found"),
        }

        Ok(())
    }

    fn playback_controls(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🎛️  Playback Controls");
        println!("===================");

        let state = self.player.get_state();
        println!("Current state: {state:?}");

        if let Some(track) = self.player.get_current_track() {
            println!("Current track: {} (ID: {})", track.title, track.id);
        }

        println!("\nControls:");
        println!("  1. Pause");
        println!("  2. Resume");
        println!("  3. Stop");
        println!("  4. Next");
        println!("  5. Previous");
        println!("  6. Set volume");
        println!("  7. Toggle shuffle");
        println!("  8. Set repeat mode");
        println!("  9. Back");

        let choice = Self::read_input("\nEnter choice: ")?;

        match choice.trim() {
            "1" => {
                self.player.pause()?;
                println!("⏸️  Paused");
            }
            "2" => {
                self.player.resume()?;
                println!("▶️  Resumed");
            }
            "3" => {
                self.player.stop()?;
                println!("⏹️  Stopped");
            }
            "4" => {
                self.player.next()?;
                println!("⏭️  Next track");
            }
            "5" => {
                self.player.previous()?;
                println!("⏮️  Previous track");
            }
            "6" => {
                let vol_str = Self::read_input("Enter volume (0.0 - 1.0): ")?;
                let vol: f32 = vol_str.trim().parse()?;
                self.player.set_volume(vol)?;
                println!("🔊 Volume set to {vol}");
            }
            "7" => {
                let enabled = !self.player.is_shuffle_enabled();
                self.player.set_shuffle(enabled)?;
                println!("🔀 Shuffle: {}", if enabled { "ON" } else { "OFF" });
            }
            "8" => {
                println!("Repeat modes: 0=Off, 1=All, 2=One");
                let mode_str = Self::read_input("Enter mode: ")?;
                let mode = match mode_str.trim() {
                    "0" => crate::models::RepeatMode::Off,
                    "1" => crate::models::RepeatMode::All,
                    "2" => crate::models::RepeatMode::One,
                    _ => {
                        println!("Invalid mode");
                        return Ok(());
                    }
                };
                self.player.set_repeat(mode)?;
                println!("🔁 Repeat mode: {mode:?}");
            }
            _ => {}
        }

        Ok(())
    }

    fn queue_management(&self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            println!("\n📋 Queue Management");
            println!("==================");

            let queue = self.player.get_queue();
            let current_idx = self.player.get_queue_index();

            if queue.is_empty() {
                println!("Queue is empty.");
            } else {
                println!("Tracks in queue: {}\n", queue.len());

                // Show queue with track details
                let conn = self.pool.get()?;
                println!(
                    "{:<4} {:<6} {:<40} {:<25}",
                    "Pos", "ID", "Title", "Artist ID"
                );
                println!("{}", "-".repeat(78));

                for (idx, track_id) in queue.iter().enumerate() {
                    let marker = if idx == current_idx { "▶" } else { " " };

                    // Try to get track details
                    if let Ok(Some(track)) = crate::db::queries::get_track(&conn, *track_id) {
                        println!(
                            "{} {:<2} {:<6} {:<40} {:<25}",
                            marker,
                            idx + 1,
                            track.id,
                            truncate(&track.title, 40),
                            track.artist_id
                        );
                    } else {
                        println!(
                            "{} {:<2} {:<6} {:<40} {:<25}",
                            marker,
                            idx + 1,
                            track_id,
                            "(track not found)",
                            "-"
                        );
                    }
                }

                println!("\n▶ = Currently playing");
            }

            // Shuffle and repeat status
            let shuffle = self.player.is_shuffle_enabled();
            let repeat = self.player.get_repeat_mode();
            println!(
                "\n🔀 Shuffle: {} | 🔁 Repeat: {:?}",
                if shuffle { "ON" } else { "OFF" },
                repeat
            );

            println!("\nOptions:");
            println!("  1. Add track to queue");
            println!("  2. Add album to queue");
            println!("  3. Remove track from queue");
            println!("  4. Move track in queue");
            println!("  5. Clear queue");
            println!("  6. Search queue");
            println!("  7. Play track from queue");
            println!("  8. Back");

            let choice = Self::read_input("\nEnter choice: ")?;

            match choice.trim() {
                "1" => {
                    let track_id_str = Self::read_input("Enter track ID: ")?;
                    let track_id: i64 = track_id_str.trim().parse()?;
                    self.player.add_to_queue(vec![track_id])?;
                    println!("✅ Track added to queue");
                }
                "2" => {
                    let album_id_str = Self::read_input("Enter album ID: ")?;
                    let album_id: i64 = album_id_str.trim().parse()?;
                    let tracks = self.library.get_album_tracks(album_id)?;
                    let track_ids: Vec<i64> = tracks.iter().map(|t| t.id).collect();
                    self.player.add_to_queue(track_ids.clone())?;
                    println!("✅ Album added to queue ({} tracks)", track_ids.len());
                }
                "3" => {
                    if queue.is_empty() {
                        println!("❌ Queue is empty");
                        continue;
                    }
                    let prompt = format!("Enter position to remove (1-{}): ", queue.len());
                    let pos_str = Self::read_input(&prompt)?;
                    let pos: usize = pos_str.trim().parse()?;
                    if pos > 0 && pos <= queue.len() {
                        self.player.remove_from_queue(pos - 1)?;
                        println!("✅ Track removed from queue");
                    } else {
                        println!("❌ Invalid position");
                    }
                }
                "4" => {
                    if queue.is_empty() {
                        println!("❌ Queue is empty");
                        continue;
                    }
                    let prompt_from = format!("Move from position (1-{}): ", queue.len());
                    let from_str = Self::read_input(&prompt_from)?;
                    let prompt_to = format!("Move to position (1-{}): ", queue.len());
                    let to_str = Self::read_input(&prompt_to)?;
                    let from: usize = from_str.trim().parse()?;
                    let to: usize = to_str.trim().parse()?;
                    if from > 0 && from <= queue.len() && to > 0 && to <= queue.len() {
                        self.player.move_in_queue(from - 1, to - 1)?;
                        println!("✅ Track moved in queue");
                    } else {
                        println!("❌ Invalid positions");
                    }
                }
                "5" => {
                    let confirm = Self::read_input("Clear queue? (y/n): ")?;
                    if confirm.trim().to_lowercase() == "y" {
                        self.player.clear_queue()?;
                        println!("✅ Queue cleared");
                    }
                }
                "6" => {
                    self.search_queue(&queue)?;
                }
                "7" => {
                    if queue.is_empty() {
                        println!("❌ Queue is empty");
                        continue;
                    }
                    let prompt = format!("Enter position to play (1-{}): ", queue.len());
                    let pos_str = Self::read_input(&prompt)?;
                    let pos: usize = pos_str.trim().parse()?;
                    if pos > 0 && pos <= queue.len() {
                        self.player.play(queue[pos - 1])?;
                        println!("▶️  Playing track at position {pos}");
                    } else {
                        println!("❌ Invalid position");
                    }
                }
                "8" => break,
                _ => println!("❌ Invalid choice"),
            }
        }

        Ok(())
    }

    fn search_queue(&self, queue: &[i64]) -> Result<(), Box<dyn std::error::Error>> {
        if queue.is_empty() {
            println!("❌ Queue is empty");
            return Ok(());
        }

        let query = Self::read_input("Search query: ")?;
        let query = query.trim().to_lowercase();

        println!("\n🔍 Search Results in Queue:");
        println!(
            "{:<4} {:<6} {:<40} {:<25}",
            "Pos", "ID", "Title", "Artist ID"
        );
        println!("{}", "-".repeat(78));

        let conn = self.pool.get()?;
        let mut found = 0;

        for (idx, track_id) in queue.iter().enumerate() {
            if let Ok(Some(track)) = crate::db::queries::get_track(&conn, *track_id)
                && track.title.to_lowercase().contains(&query)
            {
                println!(
                    "{:<4} {:<6} {:<40} {:<25}",
                    idx + 1,
                    track.id,
                    truncate(&track.title, 40),
                    track.artist_id
                );
                found += 1;
            }
        }

        if found == 0 {
            println!("No matches found");
        } else {
            println!("\nFound {found} match(es)");
        }

        Ok(())
    }

    fn show_stats(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n📊 Library Statistics");
        println!("====================");

        let conn = self.pool.get()?;

        let track_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))?;

        let album_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM albums", [], |row| row.get(0))?;

        let artist_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM artists", [], |row| row.get(0))?;

        let genre_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM genres", [], |row| row.get(0))?;

        let source_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM sources", [], |row| row.get(0))?;

        println!("  Tracks:  {track_count}");
        println!("  Albums:  {album_count}");
        println!("  Artists: {artist_count}");
        println!("  Genres:  {genre_count}");
        println!("  Sources: {source_count}");

        // Top rated tracks
        if track_count > 0 {
            println!("\n🌟 Top Rated Tracks:");
            let mut stmt = conn.prepare(
                "SELECT t.title, a.name, t.rating
                 FROM tracks t
                 JOIN artists a ON a.id = t.artist_id
                 WHERE t.rating > 0
                 ORDER BY t.rating DESC, t.title
                 LIMIT 5",
            )?;

            let tracks = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u8>(2)?,
                ))
            })?;

            for track in tracks {
                let (title, artist, rating) = track?;
                let stars = "⭐".repeat(rating as usize);
                println!("  {title} - {artist} {stars}");
            }
        }

        Ok(())
    }

    fn read_input(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
        print!("{prompt}");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        Ok(input)
    }

    fn browse_albums(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n📀 Browse Albums");
        println!("================");

        let albums = self.library.list_albums(None, None, None, Some(50), None)?;

        if albums.is_empty() {
            println!("No albums in library.");
            return Ok(());
        }

        println!(
            "\n{:<6} {:<40} {:<30} {:<6} {:<8}",
            "ID", "Album", "Artist ID", "Year", "Rating"
        );
        println!("{}", "-".repeat(95));

        for album in &albums {
            let year_str = album
                .year
                .map_or_else(|| "-".to_string(), |y| y.to_string());
            let rating_str = if album.rating.0 > 0 {
                "⭐".repeat(album.rating.0 as usize)
            } else {
                "-".to_string()
            };

            println!(
                "{:<6} {:<40} {:<30} {:<6} {}",
                album.id,
                truncate(&album.title, 40),
                album.artist_id,
                year_str,
                rating_str
            );
        }

        println!("\nTotal: {} albums", albums.len());

        // Option to view album details
        let choice =
            Self::read_input("\nEnter album ID for details (or press Enter to go back): ")?;
        if !choice.trim().is_empty() {
            let album_id: i64 = choice.trim().parse()?;
            self.show_album_details(album_id)?;
        }

        Ok(())
    }

    fn show_album_details(&self, album_id: i64) -> Result<(), Box<dyn std::error::Error>> {
        match self.library.get_album(album_id)? {
            Some(album) => {
                println!("\n📀 Album Details");
                println!("===============");
                println!("Title: {}", album.title);
                println!("Artist ID: {}", album.artist_id);
                if let Some(year) = album.year {
                    println!("Year: {year}");
                }
                if album.rating.0 > 0 {
                    println!("Rating: {}", "⭐".repeat(album.rating.0 as usize));
                }

                // Get tracks
                let tracks = self.library.get_album_tracks(album_id)?;
                println!("\nTracks ({}):", tracks.len());
                println!("{:<4} {:<40} {:>10}", "#", "Title", "Duration");
                println!("{}", "-".repeat(58));

                for track in tracks {
                    let track_num = track
                        .track_number
                        .map_or_else(|| "--".to_string(), |n| format!("{n:2}"));
                    let duration_secs = track.duration.as_secs();
                    let duration_str = format!("{}:{:02}", duration_secs / 60, duration_secs % 60);

                    println!(
                        "{:<4} {:<40} {:>10}",
                        track_num,
                        truncate(&track.title, 40),
                        duration_str
                    );
                }

                // Options
                println!("\nOptions:");
                println!("  1. Play album");
                println!("  2. Rate album");
                println!("  3. Back");

                let choice = Self::read_input("\nEnter choice: ")?;
                match choice.trim() {
                    "1" => {
                        let tracks = self.library.get_album_tracks(album_id)?;
                        let track_ids: Vec<i64> = tracks.iter().map(|t| t.id).collect();
                        if !track_ids.is_empty() {
                            self.player.set_queue(track_ids.clone())?;
                            self.player.play(track_ids[0])?;
                            println!("▶️  Playing album");
                        }
                    }
                    "2" => {
                        let rating_str = Self::read_input("Enter rating (0-5): ")?;
                        let rating: u8 = rating_str.trim().parse()?;
                        self.library.rate_album(album_id, rating)?;
                        println!("✅ Album rated: {}", "⭐".repeat(rating as usize));
                    }
                    _ => {}
                }
            }
            None => println!("❌ Album not found"),
        }

        Ok(())
    }

    fn browse_artists(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🎤 Browse Artists");
        println!("=================");

        let artists = self.library.list_artists()?;

        if artists.is_empty() {
            println!("No artists in library.");
            return Ok(());
        }

        println!("\n{:<6} {:<50}", "ID", "Artist");
        println!("{}", "-".repeat(58));

        for artist in &artists {
            println!("{:<6} {:<50}", artist.id, truncate(&artist.name, 50));
        }

        println!("\nTotal: {} artists", artists.len());

        // Option to view artist details
        let choice =
            Self::read_input("\nEnter artist ID for details (or press Enter to go back): ")?;
        if !choice.trim().is_empty() {
            let artist_id: i64 = choice.trim().parse()?;
            self.show_artist_details(artist_id)?;
        }

        Ok(())
    }

    fn show_artist_details(&self, artist_id: i64) -> Result<(), Box<dyn std::error::Error>> {
        match self.library.get_artist(artist_id)? {
            Some(artist) => {
                println!("\n🎤 Artist Details");
                println!("=================");
                println!("Name: {}", artist.name);

                // Get albums
                let albums = self.library.get_artist_albums(artist_id)?;
                println!("\nAlbums ({}):", albums.len());
                println!("{:<6} {:<45} {:<6} {:<8}", "ID", "Title", "Year", "Rating");
                println!("{}", "-".repeat(70));

                for album in albums {
                    let year_str = album
                        .year
                        .map_or_else(|| "-".to_string(), |y| y.to_string());
                    let rating_str = if album.rating.0 > 0 {
                        "⭐".repeat(album.rating.0 as usize)
                    } else {
                        "-".to_string()
                    };

                    println!(
                        "{:<6} {:<45} {:<6} {}",
                        album.id,
                        truncate(&album.title, 45),
                        year_str,
                        rating_str
                    );
                }
            }
            None => println!("❌ Artist not found"),
        }

        Ok(())
    }

    fn browse_genres(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🎸 Browse Genres");
        println!("================");

        let genres = self.library.list_genres()?;

        if genres.is_empty() {
            println!("No genres in library.");
            return Ok(());
        }

        println!(
            "\n{:<6} {:<35} {:>10} {:>10}",
            "ID", "Genre", "Tracks", "Albums"
        );
        println!("{}", "-".repeat(65));

        for (genre, track_count, album_count) in &genres {
            println!(
                "{:<6} {:<35} {:>10} {:>10}",
                genre.id,
                truncate(&genre.name, 35),
                track_count,
                album_count
            );
        }

        println!("\nTotal: {} genres", genres.len());

        // Option to filter by genre
        let choice = Self::read_input("\nEnter genre ID to filter (or press Enter to go back): ")?;
        if !choice.trim().is_empty() {
            let genre_id: i64 = choice.trim().parse()?;
            self.show_genre_tracks(genre_id)?;
        }

        Ok(())
    }

    fn show_genre_tracks(&self, genre_id: i64) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🎵 Tracks in Genre");
        println!("=================");

        let tracks = self.library.get_genre_tracks(genre_id)?;

        if tracks.is_empty() {
            println!("No tracks found for this genre.");
            return Ok(());
        }

        println!(
            "\n{:<6} {:<40} {:<25} {:>10}",
            "ID", "Title", "Artist ID", "Duration"
        );
        println!("{}", "-".repeat(85));

        for track in &tracks {
            let duration_secs = track.duration.as_secs();
            let duration_str = format!("{}:{:02}", duration_secs / 60, duration_secs % 60);

            println!(
                "{:<6} {:<40} {:<25} {:>10}",
                track.id,
                truncate(&track.title, 40),
                track.artist_id,
                duration_str
            );
        }

        println!("\nTotal: {} tracks", tracks.len());

        // Option to play all
        let choice = Self::read_input("\nPlay all tracks? (y/n): ")?;
        if choice.trim().to_lowercase() == "y" {
            let track_ids: Vec<i64> = tracks.iter().map(|t| t.id).collect();
            self.player.set_queue(track_ids.clone())?;
            if !track_ids.is_empty() {
                self.player.play(track_ids[0])?;
                println!("▶️  Playing genre");
            }
        }

        Ok(())
    }

    fn show_source_tracks(&self, source_id: i64) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🎵 Tracks in Source");
        println!("==================");

        let tracks = self.library.get_source_tracks(source_id)?;

        if tracks.is_empty() {
            println!("No tracks found for this source.");
            return Ok(());
        }

        println!(
            "\n{:<6} {:<40} {:<25} {:>10}",
            "ID", "Title", "Artist ID", "Duration"
        );
        println!("{}", "-".repeat(85));

        for track in &tracks {
            let duration_secs = track.duration.as_secs();
            let duration_str = format!("{}:{:02}", duration_secs / 60, duration_secs % 60);

            println!(
                "{:<6} {:<40} {:<25} {:>10}",
                track.id,
                truncate(&track.title, 40),
                track.artist_id,
                duration_str
            );
        }

        println!("\nTotal: {} tracks", tracks.len());

        Ok(())
    }

    fn search_library(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🔍 Search Library");
        println!("=================");

        let query = Self::read_input("Enter search query: ")?;
        let query = query.trim();

        if query.is_empty() {
            println!("❌ Empty query");
            return Ok(());
        }

        println!("\nSearching for: \"{query}\"...\n");

        let (tracks, albums, artists) = self.library.search(query, 10)?;

        if !tracks.is_empty() {
            println!("🎵 Tracks ({}):", tracks.len());
            println!("{:<6} {:<35} {:<25}", "ID", "Title", "Artist ID");
            println!("{}", "-".repeat(70));
            for track in &tracks {
                println!(
                    "{:<6} {:<35} {:<25}",
                    track.id,
                    truncate(&track.title, 35),
                    track.artist_id
                );
            }
            println!();
        }

        if !albums.is_empty() {
            println!("📀 Albums ({}):", albums.len());
            println!("{:<6} {:<35} {:<25}", "ID", "Title", "Artist ID");
            println!("{}", "-".repeat(70));
            for album in &albums {
                println!(
                    "{:<6} {:<35} {:<25}",
                    album.id,
                    truncate(&album.title, 35),
                    album.artist_id
                );
            }
            println!();
        }

        if !artists.is_empty() {
            println!("🎤 Artists ({}):", artists.len());
            println!("{:<6} {:<50}", "ID", "Name");
            println!("{}", "-".repeat(58));
            for artist in &artists {
                println!("{:<6} {:<50}", artist.id, truncate(&artist.name, 50));
            }
            println!();
        }

        if tracks.is_empty() && albums.is_empty() && artists.is_empty() {
            println!("❌ No results found");
        }

        Ok(())
    }

    fn playlist_management(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🎵 Playlist Management");
        println!("=====================");

        loop {
            println!("\nPlaylist Menu:");
            println!("  1. List all playlists");
            println!("  2. Create new playlist");
            println!("  3. Add tracks to playlist");
            println!("  4. Remove track from playlist");
            println!("  5. Move track in playlist");
            println!("  6. Rename playlist");
            println!("  7. Delete playlist");
            println!("  8. Import M3U playlist");
            println!("  9. Export playlist to M3U");
            println!("  0. Back to main menu");
            println!();

            let choice = Self::read_input("Enter your choice: ")?;

            match choice.trim() {
                "1" => self.list_playlists()?,
                "2" => self.create_playlist()?,
                "3" => self.add_tracks_to_playlist()?,
                "4" => self.remove_track_from_playlist()?,
                "5" => self.move_track_in_playlist()?,
                "6" => self.rename_playlist()?,
                "7" => self.delete_playlist()?,
                "8" => self.import_m3u_playlist()?,
                "9" => self.export_m3u_playlist()?,
                "0" => break,
                _ => println!("❌ Invalid choice, please try again."),
            }

            println!();
        }

        Ok(())
    }

    fn list_playlists(&self) -> Result<(), Box<dyn std::error::Error>> {
        let playlists = self.playlist.list_playlists()?;

        if playlists.is_empty() {
            println!("No playlists found.");
            return Ok(());
        }

        println!(
            "\n{:<6} {:<40} {:>10} {:<20}",
            "ID", "Name", "Tracks", "Updated"
        );
        println!("{}", "-".repeat(80));

        for playlist in &playlists {
            println!(
                "{:<6} {:<40} {:>10} {:<20}",
                playlist.id,
                truncate(&playlist.name, 40),
                playlist.tracks.len(),
                &playlist.updated_at[..19] // Trim to datetime
            );
        }

        println!("\nTotal: {} playlists", playlists.len());

        // Option to view playlist details
        let choice =
            Self::read_input("\nEnter playlist ID to view details (or press Enter to go back): ")?;
        if !choice.trim().is_empty() {
            let playlist_id: i64 = choice.trim().parse()?;
            self.show_playlist_details(playlist_id)?;
        }

        Ok(())
    }

    fn show_playlist_details(&self, playlist_id: i64) -> Result<(), Box<dyn std::error::Error>> {
        let playlist = self.playlist.get_playlist(playlist_id)?;

        if playlist.is_none() {
            println!("❌ Playlist not found");
            return Ok(());
        }

        let playlist = playlist.unwrap();

        println!("\n📋 Playlist: {}", playlist.name);
        println!(
            "Description: {}",
            playlist.description.as_deref().unwrap_or("None")
        );
        println!("Created: {}", &playlist.created_at[..19]);
        println!("Updated: {}", &playlist.updated_at[..19]);
        println!(
            "\n{:<6} {:<6} {:<40} {:<25}",
            "Pos", "ID", "Title", "Artist ID"
        );
        println!("{}", "-".repeat(80));

        for (pos, pt) in playlist.tracks.iter().enumerate() {
            // Fetch track details
            let conn = self.pool.get()?;
            if let Ok(Some(track)) = crate::db::queries::get_track(&conn, pt.track_id) {
                println!(
                    "{:<6} {:<6} {:<40} {:<25}",
                    pos,
                    track.id,
                    truncate(&track.title, 40),
                    track.artist_id
                );
            }
        }

        println!("\nTotal: {} tracks", playlist.tracks.len());

        // Option to play playlist
        let choice = Self::read_input("\nPlay playlist? (y/n): ")?;
        if choice.trim().to_lowercase() == "y" {
            let track_ids: Vec<i64> = playlist.tracks.iter().map(|pt| pt.track_id).collect();
            self.player.set_queue(track_ids.clone())?;
            if !track_ids.is_empty() {
                self.player.play(track_ids[0])?;
                println!("▶️  Playing playlist");
            }
        }

        Ok(())
    }

    fn create_playlist(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n📝 Create New Playlist");
        println!("=====================");

        let name = Self::read_input("Playlist name: ")?;
        let description = Self::read_input("Description (optional): ")?;

        let description = if description.trim().is_empty() {
            None
        } else {
            Some(description.trim())
        };

        match self.playlist.create_playlist(&name, description) {
            Ok(playlist) => {
                println!("✅ Playlist created successfully!");
                println!("   ID: {}", playlist.id);
                println!("   Name: {}", playlist.name);
            }
            Err(e) => println!("❌ Failed to create playlist: {e}"),
        }

        Ok(())
    }

    fn add_tracks_to_playlist(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n➕ Add Tracks to Playlist");
        println!("========================");

        // List playlists
        let playlists = self.playlist.list_playlists()?;
        if playlists.is_empty() {
            println!("❌ No playlists found. Create one first.");
            return Ok(());
        }

        println!("Available playlists:");
        for playlist in &playlists {
            println!(
                "  {}. {} ({} tracks)",
                playlist.id,
                playlist.name,
                playlist.tracks.len()
            );
        }

        let playlist_id_str = Self::read_input("\nEnter playlist ID: ")?;
        let playlist_id: i64 = playlist_id_str.trim().parse()?;

        let track_ids_str = Self::read_input("Enter track IDs (comma-separated): ")?;
        let track_ids: Vec<i64> = track_ids_str
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        if track_ids.is_empty() {
            println!("❌ No valid track IDs provided");
            return Ok(());
        }

        match self.playlist.add_tracks(playlist_id, track_ids.clone()) {
            Ok(()) => println!("✅ Added {} track(s) to playlist", track_ids.len()),
            Err(e) => println!("❌ Failed to add tracks: {e}"),
        }

        Ok(())
    }

    fn remove_track_from_playlist(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n➖ Remove Track from Playlist");
        println!("=============================");

        let playlists = self.playlist.list_playlists()?;
        if playlists.is_empty() {
            println!("❌ No playlists found.");
            return Ok(());
        }

        println!("Available playlists:");
        for playlist in &playlists {
            println!(
                "  {}. {} ({} tracks)",
                playlist.id,
                playlist.name,
                playlist.tracks.len()
            );
        }

        let playlist_id_str = Self::read_input("\nEnter playlist ID: ")?;
        let playlist_id: i64 = playlist_id_str.trim().parse()?;

        let position_str = Self::read_input("Enter track position to remove: ")?;
        let position: usize = position_str.trim().parse()?;

        match self.playlist.remove_track(playlist_id, position) {
            Ok(()) => println!("✅ Track removed from playlist"),
            Err(e) => println!("❌ Failed to remove track: {e}"),
        }

        Ok(())
    }

    fn move_track_in_playlist(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🔄 Move Track in Playlist");
        println!("=========================");

        let playlists = self.playlist.list_playlists()?;
        if playlists.is_empty() {
            println!("❌ No playlists found.");
            return Ok(());
        }

        println!("Available playlists:");
        for playlist in &playlists {
            println!(
                "  {}. {} ({} tracks)",
                playlist.id,
                playlist.name,
                playlist.tracks.len()
            );
        }

        let playlist_id_str = Self::read_input("\nEnter playlist ID: ")?;
        let playlist_id: i64 = playlist_id_str.trim().parse()?;

        let from_str = Self::read_input("From position: ")?;
        let from: usize = from_str.trim().parse()?;

        let to_str = Self::read_input("To position: ")?;
        let to: usize = to_str.trim().parse()?;

        match self.playlist.move_track(playlist_id, from, to) {
            Ok(()) => println!("✅ Track moved in playlist"),
            Err(e) => println!("❌ Failed to move track: {e}"),
        }

        Ok(())
    }

    fn rename_playlist(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n✏️  Rename Playlist");
        println!("==================");

        let playlists = self.playlist.list_playlists()?;
        if playlists.is_empty() {
            println!("❌ No playlists found.");
            return Ok(());
        }

        println!("Available playlists:");
        for playlist in &playlists {
            println!("  {}. {}", playlist.id, playlist.name);
        }

        let playlist_id_str = Self::read_input("\nEnter playlist ID: ")?;
        let playlist_id: i64 = playlist_id_str.trim().parse()?;

        let new_name = Self::read_input("New name: ")?;

        match self.playlist.rename_playlist(playlist_id, &new_name) {
            Ok(()) => println!("✅ Playlist renamed successfully"),
            Err(e) => println!("❌ Failed to rename playlist: {e}"),
        }

        Ok(())
    }

    fn delete_playlist(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🗑️  Delete Playlist");
        println!("==================");

        let playlists = self.playlist.list_playlists()?;
        if playlists.is_empty() {
            println!("❌ No playlists found.");
            return Ok(());
        }

        println!("Available playlists:");
        for playlist in &playlists {
            println!("  {}. {}", playlist.id, playlist.name);
        }

        let playlist_id_str = Self::read_input("\nEnter playlist ID: ")?;
        let playlist_id: i64 = playlist_id_str.trim().parse()?;

        let confirm = Self::read_input("Are you sure you want to delete this playlist? (y/n): ")?;

        if confirm.trim().to_lowercase() == "y" {
            match self.playlist.delete_playlist(playlist_id) {
                Ok(()) => println!("✅ Playlist deleted successfully"),
                Err(e) => println!("❌ Failed to delete playlist: {e}"),
            }
        } else {
            println!("❌ Deletion cancelled");
        }

        Ok(())
    }

    fn import_m3u_playlist(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n📥 Import M3U Playlist");
        println!("=====================");

        let path_str = Self::read_input("Path to M3U file: ")?;
        let path = PathBuf::from(path_str.trim());

        if !path.exists() {
            println!("❌ File does not exist: {path:?}");
            return Ok(());
        }

        match self.playlist.import_m3u(&path) {
            Ok(playlist) => {
                println!("✅ M3U playlist imported successfully!");
                println!("   ID: {}", playlist.id);
                println!("   Name: {}", playlist.name);
                println!("   Tracks: {}", playlist.tracks.len());
            }
            Err(e) => println!("❌ Failed to import M3U: {e}"),
        }

        Ok(())
    }

    fn export_m3u_playlist(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n📤 Export Playlist to M3U");
        println!("=========================");

        let playlists = self.playlist.list_playlists()?;
        if playlists.is_empty() {
            println!("❌ No playlists found.");
            return Ok(());
        }

        println!("Available playlists:");
        for playlist in &playlists {
            println!(
                "  {}. {} ({} tracks)",
                playlist.id,
                playlist.name,
                playlist.tracks.len()
            );
        }

        let playlist_id_str = Self::read_input("\nEnter playlist ID: ")?;
        let playlist_id: i64 = playlist_id_str.trim().parse()?;

        let path_str = Self::read_input("Output path (e.g., /path/to/playlist.m3u): ")?;
        let path = PathBuf::from(path_str.trim());

        match self.playlist.export_m3u(playlist_id, &path) {
            Ok(()) => {
                println!("✅ Playlist exported successfully to: {path:?}");
            }
            Err(e) => println!("❌ Failed to export M3U: {e}"),
        }

        Ok(())
    }

    fn find_duplicates(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🔍 Find Duplicate Tracks");
        println!("========================");

        println!("Scanning library for duplicates...");
        let duplicates = self.duplicate.find_duplicates()?;

        if duplicates.is_empty() {
            println!("✅ No duplicates found!");
            return Ok(());
        }

        println!("Found {} duplicate groups:\n", duplicates.len());

        for (idx, group) in duplicates.iter().enumerate() {
            println!("Group {} ({} tracks):", idx + 1, group.tracks.len());
            println!(
                "{:<6} {:<40} {:<25} {:<15} {:>10}",
                "ID", "Title", "Artist ID", "Format", "Size"
            );
            println!("{}", "-".repeat(100));

            for track in &group.tracks {
                let size_mb = track.file_size as f64 / 1_048_576.0;
                println!(
                    "{:<6} {:<40} {:<25} {:<15} {:>9.2} MB",
                    track.id,
                    truncate(&track.title, 40),
                    track.artist_id,
                    track.file_type.as_str(),
                    size_mb
                );
            }
            println!();
        }

        // Get statistics
        let (num_groups, num_tracks) = self.duplicate.get_duplicate_stats()?;
        println!("Total: {num_groups} duplicate groups with {num_tracks} tracks");

        // Option to mark duplicates
        let choice = Self::read_input(
            "\nMark a track as duplicate? Enter track ID (or press Enter to skip): ",
        )?;
        if !choice.trim().is_empty() {
            let track_id: i64 = choice.trim().parse()?;
            let original_id_str = Self::read_input("Enter ID of original track to keep: ")?;
            let original_id: i64 = original_id_str.trim().parse()?;

            match self.duplicate.hide_duplicate(track_id, original_id) {
                Ok(()) => println!("✅ Track {track_id} marked as duplicate of {original_id}"),
                Err(e) => println!("❌ Failed to mark duplicate: {e}"),
            }
        }

        Ok(())
    }

    fn reset_library(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n⚠️  Reset Library");
        println!("================");
        println!("⚠️  WARNING: This will delete ALL data:");
        println!("   - All sources");
        println!("   - All tracks");
        println!("   - All albums and artists");
        println!("   - All playlists");
        println!("   - All statistics and ratings");
        println!();

        let confirm = Self::read_input("Type 'DELETE' to confirm: ")?;

        if confirm.trim() != "DELETE" {
            println!("❌ Reset cancelled.");
            return Ok(());
        }

        println!("\n🗑️  Resetting library...");

        match crate::db::reset_database(&self.pool) {
            Ok(()) => {
                println!("✅ Library reset complete!");
                println!("   Database is now empty and ready to use.");
            }
            Err(e) => {
                println!("❌ Failed to reset library: {e}");
                return Err(Box::new(e));
            }
        }

        Ok(())
    }

    fn fetch_artwork(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🎨 Fetch Artwork from Online Sources");
        println!("====================================");
        println!();
        println!("Options:");
        println!("  1. Albums only");
        println!("  2. Albums + Artists");
        println!("  3. Cancel");
        println!();

        let choice = Self::read_input("Enter your choice: ")?;

        let fetch_artists = match choice.trim() {
            "1" => false,
            "2" => true,
            "3" => {
                println!("Cancelled.");
                return Ok(());
            }
            _ => {
                println!("❌ Invalid choice");
                return Ok(());
            }
        };

        println!("\n⏳ Fetching artwork from MusicBrainz...");
        println!("This may take a while depending on library size.");
        println!("Rate limited to 1 request/second to respect API limits.\n");

        // Create a tokio runtime to run async code
        let runtime = tokio::runtime::Runtime::new()?;

        // Clone the service for the async block
        let artwork_service = self.artwork.clone();

        runtime.block_on(async move {
            match artwork_service.fetch_all_artwork(fetch_artists, false).await {
                Ok(()) => {
                    // Poll for progress until complete
                    while let Some(progress) = artwork_service.get_progress() {
                        // Clear line and print progress
                        print!(
                            "\r  Progress: {}/{} | ✓ {} | ✗ {} | Current: {}                    ",
                            progress.processed_items,
                            progress.total_items,
                            progress.successful,
                            progress.failed,
                            truncate(&progress.current_item, 40)
                        );
                        std::io::stdout().flush().unwrap();

                        if progress.processed_items >= progress.total_items {
                            println!("\n");
                            break;
                        }

                        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                    }

                    if let Some(final_progress) = artwork_service.get_progress() {
                        println!("✅ Artwork fetch complete!");
                        println!("   Total items: {}", final_progress.total_items);
                        println!("   Successful: {}", final_progress.successful);
                        println!("   Failed: {}", final_progress.failed);
                    }
                }
                Err(e) => {
                    println!("\n❌ Failed to fetch artwork: {e}");
                }
            }
        });

        Ok(())
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}
