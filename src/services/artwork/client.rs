// HTTP clients for artwork fetching

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use serde::Deserialize;

/// Audio format/quality keywords that appear in file-tagger bracket annotations
/// but are not part of the actual album title in MusicBrainz.
const FORMAT_KEYWORDS: &[&str] = &[
    "flac", "mp3", "aac", "ogg", "opus", "wav", "aiff", "dsd", "sacd",
    "web", "mqa", "hires", "hi-res", "hdtracks",
    "tidal", "qobuz", "deezer",
    "kbps", "khz",
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
            if after.chars().next().map_or(false, |c| c.is_ascii_digit()) {
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

/// MusicBrainz API client
pub struct MusicBrainzClient {
    http_client: reqwest::Client,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    musicbrainz_base_url: String,
    coverart_base_url: String,
}

impl MusicBrainzClient {
    pub fn new(http_client: reqwest::Client, rate_limiter: Arc<Mutex<RateLimiter>>) -> Self {
        Self {
            http_client,
            rate_limiter,
            musicbrainz_base_url: "https://musicbrainz.org".to_string(),
            coverart_base_url: "https://coverartarchive.org".to_string(),
        }
    }

    /// Constructor with configurable base URLs (for testing)
    #[cfg(test)]
    pub fn with_base_urls(
        http_client: reqwest::Client,
        rate_limiter: Arc<Mutex<RateLimiter>>,
        musicbrainz_base_url: &str,
        coverart_base_url: &str,
    ) -> Self {
        Self {
            http_client,
            rate_limiter,
            musicbrainz_base_url: musicbrainz_base_url.to_string(),
            coverart_base_url: coverart_base_url.to_string(),
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
    pub async fn search_album_artwork(&self, album_title: &str, artist_name: &str) -> Result<Option<ArtworkSearchResult>, String> {
        let clean_title = clean_album_title(album_title);
        let clean_artist = clean_artist_for_search(artist_name);

        let queries = [
            format!("release:\"{}\" AND artist:\"{}\"", clean_title, clean_artist),
            format!("release:\"{}\"", clean_title),
            format!("release:{}", clean_title),
        ];

        for query in &queries {
            match self.try_search_query(query, artist_name, album_title).await? {
                Some(result) => return Ok(Some(result)),
                None => continue,
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
            let mut limiter = self.rate_limiter.lock().unwrap_or_else(|e| e.into_inner());
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

        log::debug!("Searching MusicBrainz: {}", url);

        let response = self.http_client
            .get(&url)
            .header("User-Agent", "Tornade-Music-Player/1.0 ( thomas@example.com )")
            .send()
            .await
            .map_err(|e| format!("MusicBrainz search failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("MusicBrainz returned status: {}", response.status()));
        }

        let search_result: MBSearchResult = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse MusicBrainz response: {}", e))?;

        if search_result.releases.is_empty() {
            return Ok(None);
        }

        // Try all returned releases (up to 5), skipping low-confidence ones
        for release in search_result.releases.iter().take(5) {
            if release.score.unwrap_or(100) < 50 {
                log::debug!("Skipping low-score release {} (score {:?})", release.id, release.score);
                continue;
            }

            let wait_time = {
                let mut limiter = self.rate_limiter.lock().unwrap_or_else(|e| e.into_inner());
                limiter.calculate_wait()
            };
            if let Some(duration) = wait_time {
                tokio::time::sleep(duration).await;
            }

            let artwork_url = format!("{}/release/{}/front-500", self.coverart_base_url, release.id);

            log::debug!("Trying Cover Art Archive: {}", artwork_url);

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
                            let label = release.label_info.as_ref()
                                .and_then(|info| info.first())
                                .and_then(|li| li.label.as_ref())
                                .map(|l| l.name.clone());

                            return Ok(Some(ArtworkSearchResult {
                                image_data: bytes.to_vec(),
                                musicbrainz_id: release.id.clone(),
                                label,
                                country: release.country.clone(),
                                barcode: release.barcode.clone(),
                                album_type: release.release_group.as_ref()
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
                            log::warn!("Failed to download artwork bytes: {}", e);
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

    /// Search for artist photo
    pub async fn search_artist_photo(&self, artist_name: &str) -> Result<Option<Vec<u8>>, String> {
        // Wait for rate limit
        let wait_time = {
            let mut limiter = self.rate_limiter.lock().unwrap_or_else(|e| e.into_inner());
            limiter.calculate_wait()
        };
        if let Some(duration) = wait_time {
            tokio::time::sleep(duration).await;
        }

        // Search for artist
        let query = format!("artist:\"{}\"", artist_name);
        let url = format!(
            "{}/ws/2/artist/?query={}&fmt=json&limit=3",
            self.musicbrainz_base_url,
            urlencoding::encode(&query)
        );

        log::debug!("Searching MusicBrainz for artist: {}", url);

        let response = self.http_client
            .get(&url)
            .header("User-Agent", "Tornade-Music-Player/1.0 ( thomas@example.com )")
            .send()
            .await
            .map_err(|e| format!("MusicBrainz artist search failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("MusicBrainz returned status: {}", response.status()));
        }

        let search_result: MBArtistSearchResult = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse MusicBrainz response: {}", e))?;

        if search_result.artists.is_empty() {
            return Ok(None);
        }

        // MusicBrainz doesn't provide artist photos directly
        // For v1, we'll just return None. v2 can add Spotify/Last.fm integration
        log::debug!("Found artist {} in MusicBrainz, but artist photos not implemented yet", artist_name);
        Ok(None)
    }
}

#[derive(Debug, Deserialize)]
struct MBSearchResult {
    releases: Vec<MBRelease>,
}

#[derive(Debug, Deserialize)]
struct MBRelease {
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
}

#[derive(Debug, Deserialize)]
struct MBReleaseGroup {
    #[serde(rename = "primary-type")]
    primary_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MBLabelInfo {
    label: Option<MBLabel>,
}

#[derive(Debug, Deserialize)]
struct MBLabel {
    name: String,
}

/// Extract a 4-digit year from a MusicBrainz date string ("1999-11-16" → Some(1999)).
fn year_from_mb_date(date: &str) -> Option<u16> {
    date.get(..4)?.parse().ok()
}

#[derive(Debug, Deserialize)]
struct MBArtistSearchResult {
    artists: Vec<MBArtist>,
}

#[derive(Debug, Deserialize)]
struct MBArtist {
    id: String,
    name: String,
    score: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};

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
        assert_eq!(clean_album_title("The Dark Side of the Moon"), "The Dark Side of the Moon");
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

    fn no_rate_limit_client(mb_url: &str, caa_url: &str) -> MusicBrainzClient {
        MusicBrainzClient::with_base_urls(
            reqwest::Client::new(),
            Arc::new(Mutex::new(RateLimiter::new(0))), // no delay in tests
            mb_url,
            caa_url,
        )
    }

    fn mb_releases_json(releases: &[(&str, &str)]) -> serde_json::Value {
        let items: Vec<serde_json::Value> = releases
            .iter()
            .map(|(id, title)| serde_json::json!({ "id": id, "title": title, "score": 100 }))
            .collect();
        serde_json::json!({ "releases": items })
    }

    fn mb_artists_json(artists: &[(&str, &str)]) -> serde_json::Value {
        let items: Vec<serde_json::Value> = artists
            .iter()
            .map(|(id, name)| serde_json::json!({ "id": id, "name": name, "score": 100 }))
            .collect();
        serde_json::json!({ "artists": items })
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

        let client = no_rate_limit_client(&mock_server.uri(), &mock_server.uri());
        let result = client.search_album_artwork("Unknown Album", "Unknown Artist").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_search_album_artwork_cover_art_found_returns_bytes() {
        let mock_server = MockServer::start().await;
        let fake_image = b"FAKE_JPEG_DATA".to_vec();

        Mock::given(method("GET"))
            .and(path("/ws/2/release/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mb_releases_json(&[("release-abc", "The Wall")])))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/release/release-abc/front-500"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(fake_image.clone()))
            .mount(&mock_server)
            .await;

        let client = no_rate_limit_client(&mock_server.uri(), &mock_server.uri());
        let result = client.search_album_artwork("The Wall", "Pink Floyd").await.unwrap();
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
            .respond_with(ResponseTemplate::new(200).set_body_json(mb_releases_json(&[("release-xyz", "Wish You Were Here")])))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/release/release-xyz/front-500"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client = no_rate_limit_client(&mock_server.uri(), &mock_server.uri());
        let result = client.search_album_artwork("Wish You Were Here", "Pink Floyd").await.unwrap();
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

        let client = no_rate_limit_client(&mock_server.uri(), &mock_server.uri());
        let result = client.search_album_artwork("Abbey Road", "The Beatles").await;
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

        let client = no_rate_limit_client(&mock_server.uri(), &mock_server.uri());
        let result = client.search_album_artwork("Abbey Road", "The Beatles").await;
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

        let client = no_rate_limit_client(&mock_server.uri(), &mock_server.uri());
        let result = client.search_album_artwork("Animals", "Pink Floyd").await.unwrap();
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.image_data, good_image);
        assert_eq!(r.musicbrainz_id, "release-good");
    }

    // ── search_artist_photo ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_search_artist_photo_no_artists_returns_none() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/ws/2/artist/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mb_artists_json(&[])))
            .mount(&mock_server)
            .await;

        let client = no_rate_limit_client(&mock_server.uri(), &mock_server.uri());
        let result = client.search_artist_photo("Unknown Artist").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_search_artist_photo_found_in_mb_returns_none() {
        // Artist photos not implemented yet — even if MB finds the artist, returns None
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/ws/2/artist/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mb_artists_json(&[("artist-123", "Pink Floyd")])))
            .mount(&mock_server)
            .await;

        let client = no_rate_limit_client(&mock_server.uri(), &mock_server.uri());
        let result = client.search_artist_photo("Pink Floyd").await.unwrap();
        assert!(result.is_none(), "artist photos not yet implemented — must return None");
    }

    #[tokio::test]
    async fn test_search_artist_photo_mb_server_error_returns_err() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/ws/2/artist/"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let client = no_rate_limit_client(&mock_server.uri(), &mock_server.uri());
        let result = client.search_artist_photo("Pink Floyd").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_search_artist_photo_mb_invalid_json_returns_err() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/ws/2/artist/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{ bad json }"))
            .mount(&mock_server)
            .await;

        let client = no_rate_limit_client(&mock_server.uri(), &mock_server.uri());
        let result = client.search_artist_photo("Pink Floyd").await;
        assert!(result.is_err());
    }
}
