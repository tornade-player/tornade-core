// M3U playlist parser and writer

use crate::models::Track;
use crate::services::error::PlaylistError;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub struct M3uData {
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
        let track_path = if trimmed.starts_with('/') || trimmed.starts_with("~") {
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
        let artist_album = format!("{} - {}",
            track.artist_id,  // In real impl, would fetch artist name
            track.title
        );

        writeln!(file, "#EXTINF:{},{}", duration_secs, artist_album)?;

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
        writeln!(temp_file, "").unwrap();  // Empty line
        writeln!(temp_file, "/path/to/song2.flac").unwrap();
        temp_file.flush().unwrap();

        let result = parse_m3u(temp_file.path()).unwrap();
        assert_eq!(result.tracks.len(), 2);
    }
}
