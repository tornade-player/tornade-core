# Changelog

All notable changes to `tornade-core` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions match the `vX.Y.Z` tags pushed to `tornade-gui` that trigger the
[release orchestration workflow](../.github/workflows/orchestrate.yml).

## [Unreleased]

## [1.1.1] – 2026-02-25

### Added
- OSS repo split: tornade-core extracted from the private monorepo into a standalone public repository.
- 3-platform CI (macOS, Linux, Windows) with `cargo test`, `cargo fmt`, `cargo clippy`.
- `CONTRIBUTING.md` and `README.md` for open-source contributors.
- `docs/audio-architecture.md` explaining the CoreAudio buffer-overload fix.

### Changed
- `AppPaths::new()` now uses `directories::BaseDirs` on Windows (replaces `$HOME` lookup).
- `PlayerService::play()` reads the audio file in a background thread to avoid NAS latency blocking the UI.

### Fixed
- `nonisolated deinit` Swift 6 compile error in Xcode CI (macOS runner).
- ALSA dev-header dependency added to Linux CI runners.

## [1.1.0] – 2026-02-20

### Added
- SOLID refactor of the Swift GUI layer (005-swift-solid-refactor): protocol-based DI for all services, `ImageCache`, `AmbientColors`, `PerformanceMonitor`.
- App localisation (006-app-localization): 210 `xcstrings` keys, plural support, `LocalizationCoverageTests`.
- NAS auto-reconnect spec (007-nas-auto-reconnect).

## [1.0.0] – 2026-01-15

### Added
- Initial public release: FLAC/MP3/AAC/M4A/ALAC library scanning.
- SQLite persistence with FTS5 full-text search.
- `PlayerService` with CoreAudio backend (cpal).
- `PlaylistService`, `ArtworkService`, `DuplicateService`, `MetadataService`.
- Swift/Rust FFI bridge via `swift-bridge`.
- Ratatui-based TUI (`tornade-tui`).

[Unreleased]: https://github.com/tornade-player/tornade-core/compare/v1.1.1...HEAD
[1.1.1]: https://github.com/tornade-player/tornade-core/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/tornade-player/tornade-core/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/tornade-player/tornade-core/releases/tag/v1.0.0
