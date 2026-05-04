//! Metadata scraping service for fetching tag candidates from MusicBrainz.
//!
//! This module provides [`MetadataScrapeService`] which queries MusicBrainz for
//! candidate metadata matches and returns ranked [`ScrapeCandidate`] results for
//! user review before writing tags.

use std::sync::Arc;

use crate::services::artwork::MusicBrainzClient;

/// A ranked metadata candidate returned from a MusicBrainz lookup.
#[derive(Debug, serde::Serialize)]
pub struct ScrapeCandidate {
    pub musicbrainz_id: String,
    pub title: String,
    /// Track artist (may differ from album artist on compilation releases).
    pub artist: String,
    /// Album artist (release-level artist credit).
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<u16>,
    pub genres: Vec<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub has_artwork: bool,
    /// Confidence score 0-100 derived from MusicBrainz match score.
    pub score: u8,
}

/// Service for querying MusicBrainz and producing ranked metadata candidates.
pub struct MetadataScrapeService {
    #[allow(dead_code)]
    client: Arc<MusicBrainzClient>,
}

impl MetadataScrapeService {
    pub fn new(client: Arc<MusicBrainzClient>) -> Self {
        Self { client }
    }
}
