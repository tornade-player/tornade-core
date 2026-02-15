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
}

impl MusicBrainzClient {
    pub fn new(http_client: reqwest::Client, rate_limiter: Arc<Mutex<RateLimiter>>) -> Self {
        Self {
            http_client,
            rate_limiter,
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
            "https://musicbrainz.org/ws/2/release/?query={}&fmt=json&limit=5",
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

            let artwork_url = format!("https://coverartarchive.org/release/{}/front-500", release.id);

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
            "https://musicbrainz.org/ws/2/artist/?query={}&fmt=json&limit=3",
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
}
