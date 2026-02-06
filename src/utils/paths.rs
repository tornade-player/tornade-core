// Application directory paths

use std::path::PathBuf;
use std::fs;
use std::env;

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
        // Use ~/.config/tornade for all data (Unix-style)
        let home_dir = env::var("HOME")
            .map_err(|_| std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine HOME directory"
            ))?;

        let base_dir = PathBuf::from(home_dir).join(".config").join("tornade");

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
}
