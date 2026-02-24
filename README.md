# tornade-core

[![CI](https://github.com/tornade-player/tornade-core/actions/workflows/ci.yml/badge.svg)](https://github.com/tornade-player/tornade-core/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

The Rust audio core library powering the [tornade](https://github.com/tornade-player) music player ecosystem. Handles audio playback, library management, metadata extraction, and provides FFI bindings for native UI frontends.

## Features

- **High-fidelity playback** — FLAC, MP3, AAC, and ALAC audio formats via [symphonia](https://github.com/pdeljanov/Symphonia) + [rodio](https://github.com/RustAudio/rodio)
- **Music library management** — SQLite-backed library with fast full-text search
- **Rich metadata** — reads album art, track info, and tags via [lofty](https://github.com/Serial-ATS/Lofty)
- **Swift/C FFI** — ergonomic bindings via [swift-bridge](https://github.com/chinedufn/swift-bridge) for native macOS UI integration
- **Queue and playlist management** — shuffle, repeat, history, and M3U export
- **Cross-platform** — builds on Linux, macOS, and Windows

## Building

### Prerequisites

- Rust 1.75+ (edition 2024)
- On macOS: Xcode Command Line Tools (for swift-bridge header generation)

```bash
git clone https://github.com/tornade-player/tornade-core.git
cd tornade-core
cargo build
cargo test
```

### Release build

```bash
cargo build --release
```

The static library (`libtornade_core.a`) is produced at `target/release/`.

## Using as a Library Dependency

Add to your `Cargo.toml`:

```toml
[dependencies]
tornade-core = { git = "https://github.com/tornade-player/tornade-core", tag = "v0.3.0" }
```

For local development with a cloned copy, use a `[patch]` override:

```toml
[patch."https://github.com/tornade-player/tornade-core"]
tornade-core = { path = "../tornade-core" }
```

## Project Structure

```
tornade-core/
├── src/
│   ├── lib.rs          # Library exports
│   ├── ffi.rs          # FFI bridge functions (Swift/C)
│   ├── db/             # SQLite database layer
│   ├── models/         # Data structures (serde-serializable)
│   ├── services/       # Business logic (playback, library, metadata)
│   └── utils/          # Utilities
├── include/            # Generated C headers (git-ignored, built by build.rs)
├── build.rs            # swift-bridge code generation
└── tests/              # Integration tests
```

## Related Projects

- [tornade-tui](https://github.com/tornade-player/tornade-tui) — Terminal UI frontend (MIT)
- [tornade-gui](https://github.com/tornade-player/tornade-gui) — Native GUI apps for macOS, Windows, Linux (proprietary)

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

MIT — see [LICENSE](LICENSE) for details. This permissive license allows use in proprietary applications.
