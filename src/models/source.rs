//! Music-source domain model.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The physical medium or device a [`Source`] resides on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    /// A local or network-mounted disk directory.
    Disk,
    /// An iPod Classic device (imported via USB).
    Ipod,
    /// An iPhone music library (imported via USB).
    Iphone,
}

impl SourceType {
    /// Returns the canonical lowercase string representation stored in the database.
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceType::Disk => "disk",
            SourceType::Ipod => "ipod",
            SourceType::Iphone => "iphone",
        }
    }

    /// Parse a database-stored string back into a [`SourceType`].
    ///
    /// Matching is case-insensitive. Returns `None` if the string is not recognised.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "disk" => Some(SourceType::Disk),
            "ipod" => Some(SourceType::Ipod),
            "iphone" => Some(SourceType::Iphone),
            _ => None,
        }
    }
}

/// A music source — a root directory or device that the library scanner indexes.
///
/// Each source maps to one row in the `sources` table. Multiple sources can
/// coexist (e.g. a NAS share and a local SSD), and each [`crate::models::Track`]
/// references its source via `source_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// SQLite primary key (`sources.id`).
    pub id: i64,
    /// Human-readable label shown in the UI (e.g. `"NAS Music"`, `"Local Library"`).
    pub name: String,
    /// The type of storage medium this source resides on.
    pub source_type: SourceType,
    /// Root filesystem path for [`SourceType::Disk`] sources. `None` for device sources.
    pub path: Option<PathBuf>,
    /// Unique device identifier for [`SourceType::Ipod`] / [`SourceType::Iphone`] sources.
    pub device_id: Option<String>,
    /// Timestamp of the most recent completed scan (ISO 8601, UTC). `None` if never scanned.
    pub last_scanned_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_type_from_str() {
        assert_eq!(SourceType::from_str("disk"), Some(SourceType::Disk));
        assert_eq!(SourceType::from_str("DISK"), Some(SourceType::Disk));
        assert_eq!(SourceType::from_str("ipod"), Some(SourceType::Ipod));
        assert_eq!(SourceType::from_str("iphone"), Some(SourceType::Iphone));
        assert_eq!(SourceType::from_str("unknown"), None);
    }

    #[test]
    fn test_source_type_as_str_roundtrip() {
        for st in [SourceType::Disk, SourceType::Ipod, SourceType::Iphone] {
            let s = st.as_str();
            let back = SourceType::from_str(s).unwrap();
            assert_eq!(back, st);
        }
    }
}
