// HTTP clients for artwork fetching

use crate::utils::MutexExt;
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Audio format/quality keywords that appear in file-tagger bracket annotations
/// but are not part of the actual album title in MusicBrainz.
const FORMAT_KEYWORDS: &[&str] = &[
    "flac", "mp3", "aac", "ogg", "opus", "wav", "aiff", "dsd", "sacd", "web", "mqa", "hires",
    "hi-res", "hdtracks", "tidal", "qobuz", "deezer", "kbps", "khz",
];

/// Remove `[...]` bracket groups whose content is a format/quality tag.
/// Examples: "[44.1-24 WEB]" → removed, "[Tidal MQA]" → removed.
/// Brackets whose content does NOT match are kept intact.
fn strip_format_brackets(title: &str) -> String {
    let mut result = String::with_capacity(title.len());
    let mut chars = title.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '[' {
            let mut content = String::new();
            let mut closed = false;
            for bc in chars.by_ref() {
                if bc == ']' {
                    closed = true;
                    break;
                }
                content.push(bc);
            }
            if closed {
                let lower = content.to_lowercase();
                let is_format = FORMAT_KEYWORDS.iter().any(|kw| lower.contains(kw));
                if !is_format {
                    result.push('[');
                    result.push_str(&content);
                    result.push(']');
                }
                // else: format tag, silently dropped
            } else {
                // Unclosed bracket — keep as-is
                result.push('[');
                result.push_str(&content);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Strip trailing disc annotations such as " (Disc 1)", " [Disc 2]", " - Disc 3", " (CD 1)".
/// MusicBrainz stores each disc as a separate release without the disc suffix.
fn strip_disc_suffix(title: &str) -> &str {
    let lower = title.to_lowercase();

    // Ordered from most to least specific to find the earliest match
    for marker in &["(disc ", "[disc ", "- disc ", "(cd "] {
        if let Some(pos) = lower.find(marker) {
            let after = &lower[pos + marker.len()..];
            if after.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return title[..pos].trim_end();
            }
        }
    }

    title
}

/// Return a clean album title suitable for MusicBrainz search:
/// strips format bracket tags and disc-number suffixes.
fn clean_album_title(title: &str) -> String {
    let t = strip_format_brackets(title);
    strip_disc_suffix(t.trim()).to_string()
}

/// Extract the primary artist name for a search query.
/// Handles multi-artist strings like "50 Cent, Snoop Dogg" → "50 Cent".
fn clean_artist_for_search(artist: &str) -> &str {
    artist.split(',').next().unwrap_or(artist).trim()
}

/// Strip Lucene special characters for an unquoted keyword search.
///
/// MusicBrainz uses Apache Lucene syntax. Characters like `:`, `(`, `)`, `[`, `]`,
/// `+`, `-`, `!`, `~`, `^`, `*`, `?` have special meaning and break unquoted queries.
/// For example `release:Ministry of Sound: The Score` is invalid syntax.
///
/// This function replaces all non-alphanumeric, non-space characters with spaces
/// and collapses whitespace, producing a safe keyword-only query.
fn sanitize_for_keyword_search(title: &str) -> String {
    title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '\'' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// All data returned when artwork is found: the image bytes plus release metadata
/// scraped from MusicBrainz at the same time.
#[derive(Debug)]
pub struct ArtworkSearchResult {
    pub image_data: Vec<u8>,
    /// MusicBrainz release UUID — used as the artwork filename (`{mbid}.jpg`).
    pub musicbrainz_id: String,
    pub label: Option<String>,
    pub country: Option<String>,
    pub barcode: Option<String>,
    /// Primary release-group type: "Album", "Single", "EP", "Compilation", etc.
    pub album_type: Option<String>,
    /// Release status: "Official", "Promotion", "Bootleg", etc.
    pub release_status: Option<String>,
    /// Release year as reported by MusicBrainz (may be more accurate than file tag).
    pub mb_year: Option<u16>,
}

/// All data returned when an artist is found in TheAudioDB: image bytes plus rich metadata.
#[derive(Debug)]
pub struct ArtistSearchResult {
    /// Raw JPEG bytes of the artist thumbnail, if one was found
    pub image_data: Option<Vec<u8>>,
    pub bio: Option<String>,
    pub country: Option<String>,
    pub genre: Option<String>,
    pub style: Option<String>,
    pub mood: Option<String>,
    pub formed_year: Option<i64>,
    pub born_year: Option<i64>,
    pub died_year: Option<i64>,
    pub disbanded: Option<String>,
    pub musicbrainz_id: Option<String>,
    pub theaudiodb_id: Option<String>,
}

/// Rate limiter to respect API rate limits
pub struct RateLimiter {
    min_interval_ms: u64,
    last_request: Option<Instant>,
}

impl RateLimiter {
    pub fn new(min_interval_ms: u64) -> Self {
        Self {
            min_interval_ms,
            last_request: None,
        }
    }

    /// Calculate wait time if necessary to respect rate limit
    pub fn calculate_wait(&mut self) -> Option<Duration> {
        let wait_time = if let Some(last) = self.last_request {
            let elapsed = last.elapsed();
            let min_duration = Duration::from_millis(self.min_interval_ms);

            if elapsed < min_duration {
                Some(min_duration - elapsed)
            } else {
                None
            }
        } else {
            None
        };

        self.last_request = Some(Instant::now());
        wait_time
    }
}

/// MusicBrainz API client (also handles TheAudioDB for artist photos)
pub struct MusicBrainzClient {
    http_client: reqwest::Client,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    musicbrainz_base_url: String,
    coverart_base_url: String,
    theaudiodb_base_url: String,
}

impl MusicBrainzClient {
    pub fn new(http_client: reqwest::Client, rate_limiter: Arc<Mutex<RateLimiter>>) -> Self {
        Self {
            http_client,
            rate_limiter,
            musicbrainz_base_url: "https://musicbrainz.org".to_string(),
            coverart_base_url: "https://coverartarchive.org".to_string(),
            theaudiodb_base_url: "https://www.theaudiodb.com".to_string(),
        }
    }

    /// Constructor with configurable base URLs.
    ///
    /// Intended for integration tests that want to point the client at a local
    /// mock server. Also useful for staging environments or embedded use-cases
    /// where the default MusicBrainz/CAA/TADB endpoints must be overridden.
    pub fn with_base_urls(
        http_client: reqwest::Client,
        rate_limiter: Arc<Mutex<RateLimiter>>,
        musicbrainz_base_url: &str,
        coverart_base_url: &str,
        theaudiodb_base_url: &str,
    ) -> Self {
        Self {
            http_client,
            rate_limiter,
            musicbrainz_base_url: musicbrainz_base_url.to_string(),
            coverart_base_url: coverart_base_url.to_string(),
            theaudiodb_base_url: theaudiodb_base_url.to_string(),
        }
    }

    /// Search for album artwork using a cascade of queries for better coverage.
    ///
    /// Returns the image bytes **and** the MusicBrainz release metadata when found.
    ///
    /// Strategy:
    ///   1. `release:"clean_title" AND artist:"primary_artist"` — most precise
    ///   2. `release:"clean_title"` — title-only (covers compilations / various-artist albums)
    ///   3. `release:clean_title` — keyword search without quotes (most permissive)
    ///
    /// Title and artist are pre-cleaned: format tags stripped, disc numbers removed,
    /// and only the first artist is used for multi-artist strings.
    pub async fn search_album_artwork(
        &self,
        album_title: &str,
        artist_name: &str,
    ) -> Result<Option<ArtworkSearchResult>, String> {
        let clean_title = clean_album_title(album_title);
        let clean_artist = clean_artist_for_search(artist_name);

        let queries = [
            format!("release:\"{clean_title}\" AND artist:\"{clean_artist}\""),
            format!("release:\"{clean_title}\""),
            // Unquoted keyword search: must sanitize Lucene special chars (colons, parens, etc.)
            // e.g. "Ministry of Sound: The Score" → "Ministry of Sound The Score"
            format!("release:{}", sanitize_for_keyword_search(&clean_title)),
        ];

        for query in &queries {
            if let Some(result) = self
                .try_search_query(query, artist_name, album_title)
                .await?
            {
                return Ok(Some(result));
            }
        }

        Ok(None)
    }

    /// Execute a single MusicBrainz query and attempt to download artwork from
    /// Cover Art Archive for the returned releases (up to 5).
    async fn try_search_query(
        &self,
        query: &str,
        artist_name: &str,
        album_title: &str,
    ) -> Result<Option<ArtworkSearchResult>, String> {
        let wait_time = {
            let mut limiter = self.rate_limiter.lock_infallible();
            limiter.calculate_wait()
        };
        if let Some(duration) = wait_time {
            tokio::time::sleep(duration).await;
        }

        let url = format!(
            "{}/ws/2/release/?query={}&fmt=json&limit=5",
            self.musicbrainz_base_url,
            urlencoding::encode(query)
        );

        log::debug!("Searching MusicBrainz: {url}");

        let response = self
            .http_client
            .get(&url)
            .header(
                "User-Agent",
                "Tornade-Music-Player/1.0 ( thomas@example.com )",
            )
            .send()
            .await
            .map_err(|e| format!("MusicBrainz search failed: {e}"))?;

        if !response.status().is_success() {
            return Err(format!(
                "MusicBrainz returned status: {}",
                response.status()
            ));
        }

        let search_result: MBSearchResult = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse MusicBrainz response: {e}"))?;

        if search_result.releases.is_empty() {
            return Ok(None);
        }

        // Try all returned releases (up to 5), skipping low-confidence ones
        for release in search_result.releases.iter().take(5) {
            if release.score.unwrap_or(100) < 50 {
                log::debug!(
                    "Skipping low-score release {} (score {:?})",
                    release.id,
                    release.score
                );
                continue;
            }

            let wait_time = {
                let mut limiter = self.rate_limiter.lock_infallible();
                limiter.calculate_wait()
            };
            if let Some(duration) = wait_time {
                tokio::time::sleep(duration).await;
            }

            let artwork_url = format!(
                "{}/release/{}/front-500",
                self.coverart_base_url, release.id
            );

            log::debug!("Trying Cover Art Archive: {artwork_url}");

            match self.http_client.get(&artwork_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    const MAX_IMAGE_SIZE: usize = 5 * 1024 * 1024;
                    match resp.bytes().await {
                        Ok(bytes) if bytes.len() <= MAX_IMAGE_SIZE => {
                            log::info!(
                                "Found artwork for {} - {} ({} KB)",
                                artist_name,
                                album_title,
                                bytes.len() / 1024
                            );
                            // Extract label name from first label-info entry
                            let label = release
                                .label_info
                                .as_ref()
                                .and_then(|info| info.first())
                                .and_then(|li| li.label.as_ref())
                                .map(|l| l.name.clone());

                            return Ok(Some(ArtworkSearchResult {
                                image_data: bytes.to_vec(),
                                musicbrainz_id: release.id.clone(),
                                label,
                                country: release.country.clone(),
                                barcode: release.barcode.clone(),
                                album_type: release
                                    .release_group
                                    .as_ref()
                                    .and_then(|rg| rg.primary_type.clone()),
                                release_status: release.status.clone(),
                                mb_year: release.date.as_deref().and_then(year_from_mb_date),
                            }));
                        }
                        Ok(bytes) => {
                            log::warn!(
                                "Artwork too large ({} MB) for {} - {}, skipping",
                                bytes.len() / 1024 / 1024,
                                artist_name,
                                album_title
                            );
                        }
                        Err(e) => {
                            log::warn!("Failed to download artwork bytes: {e}");
                        }
                    }
                }
                Ok(resp) => {
                    log::debug!(
                        "Cover Art Archive returned status {} for {}",
                        resp.status(),
                        release.id
                    );
                }
                Err(e) => {
                    log::debug!("Cover Art Archive request failed for {}: {}", release.id, e);
                }
            }
        }

        Ok(None)
    }

    /// Search for an artist photo and rich metadata via TheAudioDB.
    ///
    /// Uses the free public API (key `2`) — no registration required.
    /// Returns an `ArtistSearchResult` with all available metadata plus image bytes when found.
    /// Returns `Ok(None)` only if the artist was not found in TheAudioDB at all.
    pub async fn search_artist_photo(
        &self,
        artist_name: &str,
    ) -> Result<Option<ArtistSearchResult>, String> {
        let wait_time = {
            let mut limiter = self.rate_limiter.lock_infallible();
            limiter.calculate_wait()
        };
        if let Some(duration) = wait_time {
            tokio::time::sleep(duration).await;
        }

        let url = format!(
            "{}/api/v1/json/2/search.php?s={}",
            self.theaudiodb_base_url,
            urlencoding::encode(artist_name)
        );

        log::debug!("Searching TheAudioDB for artist: {url}");

        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("TheAudioDB search failed: {e}"))?;

        if !response.status().is_success() {
            return Err(format!("TheAudioDB returned status: {}", response.status()));
        }

        let search_result: TADBSearchResult = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse TheAudioDB response: {e}"))?;

        // TheAudioDB returns `"artists": null` when nothing is found
        let Some(artist) = search_result
            .artists
            .as_ref()
            .and_then(|artists| artists.first())
        else {
            log::debug!("Artist not found in TheAudioDB: {artist_name}");
            return Ok(None);
        };

        // Build result struct from all available metadata
        let mut result = ArtistSearchResult {
            image_data: None,
            bio: artist.biography_en.clone(),
            country: artist.country.clone(),
            genre: artist.genre.clone(),
            style: artist.style.clone(),
            mood: artist.mood.clone(),
            formed_year: artist
                .formed_year
                .as_deref()
                .and_then(|s| s.parse::<i64>().ok()),
            born_year: artist
                .born_year
                .as_deref()
                .and_then(|s| s.parse::<i64>().ok()),
            died_year: artist
                .died_year
                .as_deref()
                .and_then(|s| s.parse::<i64>().ok()),
            disbanded: artist.disbanded.clone(),
            musicbrainz_id: artist.musicbrainz_id.clone(),
            theaudiodb_id: artist.id.clone(),
        };

        // Download the thumbnail image if available
        if let Some(ref thumb_url) = artist.artist_thumb.clone() {
            let wait_time = {
                let mut limiter = self.rate_limiter.lock_infallible();
                limiter.calculate_wait()
            };
            if let Some(duration) = wait_time {
                tokio::time::sleep(duration).await;
            }

            log::debug!("Downloading artist photo: {thumb_url}");

            match self.http_client.get(thumb_url).send().await {
                Ok(img_response) if img_response.status().is_success() => {
                    const MAX_IMAGE_SIZE: usize = 5 * 1024 * 1024;
                    match img_response.bytes().await {
                        Ok(bytes) if bytes.len() <= MAX_IMAGE_SIZE => {
                            log::info!(
                                "Found artist photo for {} ({} KB)",
                                artist_name,
                                bytes.len() / 1024
                            );
                            result.image_data = Some(bytes.to_vec());
                        }
                        Ok(bytes) => {
                            log::warn!(
                                "Artist photo too large ({} MB) for {}, skipping image",
                                bytes.len() / 1024 / 1024,
                                artist_name
                            );
                        }
                        Err(e) => {
                            log::warn!("Failed to read artist photo bytes for {artist_name}: {e}");
                        }
                    }
                }
                Ok(img_response) => {
                    log::debug!(
                        "Artist photo download returned {} for {}",
                        img_response.status(),
                        artist_name
                    );
                }
                Err(e) => {
                    log::warn!("Artist photo download failed for {artist_name}: {e}");
                }
            }
        }

        // Return the result whether or not we got a photo — the metadata is always valuable
        Ok(Some(result))
    }

    /// Search MusicBrainz for recording (track) metadata candidates.
    ///
    /// Returns up to 5 `ScrapeCandidate` results ranked by score descending.
    pub async fn search_recording_metadata(
        &self,
        title: &str,
        artist: &str,
    ) -> Result<Vec<crate::services::metadata_scrape::ScrapeCandidate>, String> {
        let wait_time = {
            let mut limiter = self.rate_limiter.lock_infallible();
            limiter.calculate_wait()
        };
        if let Some(duration) = wait_time {
            tokio::time::sleep(duration).await;
        }

        let query = format!("recording:\"{title}\" AND artist:\"{artist}\"");
        let url = format!(
            "{}/ws/2/recording/?query={}&fmt=json&inc=artist-credits+releases+release-groups+genres+tags&limit=5",
            self.musicbrainz_base_url,
            urlencoding::encode(&query)
        );

        log::debug!("Searching MusicBrainz recordings: {url}");

        let response = self
            .http_client
            .get(&url)
            .header(
                "User-Agent",
                "Tornade-Music-Player/1.0 ( thomas@example.com )",
            )
            .send()
            .await
            .map_err(|e| format!("MusicBrainz recording search failed: {e}"))?;

        if !response.status().is_success() {
            return Err(format!(
                "MusicBrainz returned status: {}",
                response.status()
            ));
        }

        let search_result: MBRecordingSearchResult = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse MusicBrainz recording response: {e}"))?;

        let mut candidates: Vec<crate::services::metadata_scrape::ScrapeCandidate> = search_result
            .recordings
            .into_iter()
            .map(|recording| {
                let artist_name = recording
                    .artist_credit
                    .as_ref()
                    .and_then(|ac| ac.first())
                    .map(|ac| ac.artist.name.clone())
                    .unwrap_or_else(|| "Unknown".to_string());

                let release_id = recording
                    .releases
                    .as_ref()
                    .and_then(|r| r.first())
                    .map(|r| r.id.clone());

                let first_release = recording.releases.as_ref().and_then(|r| r.first());

                let album = first_release.map(|r| r.title.clone());

                // Album artist comes from the release's artist_credit, which may differ
                // from the recording's artist on compilation releases.
                let album_artist = first_release
                    .and_then(|r| r.artist_credit.as_ref())
                    .and_then(|ac| ac.first())
                    .map(|ac| ac.artist.name.clone());

                // Year: scan all releases for any date — first release may lack one.
                let year = recording
                    .releases
                    .as_ref()
                    .and_then(|releases| {
                        releases
                            .iter()
                            .find_map(|r| r.date.as_deref().and_then(year_from_mb_date))
                    });

                // Genre priority: recording → any release → release-group.
                let genres: Vec<String> = recording
                    .genres
                    .as_ref()
                    .map(|gs| gs.iter().map(|g| g.name.clone()).collect::<Vec<_>>())
                    .filter(|v| !v.is_empty())
                    .or_else(|| {
                        recording.releases.as_ref().and_then(|releases| {
                            releases.iter().find_map(|r| {
                                r.genres
                                    .as_ref()
                                    .filter(|gs| !gs.is_empty())
                                    .map(|gs| gs.iter().map(|g| g.name.clone()).collect())
                            })
                        })
                    })
                    .or_else(|| {
                        // Release-group genres.
                        recording.releases.as_ref().and_then(|releases| {
                            releases.iter().find_map(|r| {
                                r.release_group
                                    .as_ref()
                                    .and_then(|rg| rg.genres.as_ref())
                                    .filter(|gs| !gs.is_empty())
                                    .map(|gs| gs.iter().map(|g| g.name.clone()).collect())
                            })
                        })
                    })
                    .or_else(|| {
                        // Last resort: user-submitted tags (more broadly populated than genres).
                        recording
                            .tags
                            .as_ref()
                            .filter(|ts| !ts.is_empty())
                            .map(|ts| ts.iter().map(|t| t.name.clone()).collect())
                    })
                    .unwrap_or_default();

                let first_media = first_release
                    .and_then(|r| r.media.as_ref())
                    .and_then(|m| m.first());

                // Track number: prefer explicit position; fall back to track-offset + 1
                // (MB recording search may return offset without full tracks array).
                let track_number = first_media
                    .and_then(|m| m.tracks.as_ref())
                    .and_then(|t| t.first())
                    .and_then(|t| t.position)
                    .or_else(|| first_media.and_then(|m| m.track_offset).map(|o| o + 1));

                let disc_number = first_media.and_then(|m| m.disc_number);

                let score = recording.score.unwrap_or(0).clamp(0, 100) as u8;

                let artwork_id = release_id.unwrap_or_else(|| recording.id.clone());
                let has_artwork = first_release.is_some();

                crate::services::metadata_scrape::ScrapeCandidate {
                    musicbrainz_id: artwork_id,
                    title: recording.title,
                    artist: artist_name,
                    album_artist,
                    album,
                    year,
                    genres,
                    track_number,
                    disc_number,
                    has_artwork,
                    score,
                }
            })
            .collect();

        candidates.sort_by_key(|b| std::cmp::Reverse(b.score));
        Ok(candidates)
    }

    /// Search MusicBrainz for release (album) metadata candidates.
    ///
    /// Returns up to 5 `ScrapeCandidate` results ranked by score descending.
    pub async fn search_release_metadata(
        &self,
        album_title: &str,
        artist: &str,
    ) -> Result<Vec<crate::services::metadata_scrape::ScrapeCandidate>, String> {
        let wait_time = {
            let mut limiter = self.rate_limiter.lock_infallible();
            limiter.calculate_wait()
        };
        if let Some(duration) = wait_time {
            tokio::time::sleep(duration).await;
        }

        let query = format!("release:\"{album_title}\" AND artist:\"{artist}\"");
        let url = format!(
            "{}/ws/2/release/?query={}&fmt=json&inc=recordings+artist-credits+genres&limit=5",
            self.musicbrainz_base_url,
            urlencoding::encode(&query)
        );

        log::debug!("Searching MusicBrainz releases: {url}");

        let response = self
            .http_client
            .get(&url)
            .header(
                "User-Agent",
                "Tornade-Music-Player/1.0 ( thomas@example.com )",
            )
            .send()
            .await
            .map_err(|e| format!("MusicBrainz release search failed: {e}"))?;

        if !response.status().is_success() {
            return Err(format!(
                "MusicBrainz returned status: {}",
                response.status()
            ));
        }

        let search_result: MBSearchResult = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse MusicBrainz release response: {e}"))?;

        let mut candidates: Vec<crate::services::metadata_scrape::ScrapeCandidate> = search_result
            .releases
            .into_iter()
            .map(|release| {
                let artist_name = release
                    .artist_credit
                    .as_ref()
                    .and_then(|ac| ac.first())
                    .map(|ac| ac.artist.name.clone())
                    .unwrap_or_else(|| "Unknown".to_string());

                let year = release.date.as_deref().and_then(year_from_mb_date);

                let genres = release
                    .genres
                    .as_ref()
                    .map(|gs| gs.iter().map(|g| g.name.clone()).collect())
                    .unwrap_or_default();

                let has_artwork = release.release_group.is_some();

                let score = release.score.unwrap_or(0).clamp(0, 100) as u8;

                crate::services::metadata_scrape::ScrapeCandidate {
                    musicbrainz_id: release.id,
                    title: release.title,
                    artist: artist_name.clone(),
                    album_artist: Some(artist_name),
                    album: None,
                    year,
                    genres,
                    track_number: None,
                    disc_number: None,
                    has_artwork,
                    score,
                }
            })
            .collect();

        candidates.sort_by_key(|b| std::cmp::Reverse(b.score));
        Ok(candidates)
    }
}

#[derive(Debug, Deserialize)]
struct MBSearchResult {
    releases: Vec<MBRelease>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MBRelease {
    id: String,
    #[allow(dead_code)]
    title: String,
    score: Option<i32>,
    /// Release date, e.g. "1999-11-16" or "1999"
    date: Option<String>,
    country: Option<String>,
    barcode: Option<String>,
    status: Option<String>,
    #[serde(rename = "release-group")]
    release_group: Option<MBReleaseGroup>,
    #[serde(rename = "label-info")]
    label_info: Option<Vec<MBLabelInfo>>,
    #[serde(rename = "artist-credit")]
    pub artist_credit: Option<Vec<MBArtistCredit>>,
    pub genres: Option<Vec<MBGenre>>,
    pub media: Option<Vec<MBMedia>>,
}

#[derive(Debug, Deserialize)]
struct MBReleaseGroup {
    #[serde(rename = "primary-type")]
    primary_type: Option<String>,
    pub genres: Option<Vec<MBGenre>>,
}

#[derive(Debug, Deserialize)]
struct MBLabelInfo {
    label: Option<MBLabel>,
}

#[derive(Debug, Deserialize)]
struct MBLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
pub struct MBArtistCredit {
    pub artist: MBArtistRef,
}

#[derive(Debug, Deserialize)]
pub struct MBArtistRef {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct MBGenre {
    pub name: String,
    #[allow(dead_code)]
    pub count: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct MBMedia {
    pub tracks: Option<Vec<MBTrackInRelease>>,
    #[serde(rename = "position")]
    pub disc_number: Option<u32>,
    /// 0-based index of the first track of this recording within the disc.
    /// Used as fallback when `tracks` array is absent in recording search results.
    #[serde(rename = "track-offset")]
    pub track_offset: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct MBTrackInRelease {
    #[allow(dead_code)]
    pub title: String,
    #[allow(dead_code)]
    pub number: Option<String>,
    pub position: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct MBRecordingSearchResult {
    pub recordings: Vec<MBRecording>,
}

#[derive(Debug, Deserialize)]
pub struct MBRecording {
    pub id: String,
    pub title: String,
    #[serde(rename = "artist-credit")]
    pub artist_credit: Option<Vec<MBArtistCredit>>,
    pub releases: Option<Vec<MBRelease>>,
    pub genres: Option<Vec<MBGenre>>,
    /// User-submitted tags — more broadly populated than formal genres.
    pub tags: Option<Vec<MBTag>>,
    pub score: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct MBTag {
    pub name: String,
    #[allow(dead_code)]
    pub count: Option<u32>,
}

/// Extract a 4-digit year from a MusicBrainz date string ("1999-11-16" → Some(1999)).
fn year_from_mb_date(date: &str) -> Option<u16> {
    date.get(..4)?.parse().ok()
}

/// TheAudioDB search response — `artists` is `null` when nothing is found.
#[derive(Debug, Deserialize)]
struct TADBSearchResult {
    artists: Option<Vec<TADBArtist>>,
}

#[derive(Debug, Deserialize)]
struct TADBArtist {
    #[serde(rename = "idArtist")]
    id: Option<String>,
    /// URL of the artist thumbnail image.
    #[serde(rename = "strArtistThumb")]
    artist_thumb: Option<String>,
    #[serde(rename = "strBiographyEN")]
    biography_en: Option<String>,
    #[serde(rename = "strCountry")]
    country: Option<String>,
    #[serde(rename = "strGenre")]
    genre: Option<String>,
    #[serde(rename = "strStyle")]
    style: Option<String>,
    #[serde(rename = "strMood")]
    mood: Option<String>,
    /// TheAudioDB returns years as strings (e.g. "1970") or null
    #[serde(rename = "intFormedYear")]
    formed_year: Option<String>,
    #[serde(rename = "intBornYear")]
    born_year: Option<String>,
    #[serde(rename = "intDiedYear")]
    died_year: Option<String>,
    #[serde(rename = "strDisbanded")]
    disbanded: Option<String>,
    #[serde(rename = "strMusicBrainzID")]
    musicbrainz_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── clean_album_title ────────────────────────────────────────────────────

    #[test]
    fn test_clean_title_strips_web_format_tag() {
        assert_eq!(
            clean_album_title("Awesome Mix Vol. 1 [44.1-24 WEB]"),
            "Awesome Mix Vol. 1"
        );
    }

    #[test]
    fn test_clean_title_strips_tidal_mqa_tag() {
        assert_eq!(
            clean_album_title("Ralph Breaks the Internet [44.1-24 Tidal MQA]"),
            "Ralph Breaks the Internet"
        );
    }

    #[test]
    fn test_clean_title_strips_disc_suffix_parentheses() {
        assert_eq!(
            clean_album_title("Ministry of Sound: The Score (Disc 3)"),
            "Ministry of Sound: The Score"
        );
    }

    #[test]
    fn test_clean_title_strips_disc_suffix_dash() {
        assert_eq!(clean_album_title("Ultimate 80s - Disc 4"), "Ultimate 80s");
    }

    #[test]
    fn test_clean_title_strips_disc_suffix_bracket() {
        // [Disc 2] is not a format keyword so strip_format_brackets keeps it,
        // then strip_disc_suffix catches the "[disc " pattern.
        assert_eq!(clean_album_title("The Score [Disc 2]"), "The Score");
    }

    #[test]
    fn test_clean_title_strips_cd_suffix() {
        assert_eq!(clean_album_title("Anthology (CD 1)"), "Anthology");
    }

    #[test]
    fn test_clean_title_preserves_regular_brackets() {
        // A bracket that is NOT a format tag should be kept
        assert_eq!(
            clean_album_title("Guardians of the Galaxy (Deluxe)"),
            "Guardians of the Galaxy (Deluxe)"
        );
    }

    #[test]
    fn test_clean_title_no_change_for_plain_title() {
        assert_eq!(
            clean_album_title("The Dark Side of the Moon"),
            "The Dark Side of the Moon"
        );
    }

    #[test]
    fn test_clean_title_strips_flac_tag() {
        assert_eq!(clean_album_title("Rumours [FLAC]"), "Rumours");
    }

    // ── clean_artist_for_search ──────────────────────────────────────────────

    #[test]
    fn test_clean_artist_single() {
        assert_eq!(clean_artist_for_search("Pink Floyd"), "Pink Floyd");
    }

    #[test]
    fn test_clean_artist_multi_comma() {
        assert_eq!(clean_artist_for_search("50 Cent, Snoop Dogg"), "50 Cent");
    }

    #[test]
    fn test_clean_artist_multi_with_spaces() {
        assert_eq!(
            clean_artist_for_search("Al Green, Anthony Hamilton"),
            "Al Green"
        );
    }

    #[test]
    fn test_clean_artist_long_list() {
        assert_eq!(
            clean_artist_for_search(
                "Akhenaton, Disiz la Peste, Kool Shen, Lino, Soprano, Taïro, Nessbeal"
            ),
            "Akhenaton"
        );
    }

    // ── sanitize_for_keyword_search ──────────────────────────────────────────

    #[test]
    fn test_sanitize_strips_colon() {
        assert_eq!(
            sanitize_for_keyword_search("Ministry of Sound: The Score"),
            "Ministry of Sound The Score"
        );
    }

    #[test]
    fn test_sanitize_strips_parens_and_colon() {
        assert_eq!(
            sanitize_for_keyword_search("Guardians of the Galaxy: Awesome Mix Vol. 2"),
            "Guardians of the Galaxy Awesome Mix Vol 2"
        );
    }

    #[test]
    fn test_sanitize_collapses_whitespace() {
        assert_eq!(
            sanitize_for_keyword_search("The Dark Side of the Moon"),
            "The Dark Side of the Moon"
        );
    }

    #[test]
    fn test_sanitize_preserves_apostrophe() {
        assert_eq!(
            sanitize_for_keyword_search("Guns N' Roses"),
            "Guns N' Roses"
        );
    }

    // ── RateLimiter ──────────────────────────────────────────────────────────

    #[test]
    fn test_rate_limiter_first_call_no_wait() {
        let mut limiter = RateLimiter::new(1000);
        let wait = limiter.calculate_wait();
        assert!(wait.is_none());
    }

    #[test]
    fn test_rate_limiter_immediate_second_call_waits() {
        let mut limiter = RateLimiter::new(1000);
        limiter.calculate_wait(); // first call
        let wait = limiter.calculate_wait(); // immediate second
        assert!(wait.is_some());
        let wait_ms = wait.unwrap().as_millis();
        assert!(wait_ms > 0 && wait_ms <= 1000);
    }

    #[test]
    fn test_rate_limiter_after_interval_no_wait() {
        let mut limiter = RateLimiter::new(10); // 10ms interval
        limiter.calculate_wait();
        std::thread::sleep(Duration::from_millis(20));
        let wait = limiter.calculate_wait();
        assert!(wait.is_none());
    }

    #[test]
    fn test_rate_limiter_custom_interval() {
        let mut limiter = RateLimiter::new(500);
        assert_eq!(limiter.min_interval_ms, 500);
        limiter.calculate_wait();
        let wait = limiter.calculate_wait();
        assert!(wait.is_some());
        assert!(wait.unwrap().as_millis() <= 500);
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn no_rate_limit_client(mb_url: &str, caa_url: &str, tadb_url: &str) -> MusicBrainzClient {
        MusicBrainzClient::with_base_urls(
            reqwest::Client::new(),
            Arc::new(Mutex::new(RateLimiter::new(0))), // no delay in tests
            mb_url,
            caa_url,
            tadb_url,
        )
    }

    fn mb_releases_json(releases: &[(&str, &str)]) -> serde_json::Value {
        let items: Vec<serde_json::Value> = releases
            .iter()
            .map(|(id, title)| serde_json::json!({ "id": id, "title": title, "score": 100 }))
            .collect();
        serde_json::json!({ "releases": items })
    }

    fn tadb_artists_json(thumbs: &[&str]) -> serde_json::Value {
        if thumbs.is_empty() {
            return serde_json::json!({ "artists": null });
        }
        let items: Vec<serde_json::Value> = thumbs
            .iter()
            .map(|thumb| {
                serde_json::json!({
                    "idArtist": "111258",
                    "strArtistThumb": thumb,
                    "strBiographyEN": null,
                    "strCountry": null,
                    "strGenre": null,
                    "strStyle": null,
                    "strMood": null,
                    "intFormedYear": null,
                    "intBornYear": null,
                    "intDiedYear": null,
                    "strDisbanded": null,
                    "strMusicBrainzID": null
                })
            })
            .collect();
        serde_json::json!({ "artists": items })
    }

    fn tadb_artists_json_with_metadata(thumb: &str) -> serde_json::Value {
        serde_json::json!({
            "artists": [{
                "idArtist": "111258",
                "strArtistThumb": thumb,
                "strBiographyEN": "ABBA was a Swedish pop/rock group...",
                "strCountry": "Stockholm, Sweden",
                "strGenre": "Pop",
                "strStyle": "Rock/Pop",
                "strMood": "Cheerful",
                "intFormedYear": "1970",
                "intBornYear": null,
                "intDiedYear": null,
                "strDisbanded": null,
                "strMusicBrainzID": "d87e52c5-bb8d-4da8-b941-9f4928627dc8"
            }]
        })
    }

    // ── search_album_artwork ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_search_album_artwork_no_releases_returns_none() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/ws/2/release/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mb_releases_json(&[])))
            .mount(&mock_server)
            .await;

        let client =
            no_rate_limit_client(&mock_server.uri(), &mock_server.uri(), &mock_server.uri());
        let result = client
            .search_album_artwork("Unknown Album", "Unknown Artist")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_search_album_artwork_cover_art_found_returns_bytes() {
        let mock_server = MockServer::start().await;
        let fake_image = b"FAKE_JPEG_DATA".to_vec();

        Mock::given(method("GET"))
            .and(path("/ws/2/release/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(mb_releases_json(&[("release-abc", "The Wall")])),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/release/release-abc/front-500"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(fake_image.clone()))
            .mount(&mock_server)
            .await;

        let client =
            no_rate_limit_client(&mock_server.uri(), &mock_server.uri(), &mock_server.uri());
        let result = client
            .search_album_artwork("The Wall", "Pink Floyd")
            .await
            .unwrap();
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.image_data, fake_image);
        assert_eq!(r.musicbrainz_id, "release-abc");
    }

    #[tokio::test]
    async fn test_search_album_artwork_cover_art_404_returns_none() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/ws/2/release/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(mb_releases_json(&[("release-xyz", "Wish You Were Here")])),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/release/release-xyz/front-500"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client =
            no_rate_limit_client(&mock_server.uri(), &mock_server.uri(), &mock_server.uri());
        let result = client
            .search_album_artwork("Wish You Were Here", "Pink Floyd")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_search_album_artwork_mb_server_error_returns_err() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/ws/2/release/"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&mock_server)
            .await;

        let client =
            no_rate_limit_client(&mock_server.uri(), &mock_server.uri(), &mock_server.uri());
        let result = client
            .search_album_artwork("Abbey Road", "The Beatles")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_search_album_artwork_mb_invalid_json_returns_err() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/ws/2/release/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{ not json }"))
            .mount(&mock_server)
            .await;

        let client =
            no_rate_limit_client(&mock_server.uri(), &mock_server.uri(), &mock_server.uri());
        let result = client
            .search_album_artwork("Abbey Road", "The Beatles")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_search_album_artwork_tries_multiple_releases_on_caa_failure() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/ws/2/release/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mb_releases_json(&[
                ("release-bad", "Animals"),
                ("release-good", "Animals"),
            ])))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/release/release-bad/front-500"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let good_image = b"GOOD_IMAGE_DATA".to_vec();
        Mock::given(method("GET"))
            .and(path("/release/release-good/front-500"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(good_image.clone()))
            .mount(&mock_server)
            .await;

        let client =
            no_rate_limit_client(&mock_server.uri(), &mock_server.uri(), &mock_server.uri());
        let result = client
            .search_album_artwork("Animals", "Pink Floyd")
            .await
            .unwrap();
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.image_data, good_image);
        assert_eq!(r.musicbrainz_id, "release-good");
    }

    // ── search_artist_photo (TheAudioDB) ─────────────────────────────────────

    #[tokio::test]
    async fn test_search_artist_photo_not_found_returns_none() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/json/2/search.php"))
            .respond_with(ResponseTemplate::new(200).set_body_json(tadb_artists_json(&[])))
            .mount(&mock_server)
            .await;

        let client =
            no_rate_limit_client(&mock_server.uri(), &mock_server.uri(), &mock_server.uri());
        let result = client.search_artist_photo("Unknown Artist").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_search_artist_photo_found_returns_bytes() {
        let mock_server = MockServer::start().await;
        let fake_photo = b"FAKE_ARTIST_PHOTO".to_vec();
        let photo_path = "/images/media/artist/thumb/pinkfloyd.jpg";

        Mock::given(method("GET"))
            .and(path("/api/v1/json/2/search.php"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(tadb_artists_json(&[&format!(
                    "{}{}",
                    mock_server.uri(),
                    photo_path
                )])),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(photo_path))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(fake_photo.clone()))
            .mount(&mock_server)
            .await;

        let client =
            no_rate_limit_client(&mock_server.uri(), &mock_server.uri(), &mock_server.uri());
        let result = client.search_artist_photo("Pink Floyd").await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().image_data, Some(fake_photo));
    }

    #[tokio::test]
    async fn test_search_artist_photo_thumb_404_returns_some_without_image() {
        // When the artist is found but the thumbnail URL returns 404,
        // we still return Some(result) with the metadata (image_data = None).
        let mock_server = MockServer::start().await;
        let photo_path = "/images/media/artist/thumb/artist.jpg";

        Mock::given(method("GET"))
            .and(path("/api/v1/json/2/search.php"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(tadb_artists_json(&[&format!(
                    "{}{}",
                    mock_server.uri(),
                    photo_path
                )])),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(photo_path))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client =
            no_rate_limit_client(&mock_server.uri(), &mock_server.uri(), &mock_server.uri());
        let result = client.search_artist_photo("Some Artist").await.unwrap();
        // Artist was found — result should be Some, but image_data should be None
        assert!(result.is_some());
        assert!(result.unwrap().image_data.is_none());
    }

    #[tokio::test]
    async fn test_search_artist_photo_captures_metadata() {
        let mock_server = MockServer::start().await;
        let photo_path = "/images/media/artist/thumb/abba.jpg";
        let fake_photo = b"FAKE_ABBA_PHOTO".to_vec();

        Mock::given(method("GET"))
            .and(path("/api/v1/json/2/search.php"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                tadb_artists_json_with_metadata(&format!("{}{}", mock_server.uri(), photo_path)),
            ))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(photo_path))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(fake_photo.clone()))
            .mount(&mock_server)
            .await;

        let client =
            no_rate_limit_client(&mock_server.uri(), &mock_server.uri(), &mock_server.uri());
        let result = client.search_artist_photo("ABBA").await.unwrap();
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.image_data, Some(fake_photo));
        assert_eq!(
            r.bio.as_deref(),
            Some("ABBA was a Swedish pop/rock group...")
        );
        assert_eq!(r.country.as_deref(), Some("Stockholm, Sweden"));
        assert_eq!(r.genre.as_deref(), Some("Pop"));
        assert_eq!(r.style.as_deref(), Some("Rock/Pop"));
        assert_eq!(r.mood.as_deref(), Some("Cheerful"));
        assert_eq!(r.formed_year, Some(1970));
        assert_eq!(r.born_year, None);
        assert_eq!(r.died_year, None);
        assert_eq!(r.disbanded, None);
        assert_eq!(
            r.musicbrainz_id.as_deref(),
            Some("d87e52c5-bb8d-4da8-b941-9f4928627dc8")
        );
        assert_eq!(r.theaudiodb_id.as_deref(), Some("111258"));
    }

    #[tokio::test]
    async fn test_search_artist_photo_server_error_returns_err() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/json/2/search.php"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let client =
            no_rate_limit_client(&mock_server.uri(), &mock_server.uri(), &mock_server.uri());
        let result = client.search_artist_photo("Pink Floyd").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_search_artist_photo_invalid_json_returns_err() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/json/2/search.php"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{ bad json }"))
            .mount(&mock_server)
            .await;

        let client =
            no_rate_limit_client(&mock_server.uri(), &mock_server.uri(), &mock_server.uri());
        let result = client.search_artist_photo("Pink Floyd").await;
        assert!(result.is_err());
    }

    /// Integration test — calls the real TheAudioDB API.
    /// Run with: cargo test test_real_theaudiodb -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn test_real_theaudiodb_abba() {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("Tornade-Music-Player/1.0 ( contact@tornade.app )")
            .build()
            .unwrap();
        let rate_limiter = Arc::new(Mutex::new(RateLimiter::new(0)));
        let client = MusicBrainzClient::new(http_client, rate_limiter);

        let result = client.search_artist_photo("ABBA").await;
        if let Ok(Some(ref a)) = result {
            println!(
                "image: {:?}",
                a.image_data.as_ref().map(|d| format!("{} bytes", d.len()))
            );
            println!("country: {:?}", a.country);
            println!("genre: {:?}", a.genre);
            println!("bio: {:?}", a.bio.as_deref().map(|s| &s[..50.min(s.len())]));
        } else {
            println!("Result: {:?}", result);
        }
        assert!(result.is_ok(), "API call failed: {:?}", result.err());
        let result = result.unwrap();
        assert!(result.is_some(), "ABBA should be found in TheAudioDB");
        let data = result.unwrap();
        assert!(data.country.is_some(), "ABBA should have a country");
        assert!(data.genre.is_some(), "ABBA should have a genre");
    }

    #[test]
    fn strip_format_brackets_removes_format_tags() {
        assert_eq!(strip_format_brackets("Album [44.1-24 WEB]"), "Album ");
        assert_eq!(strip_format_brackets("Album [FLAC]"), "Album ");
        assert_eq!(
            strip_format_brackets("Album [Deluxe Edition]"),
            "Album [Deluxe Edition]"
        );
    }

    #[test]
    fn clean_album_title_strips_disc_suffix() {
        assert_eq!(clean_album_title("Abbey Road (Disc 1)"), "Abbey Road");
        assert_eq!(clean_album_title("Dark Side (CD 1) [FLAC]"), "Dark Side");
    }
}
