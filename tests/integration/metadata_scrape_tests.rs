//! Integration tests for `MusicBrainzClient::search_recording_metadata` and
//! `MusicBrainzClient::search_release_metadata`.
//!
//! All HTTP calls are intercepted by a wiremock `MockServer` started on
//! localhost — no real network access is required.

use std::sync::{Arc, Mutex};

use tornade_core::services::artwork::{MusicBrainzClient, RateLimiter};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── JSON fixtures ─────────────────────────────────────────────────────────────

/// Realistic MusicBrainz recording-search response for "One More Time" by Daft Punk.
const MB_RECORDING_JSON: &str = r#"{
  "recordings": [{
    "id": "abc-123",
    "title": "One More Time",
    "artist-credit": [{"artist": {"name": "Daft Punk"}}],
    "releases": [{
      "id": "r1",
      "title": "Discovery",
      "date": "2001-03-12",
      "media": [{"position": 1, "tracks": [{"title": "One More Time", "number": "1", "position": 1}]}]
    }],
    "genres": [{"name": "Electronic", "count": 5}],
    "score": 98
  }]
}"#;

/// Empty recording-search response.
const MB_RECORDING_EMPTY_JSON: &str = r#"{"recordings": []}"#;

/// Realistic MusicBrainz release-search response for "Discovery" by Daft Punk.
const MB_RELEASE_JSON: &str = r#"{
  "releases": [{
    "id": "r1",
    "title": "Discovery",
    "date": "2001-03-12",
    "score": 95,
    "artist-credit": [{"artist": {"name": "Daft Punk"}}],
    "genres": [{"name": "Electronic", "count": 8}],
    "release-group": {"primary-type": "Album"}
  }]
}"#;

/// Empty release-search response.
const MB_RELEASE_EMPTY_JSON: &str = r#"{"releases": []}"#;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a `MusicBrainzClient` with zero rate-limit delay pointed at the given
/// mock server.
fn client_for(server: &MockServer) -> MusicBrainzClient {
    MusicBrainzClient::with_base_urls(
        reqwest::Client::new(),
        Arc::new(Mutex::new(RateLimiter::new(0))),
        &server.uri(),
        &server.uri(),
        &server.uri(),
    )
}

// ── search_recording_metadata ─────────────────────────────────────────────────

/// Happy path: server returns one recording; all `ScrapeCandidate` fields are
/// mapped correctly from the MB JSON.
#[tokio::test]
async fn test_search_recording_metadata_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ws/2/recording/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(MB_RECORDING_JSON))
        .mount(&mock_server)
        .await;

    let client = client_for(&mock_server);
    let candidates = client
        .search_recording_metadata("One More Time", "Daft Punk")
        .await
        .expect("should succeed");

    assert_eq!(candidates.len(), 1, "expected exactly one candidate");

    let c = &candidates[0];
    assert_eq!(c.musicbrainz_id, "abc-123");
    assert_eq!(c.title, "One More Time");
    assert_eq!(c.artist, "Daft Punk");
    assert_eq!(c.album.as_deref(), Some("Discovery"));
    assert_eq!(c.year, Some(2001));
    assert_eq!(c.genres, vec!["Electronic"]);
    assert_eq!(c.track_number, Some(1));
    assert_eq!(c.score, 98);
}

/// Empty `recordings` array returns `Ok(vec![])`.
#[tokio::test]
async fn test_search_recording_metadata_empty_result() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ws/2/recording/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(MB_RECORDING_EMPTY_JSON))
        .mount(&mock_server)
        .await;

    let client = client_for(&mock_server);
    let candidates = client
        .search_recording_metadata("Unknown Track", "Unknown Artist")
        .await
        .expect("should succeed with empty list");

    assert!(candidates.is_empty(), "expected empty candidate list");
}

/// HTTP 503 from MusicBrainz returns `Err(...)`.
#[tokio::test]
async fn test_search_recording_metadata_server_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ws/2/recording/"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock_server)
        .await;

    let client = client_for(&mock_server);
    let result = client
        .search_recording_metadata("Some Track", "Some Artist")
        .await;

    assert!(result.is_err(), "expected Err on HTTP 503");
}

/// Invalid JSON body returns `Err(...)` (parse failure).
#[tokio::test]
async fn test_search_recording_metadata_invalid_json() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ws/2/recording/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{ not valid json }"))
        .mount(&mock_server)
        .await;

    let client = client_for(&mock_server);
    let result = client
        .search_recording_metadata("Some Track", "Some Artist")
        .await;

    assert!(result.is_err(), "expected Err on malformed JSON");
}

// ── search_release_metadata ───────────────────────────────────────────────────

/// Happy path: server returns one release; all `ScrapeCandidate` fields are
/// mapped correctly from the MB JSON.
#[tokio::test]
async fn test_search_release_metadata_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ws/2/release/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(MB_RELEASE_JSON))
        .mount(&mock_server)
        .await;

    let client = client_for(&mock_server);
    let candidates = client
        .search_release_metadata("Discovery", "Daft Punk")
        .await
        .expect("should succeed");

    assert_eq!(candidates.len(), 1, "expected exactly one candidate");

    let c = &candidates[0];
    assert_eq!(c.musicbrainz_id, "r1");
    assert_eq!(c.title, "Discovery");
    assert_eq!(c.artist, "Daft Punk");
    assert_eq!(c.year, Some(2001));
    assert_eq!(c.genres, vec!["Electronic"]);
    assert!(
        c.has_artwork,
        "release-group present should imply has_artwork"
    );
    assert_eq!(c.score, 95);
    // Release search sets album to None (the release IS the album)
    assert!(c.album.is_none());
}

/// Empty `releases` array returns `Ok(vec![])`.
#[tokio::test]
async fn test_search_release_metadata_empty_result() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ws/2/release/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(MB_RELEASE_EMPTY_JSON))
        .mount(&mock_server)
        .await;

    let client = client_for(&mock_server);
    let candidates = client
        .search_release_metadata("Unknown Album", "Unknown Artist")
        .await
        .expect("should succeed with empty list");

    assert!(candidates.is_empty(), "expected empty candidate list");
}

/// HTTP 500 from MusicBrainz returns `Err(...)`.
#[tokio::test]
async fn test_search_release_metadata_server_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ws/2/release/"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let client = client_for(&mock_server);
    let result = client
        .search_release_metadata("Some Album", "Some Artist")
        .await;

    assert!(result.is_err(), "expected Err on HTTP 500");
}

/// Invalid JSON body returns `Err(...)` (parse failure).
#[tokio::test]
async fn test_search_release_metadata_invalid_json() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ws/2/release/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{ not valid json }"))
        .mount(&mock_server)
        .await;

    let client = client_for(&mock_server);
    let result = client
        .search_release_metadata("Some Album", "Some Artist")
        .await;

    assert!(result.is_err(), "expected Err on malformed JSON");
}

/// Results are sorted by score descending when multiple candidates are returned.
#[tokio::test]
async fn test_search_release_metadata_sorted_by_score_descending() {
    let mock_server = MockServer::start().await;

    let json = r#"{
      "releases": [
        {"id": "low",  "title": "Discovery", "score": 40, "artist-credit": [{"artist": {"name": "Daft Punk"}}]},
        {"id": "high", "title": "Discovery", "score": 95, "artist-credit": [{"artist": {"name": "Daft Punk"}}]},
        {"id": "mid",  "title": "Discovery", "score": 70, "artist-credit": [{"artist": {"name": "Daft Punk"}}]}
      ]
    }"#;

    Mock::given(method("GET"))
        .and(path("/ws/2/release/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(json))
        .mount(&mock_server)
        .await;

    let client = client_for(&mock_server);
    let candidates = client
        .search_release_metadata("Discovery", "Daft Punk")
        .await
        .expect("should succeed");

    assert_eq!(candidates.len(), 3);
    assert_eq!(
        candidates[0].musicbrainz_id, "high",
        "highest score should be first"
    );
    assert_eq!(candidates[1].musicbrainz_id, "mid");
    assert_eq!(candidates[2].musicbrainz_id, "low");
}
