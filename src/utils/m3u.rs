//! M3U playlist parser and writer.

use crate::models::Track;
use crate::services::error::PlaylistError;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Parsed contents of an M3U or M3U8 playlist file.
pub struct M3uData {
    /// Ordered list of file paths extracted from the playlist.
    /// Relative paths are resolved against the directory of the M3U file.
    pub tracks: Vec<PathBuf>,
}

/// Parse M3U file
pub fn parse_m3u(path: &Path) -> Result<M3uData, PlaylistError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut tracks = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();

        // Skip empty lines and comments (except #EXTM3U and #EXTINF)
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // This is a file path
        let track_path = if trimmed.starts_with('/') || trimmed.starts_with('~') {
            // Absolute path
            PathBuf::from(trimmed)
        } else {
            // Relative path - make it relative to M3U location
            if let Some(parent) = path.parent() {
                parent.join(trimmed)
            } else {
                PathBuf::from(trimmed)
            }
        };

        tracks.push(track_path);
    }

    Ok(M3uData { tracks })
}

/// Write M3U file
pub fn write_m3u(path: &Path, tracks: &[Track]) -> Result<(), PlaylistError> {
    let mut file = File::create(path)?;

    // Write M3U header
    writeln!(file, "#EXTM3U")?;

    for track in tracks {
        // Write extended info
        let duration_secs = track.duration.as_secs();
        let artist_album = format!(
            "{} - {}",
            track.artist_id, // In real impl, would fetch artist name
            track.title
        );

        writeln!(file, "#EXTINF:{duration_secs},{artist_album}")?;

        // Write file path
        writeln!(file, "{}", track.file_path.display())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_simple_m3u() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "#EXTM3U").unwrap();
        writeln!(temp_file, "#EXTINF:324,Artist - Song Title").unwrap();
        writeln!(temp_file, "/path/to/song.flac").unwrap();
        temp_file.flush().unwrap();

        let result = parse_m3u(temp_file.path()).unwrap();
        assert_eq!(result.tracks.len(), 1);
        assert_eq!(result.tracks[0], PathBuf::from("/path/to/song.flac"));
    }

    #[test]
    fn test_parse_m3u_with_comments() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "#EXTM3U").unwrap();
        writeln!(temp_file, "# This is a comment").unwrap();
        writeln!(temp_file, "/path/to/song1.flac").unwrap();
        writeln!(temp_file, "").unwrap(); // Empty line
        writeln!(temp_file, "/path/to/song2.flac").unwrap();
        temp_file.flush().unwrap();

        let result = parse_m3u(temp_file.path()).unwrap();
        assert_eq!(result.tracks.len(), 2);
    }

    // ── write_m3u ────────────────────────────────────────────────────────────

    fn make_track(id: i64, title: &str, file_path: &str, duration_secs: u64) -> Track {
        use crate::models::{AudioFormat, TrackBuilder};
        use std::time::Duration;
        TrackBuilder::new(
            title.to_string(),
            1,
            1,
            PathBuf::from(file_path),
            AudioFormat::Flac,
            1000,
            Duration::from_secs(duration_secs),
        )
        .id(id)
        .build()
    }

    #[test]
    fn test_write_m3u_creates_extm3u_header() {
        let tmp = NamedTempFile::new().unwrap();
        write_m3u(tmp.path(), &[]).unwrap();

        let contents = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(
            contents.starts_with("#EXTM3U"),
            "file must start with #EXTM3U header"
        );
    }

    #[test]
    fn test_write_m3u_empty_tracks_produces_header_only() {
        let tmp = NamedTempFile::new().unwrap();
        write_m3u(tmp.path(), &[]).unwrap();

        let contents = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(
            !contents.contains("#EXTINF"),
            "no EXTINF lines for empty track list"
        );
    }

    #[test]
    fn test_write_m3u_single_track_includes_extinf_and_path() {
        let tmp = NamedTempFile::new().unwrap();
        let track = make_track(1, "Song Title", "/music/song.flac", 180);
        write_m3u(tmp.path(), &[track]).unwrap();

        let contents = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(
            contents.contains("#EXTINF:180,"),
            "EXTINF must include duration in seconds"
        );
        assert!(
            contents.contains("/music/song.flac"),
            "track path must be written"
        );
    }

    #[test]
    fn test_write_m3u_multiple_tracks() {
        let tmp = NamedTempFile::new().unwrap();
        let tracks = vec![
            make_track(1, "Track 1", "/music/t1.flac", 120),
            make_track(2, "Track 2", "/music/t2.flac", 240),
            make_track(3, "Track 3", "/music/t3.flac", 300),
        ];
        write_m3u(tmp.path(), &tracks).unwrap();

        let contents = std::fs::read_to_string(tmp.path()).unwrap();
        let extinf_count = contents
            .lines()
            .filter(|l| l.starts_with("#EXTINF"))
            .count();
        assert_eq!(extinf_count, 3);
        assert!(contents.contains("/music/t1.flac"));
        assert!(contents.contains("/music/t2.flac"));
        assert!(contents.contains("/music/t3.flac"));
    }

    #[test]
    fn test_write_m3u_roundtrip_absolute_paths() {
        let tmp = NamedTempFile::new().unwrap();
        let tracks = vec![
            make_track(1, "A", "/music/a.flac", 100),
            make_track(2, "B", "/music/b.flac", 200),
        ];
        write_m3u(tmp.path(), &tracks).unwrap();

        let parsed = parse_m3u(tmp.path()).unwrap();
        assert_eq!(parsed.tracks.len(), 2);
        assert_eq!(parsed.tracks[0], PathBuf::from("/music/a.flac"));
        assert_eq!(parsed.tracks[1], PathBuf::from("/music/b.flac"));
    }
}
