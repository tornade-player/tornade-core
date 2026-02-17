// HTTP clients for artwork fetching

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use serde::Deserialize;

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

    /// Search for album artwork
    pub async fn search_album_artwork(&self, album_title: &str, artist_name: &str) -> Result<Option<Vec<u8>>, String> {
        // Wait for rate limit
        let wait_time = {
            let mut limiter = self.rate_limiter.lock().unwrap_or_else(|e| e.into_inner());
            limiter.calculate_wait()
        };
        if let Some(duration) = wait_time {
            tokio::time::sleep(duration).await;
        }

        // Search for release
        let query = format!("release:\"{}\" AND artist:\"{}\"", album_title, artist_name);
        let url = format!(
            "{}/ws/2/release/?query={}&fmt=json&limit=5",
            self.musicbrainz_base_url,
            urlencoding::encode(&query)
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

        // Try to download artwork from Cover Art Archive
        for release in search_result.releases.iter().take(3) {
            // Wait for rate limit
            let wait_time = {
                let mut limiter = self.rate_limiter.lock().unwrap_or_else(|e| e.into_inner());
                limiter.calculate_wait()
            };
            if let Some(duration) = wait_time {
                tokio::time::sleep(duration).await;
            }

            let artwork_url = format!("{}/release/{}/front-500", self.coverart_base_url, release.id);

            log::debug!("Trying Cover Art Archive: {}", artwork_url);

            match self.http_client
                .get(&artwork_url)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    // Limit image size to 5 MB to prevent memory issues
                    const MAX_IMAGE_SIZE: usize = 5 * 1024 * 1024;

                    match resp.bytes().await {
                        Ok(bytes) if bytes.len() <= MAX_IMAGE_SIZE => {
                            log::info!("Found artwork for {} - {} ({} KB)", artist_name, album_title, bytes.len() / 1024);
                            return Ok(Some(bytes.to_vec()));
                        }
                        Ok(bytes) => {
                            log::warn!("Artwork too large ({} MB) for {} - {}, skipping",
                                bytes.len() / 1024 / 1024, artist_name, album_title);
                            continue;
                        }
                        Err(e) => {
                            log::warn!("Failed to download artwork bytes: {}", e);
                            continue;
                        }
                    }
                }
                Ok(resp) => {
                    log::debug!("Cover Art Archive returned status {} for {}", resp.status(), release.id);
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
    title: String,
    score: Option<i32>,
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
        assert_eq!(result, Some(fake_image));
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
        assert_eq!(result, Some(good_image));
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
