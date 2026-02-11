// Reports generation for scan and artwork operations

use std::fs;
use std::path::PathBuf;
use chrono::{DateTime, Local};

/// Scan report data
#[derive(Debug, Clone)]
pub struct ScanReport {
    pub start_time: DateTime<Local>,
    pub end_time: DateTime<Local>,
    pub folder_path: String,
    pub total_files: usize,
    pub tracks_added: usize,
    pub albums_created: usize,
    pub artists_created: usize,
    pub errors: Vec<String>,
}

impl ScanReport {
    pub fn new(folder_path: String, start_time: DateTime<Local>) -> Self {
        Self {
            start_time,
            end_time: Local::now(),
            folder_path,
            total_files: 0,
            tracks_added: 0,
            albums_created: 0,
            artists_created: 0,
            errors: Vec::new(),
        }
    }

    pub fn duration_seconds(&self) -> i64 {
        (self.end_time - self.start_time).num_seconds()
    }

    pub fn format_duration(&self) -> String {
        let seconds = self.duration_seconds();
        let minutes = seconds / 60;
        let remaining_seconds = seconds % 60;

        if minutes > 0 {
            format!("{}m {}s", minutes, remaining_seconds)
        } else {
            format!("{}s", seconds)
        }
    }

    pub fn to_text(&self) -> String {
        let mut report = String::new();

        report.push_str("═══════════════════════════════════════════════════════════\n");
        report.push_str("                  LIBRARY SCAN REPORT\n");
        report.push_str("═══════════════════════════════════════════════════════════\n\n");

        report.push_str(&format!("Scan started:  {}\n", self.start_time.format("%Y-%m-%d %H:%M:%S")));
        report.push_str(&format!("Scan finished: {}\n", self.end_time.format("%Y-%m-%d %H:%M:%S")));
        report.push_str(&format!("Duration:      {}\n\n", self.format_duration()));

        report.push_str(&format!("Folder scanned:\n  {}\n\n", self.folder_path));

        report.push_str("RESULTS\n");
        report.push_str("───────────────────────────────────────────────────────────\n");
        report.push_str(&format!("📁 Files found:     {:>6}\n", self.total_files));
        report.push_str(&format!("🎵 Tracks added:    {:>6}\n", self.tracks_added));
        report.push_str(&format!("💿 Albums created:  {:>6}\n", self.albums_created));
        report.push_str(&format!("👤 Artists created: {:>6}\n", self.artists_created));

        if !self.errors.is_empty() {
            report.push_str(&format!("\n⚠️  ERRORS ({} total)\n", self.errors.len()));
            report.push_str("───────────────────────────────────────────────────────────\n");
            for (i, error) in self.errors.iter().enumerate().take(10) {
                report.push_str(&format!("{}. {}\n", i + 1, error));
            }
            if self.errors.len() > 10 {
                report.push_str(&format!("\n... and {} more errors\n", self.errors.len() - 10));
            }
        }

        report.push_str("\n═══════════════════════════════════════════════════════════\n");
        report.push_str(&format!("Report generated: {}\n", Local::now().format("%Y-%m-%d %H:%M:%S")));
        report.push_str("═══════════════════════════════════════════════════════════\n");

        report
    }

    pub fn save(&self, reports_dir: &PathBuf) -> Result<PathBuf, String> {
        // Create reports directory if it doesn't exist
        fs::create_dir_all(reports_dir).map_err(|e| e.to_string())?;

        // Generate filename with timestamp
        let filename = format!(
            "scan_{}.txt",
            self.start_time.format("%Y%m%d_%H%M%S")
        );
        let filepath = reports_dir.join(filename);

        // Write report
        fs::write(&filepath, self.to_text()).map_err(|e| e.to_string())?;

        Ok(filepath)
    }
}

/// Artwork scraping report data
#[derive(Debug, Clone)]
pub struct ArtworkReport {
    pub start_time: DateTime<Local>,
    pub end_time: DateTime<Local>,
    pub total_albums: usize,
    pub albums_successful: usize,
    pub albums_failed: Vec<(String, String)>, // (album name, reason)
    pub total_artists: usize,
    pub artists_successful: usize,
    pub artists_failed: Vec<(String, String)>, // (artist name, reason)
}

impl ArtworkReport {
    pub fn new(start_time: DateTime<Local>) -> Self {
        Self {
            start_time,
            end_time: Local::now(),
            total_albums: 0,
            albums_successful: 0,
            albums_failed: Vec::new(),
            total_artists: 0,
            artists_successful: 0,
            artists_failed: Vec::new(),
        }
    }

    pub fn duration_seconds(&self) -> i64 {
        (self.end_time - self.start_time).num_seconds()
    }

    pub fn format_duration(&self) -> String {
        let seconds = self.duration_seconds();
        let minutes = seconds / 60;
        let remaining_seconds = seconds % 60;

        if minutes > 0 {
            format!("{}m {}s", minutes, remaining_seconds)
        } else {
            format!("{}s", seconds)
        }
    }

    pub fn albums_failed_count(&self) -> usize {
        self.albums_failed.len()
    }

    pub fn artists_failed_count(&self) -> usize {
        self.artists_failed.len()
    }

    pub fn success_rate(&self) -> f64 {
        let total = self.total_albums + self.total_artists;
        let successful = self.albums_successful + self.artists_successful;

        if total == 0 {
            0.0
        } else {
            (successful as f64 / total as f64) * 100.0
        }
    }

    pub fn to_text(&self) -> String {
        let mut report = String::new();

        report.push_str("═══════════════════════════════════════════════════════════\n");
        report.push_str("               ARTWORK SCRAPING REPORT\n");
        report.push_str("═══════════════════════════════════════════════════════════\n\n");

        report.push_str(&format!("Scraping started:  {}\n", self.start_time.format("%Y-%m-%d %H:%M:%S")));
        report.push_str(&format!("Scraping finished: {}\n", self.end_time.format("%Y-%m-%d %H:%M:%S")));
        report.push_str(&format!("Duration:          {}\n\n", self.format_duration()));

        report.push_str("ALBUMS\n");
        report.push_str("───────────────────────────────────────────────────────────\n");
        report.push_str(&format!("Total albums:      {:>6}\n", self.total_albums));
        report.push_str(&format!("✅ Successful:     {:>6}\n", self.albums_successful));
        report.push_str(&format!("❌ Failed:         {:>6}\n", self.albums_failed_count()));

        if self.total_albums > 0 {
            let rate = (self.albums_successful as f64 / self.total_albums as f64) * 100.0;
            report.push_str(&format!("Success rate:      {:>5.1}%\n", rate));
        }

        if !self.albums_failed.is_empty() {
            report.push_str("\nFailed albums:\n");
            for (i, (name, reason)) in self.albums_failed.iter().enumerate().take(20) {
                report.push_str(&format!("  {}. {} - {}\n", i + 1, name, reason));
            }
            if self.albums_failed.len() > 20 {
                report.push_str(&format!("  ... and {} more\n", self.albums_failed.len() - 20));
            }
        }

        report.push_str("\nARTISTS\n");
        report.push_str("───────────────────────────────────────────────────────────\n");
        report.push_str(&format!("Total artists:     {:>6}\n", self.total_artists));
        report.push_str(&format!("✅ Successful:     {:>6}\n", self.artists_successful));
        report.push_str(&format!("❌ Failed:         {:>6}\n", self.artists_failed_count()));

        if self.total_artists > 0 {
            let rate = (self.artists_successful as f64 / self.total_artists as f64) * 100.0;
            report.push_str(&format!("Success rate:      {:>5.1}%\n", rate));
        }

        if !self.artists_failed.is_empty() {
            report.push_str("\nFailed artists:\n");
            for (i, (name, reason)) in self.artists_failed.iter().enumerate().take(20) {
                report.push_str(&format!("  {}. {} - {}\n", i + 1, name, reason));
            }
            if self.artists_failed.len() > 20 {
                report.push_str(&format!("  ... and {} more\n", self.artists_failed.len() - 20));
            }
        }

        report.push_str("\nOVERALL\n");
        report.push_str("───────────────────────────────────────────────────────────\n");
        let total = self.total_albums + self.total_artists;
        let successful = self.albums_successful + self.artists_successful;
        let failed = self.albums_failed_count() + self.artists_failed_count();
        report.push_str(&format!("Total items:       {:>6}\n", total));
        report.push_str(&format!("✅ Successful:     {:>6}\n", successful));
        report.push_str(&format!("❌ Failed:         {:>6}\n", failed));
        report.push_str(&format!("Success rate:      {:>5.1}%\n", self.success_rate()));

        report.push_str("\n═══════════════════════════════════════════════════════════\n");
        report.push_str("Source: MusicBrainz + Cover Art Archive (free, no API key)\n");
        report.push_str(&format!("Report generated: {}\n", Local::now().format("%Y-%m-%d %H:%M:%S")));
        report.push_str("═══════════════════════════════════════════════════════════\n");

        report
    }

    pub fn save(&self, reports_dir: &PathBuf) -> Result<PathBuf, String> {
        // Create reports directory if it doesn't exist
        fs::create_dir_all(reports_dir).map_err(|e| e.to_string())?;

        // Generate filename with timestamp
        let filename = format!(
            "artwork_{}.txt",
            self.start_time.format("%Y%m%d_%H%M%S")
        );
        let filepath = reports_dir.join(filename);

        // Write report
        fs::write(&filepath, self.to_text()).map_err(|e| e.to_string())?;

        Ok(filepath)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    fn make_scan_report(seconds: i64) -> ScanReport {
        let start = Local::now() - ChronoDuration::seconds(seconds);
        let mut r = ScanReport::new("/music".to_string(), start);
        r.end_time = start + ChronoDuration::seconds(seconds);
        r.total_files = 100;
        r.tracks_added = 90;
        r.albums_created = 10;
        r.artists_created = 5;
        r
    }

    #[test]
    fn test_scan_report_new() {
        let start = Local::now();
        let report = ScanReport::new("/test".to_string(), start);
        assert_eq!(report.folder_path, "/test");
        assert_eq!(report.total_files, 0);
        assert_eq!(report.tracks_added, 0);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn test_scan_report_duration() {
        let report = make_scan_report(65);
        assert_eq!(report.duration_seconds(), 65);
    }

    #[test]
    fn test_scan_report_format_duration_seconds_only() {
        let report = make_scan_report(42);
        assert_eq!(report.format_duration(), "42s");
    }

    #[test]
    fn test_scan_report_format_duration_with_minutes() {
        let report = make_scan_report(125);
        assert_eq!(report.format_duration(), "2m 5s");
    }

    #[test]
    fn test_scan_report_to_text() {
        let report = make_scan_report(10);
        let text = report.to_text();
        assert!(text.contains("LIBRARY SCAN REPORT"));
        assert!(text.contains("/music"));
        assert!(text.contains("100"));
        assert!(text.contains("90"));
    }

    #[test]
    fn test_scan_report_to_text_with_errors() {
        let mut report = make_scan_report(10);
        report.errors = vec!["Error 1".to_string(), "Error 2".to_string()];
        let text = report.to_text();
        assert!(text.contains("ERRORS (2 total)"));
        assert!(text.contains("Error 1"));
    }

    #[test]
    fn test_scan_report_save() {
        let tmp = tempfile::tempdir().unwrap();
        let report = make_scan_report(5);
        let path = report.save(&tmp.path().to_path_buf()).unwrap();
        assert!(path.exists());
        assert!(path.to_string_lossy().contains("scan_"));
    }

    #[test]
    fn test_scan_report_filename_format() {
        let tmp = tempfile::tempdir().unwrap();
        let report = make_scan_report(5);
        let path = report.save(&tmp.path().to_path_buf()).unwrap();
        let filename = path.file_name().unwrap().to_string_lossy();
        assert!(filename.starts_with("scan_"));
        assert!(filename.ends_with(".txt"));
    }

    fn make_artwork_report() -> ArtworkReport {
        let start = Local::now() - ChronoDuration::seconds(30);
        let mut r = ArtworkReport::new(start);
        r.end_time = start + ChronoDuration::seconds(30);
        r.total_albums = 10;
        r.albums_successful = 7;
        r.albums_failed = vec![
            ("Album X".to_string(), "Not found".to_string()),
            ("Album Y".to_string(), "Timeout".to_string()),
            ("Album Z".to_string(), "Error".to_string()),
        ];
        r.total_artists = 5;
        r.artists_successful = 3;
        r.artists_failed = vec![
            ("Artist A".to_string(), "Not found".to_string()),
            ("Artist B".to_string(), "Error".to_string()),
        ];
        r
    }

    #[test]
    fn test_artwork_report_new() {
        let start = Local::now();
        let report = ArtworkReport::new(start);
        assert_eq!(report.total_albums, 0);
        assert_eq!(report.albums_successful, 0);
        assert!(report.albums_failed.is_empty());
    }

    #[test]
    fn test_artwork_report_success_rate() {
        let report = make_artwork_report();
        let rate = report.success_rate();
        assert!((rate - 66.666).abs() < 1.0);
    }

    #[test]
    fn test_artwork_report_success_rate_zero() {
        let start = Local::now();
        let report = ArtworkReport::new(start);
        assert_eq!(report.success_rate(), 0.0);
    }

    #[test]
    fn test_artwork_report_to_text() {
        let report = make_artwork_report();
        let text = report.to_text();
        assert!(text.contains("ARTWORK SCRAPING REPORT"));
        assert!(text.contains("ALBUMS"));
        assert!(text.contains("ARTISTS"));
        assert!(text.contains("OVERALL"));
        assert!(text.contains("Album X"));
    }

    #[test]
    fn test_artwork_report_save() {
        let tmp = tempfile::tempdir().unwrap();
        let report = make_artwork_report();
        let path = report.save(&tmp.path().to_path_buf()).unwrap();
        assert!(path.exists());
        let filename = path.file_name().unwrap().to_string_lossy();
        assert!(filename.starts_with("artwork_"));
        assert!(filename.ends_with(".txt"));
    }
}
