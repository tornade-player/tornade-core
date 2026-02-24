// Application directory paths

use std::fs;
use std::path::PathBuf;

/// Get application directories
#[derive(Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl AppPaths {
    /// Initialize application directories
    pub fn new() -> std::io::Result<Self> {
        // Use <home>/.config/tornade for all data.
        // `directories::BaseDirs` resolves the home directory cross-platform
        // (HOME on Unix, USERPROFILE / FOLDERID_Profile on Windows).
        let home_dir = directories::BaseDirs::new()
            .map(|b| b.home_dir().to_path_buf())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not determine home directory",
                )
            })?;

        let base_dir = home_dir.join(".config").join("tornade");

        let config_dir = base_dir.clone();
        let data_dir = base_dir.clone();
        let cache_dir = base_dir.join("cache");

        // Create directories if they don't exist
        fs::create_dir_all(&config_dir)?;
        fs::create_dir_all(&data_dir)?;
        fs::create_dir_all(&cache_dir)?;

        // Create subdirectories
        fs::create_dir_all(cache_dir.join("artwork"))?;

        // Create assets directory for online artwork
        let assets_dir = base_dir.join("assets");
        fs::create_dir_all(assets_dir.join("albums"))?;
        fs::create_dir_all(assets_dir.join("artists"))?;

        // Create reports directory
        fs::create_dir_all(base_dir.join("reports"))?;

        Ok(AppPaths {
            config_dir,
            data_dir,
            cache_dir,
        })
    }

    /// Get database file path
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("library.db")
    }

    /// Get config file path
    pub fn config_path(&self) -> PathBuf {
        self.config_dir.join("config.json")
    }

    /// Get artwork cache directory
    pub fn artwork_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("artwork")
    }

    /// Get assets directory (for online artwork)
    pub fn assets_dir(&self) -> PathBuf {
        self.config_dir.join("assets")
    }

    /// Get album artwork directory
    pub fn album_artwork_dir(&self) -> PathBuf {
        self.assets_dir().join("albums")
    }

    /// Get artist photo directory
    pub fn artist_photo_dir(&self) -> PathBuf {
        self.assets_dir().join("artists")
    }

    /// Get reports directory
    pub fn reports_dir(&self) -> PathBuf {
        self.config_dir.join("reports")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths() -> AppPaths {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join(".config").join("tornade");
        std::fs::create_dir_all(base.join("cache")).unwrap();
        AppPaths {
            config_dir: base.clone(),
            data_dir: base.clone(),
            cache_dir: base.join("cache"),
        }
    }

    #[test]
    fn test_database_path() {
        let paths = test_paths();
        assert!(paths.database_path().ends_with("library.db"));
        assert_eq!(paths.database_path(), paths.data_dir.join("library.db"));
    }

    #[test]
    fn test_artwork_dirs() {
        let paths = test_paths();
        assert!(paths.artwork_cache_dir().ends_with("artwork"));
        assert!(paths.album_artwork_dir().ends_with("albums"));
        assert!(paths.artist_photo_dir().ends_with("artists"));
    }

    #[test]
    fn test_reports_dir() {
        let paths = test_paths();
        assert!(paths.reports_dir().ends_with("reports"));
    }

    #[test]
    fn test_assets_dir() {
        let paths = test_paths();
        assert!(paths.assets_dir().ends_with("assets"));
        assert!(paths.config_path().ends_with("config.json"));
    }
}
