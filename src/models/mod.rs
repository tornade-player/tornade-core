//! Domain model types shared across all tornade-core services.
//!
//! Every entity exposed to the Swift GUI is defined here. The module re-exports
//! the most commonly used types so callers can write `use crate::models::Track`
//! without knowing which sub-module the type lives in.
//!
//! ## ID newtypes
//!
//! Each entity has a dedicated newtype ID (e.g. [`TrackId`], [`AlbumId`]) that
//! wraps an `i64` (the underlying SQLite `ROWID` type). Using newtypes prevents
//! accidentally passing an album ID where a track ID is expected at compile time.
//! The inner field is `pub i64` so FFI code can unwrap at the boundary without
//! extra boilerplate.

pub mod album;
pub mod artist;
pub mod genre;
pub mod playlist;
pub mod source;
pub mod track;

pub use album::Album;
pub use artist::Artist;
pub use genre::Genre;
pub use playlist::{Playlist, Queue, RepeatMode};
pub use source::Source;
pub use track::{AudioFormat, InvalidRating, Rating, Track, TrackBuilder};

// ── ID newtypes ───────────────────────────────────────────────────────────────
// Provide compile-time type safety over raw i64 IDs.
// Inner field is `pub i64` so FFI code can unwrap at the boundary.
macro_rules! define_id_newtype {
    ($name:ident) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            serde::Serialize,
            serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub i64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<i64> for $name {
            fn from(v: i64) -> Self {
                Self(v)
            }
        }

        impl From<$name> for i64 {
            fn from(id: $name) -> i64 {
                id.0
            }
        }

        impl rusqlite::types::FromSql for $name {
            fn column_result(
                value: rusqlite::types::ValueRef<'_>,
            ) -> rusqlite::types::FromSqlResult<Self> {
                i64::column_result(value).map(Self)
            }
        }

        impl rusqlite::types::ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                self.0.to_sql()
            }
        }
    };
}

// Each newtype wraps its entity's SQLite ROWID:
//   TrackId    → tracks.id
//   AlbumId    → albums.id
//   ArtistId   → artists.id
//   GenreId    → genres.id
//   PlaylistId → playlists.id
//   SourceId   → sources.id
define_id_newtype!(TrackId);
define_id_newtype!(AlbumId);
define_id_newtype!(ArtistId);
define_id_newtype!(GenreId);
define_id_newtype!(PlaylistId);
define_id_newtype!(SourceId);
