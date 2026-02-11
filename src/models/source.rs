// Source model

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    Disk,
    Ipod,
    Iphone,
}

impl SourceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceType::Disk => "disk",
            SourceType::Ipod => "ipod",
            SourceType::Iphone => "iphone",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "disk" => Some(SourceType::Disk),
            "ipod" => Some(SourceType::Ipod),
            "iphone" => Some(SourceType::Iphone),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: i64,
    pub name: String,
    pub source_type: SourceType,
    pub path: Option<PathBuf>,
    pub device_id: Option<String>,
    pub last_scanned_at: Option<String>,  // ISO 8601
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
