// Data structures and entities

pub mod track;
pub mod album;
pub mod artist;
pub mod genre;
pub mod playlist;
pub mod source;

pub use track::{Track, AudioFormat};
pub use album::Album;
pub use artist::Artist;
pub use genre::Genre;
pub use playlist::{Playlist, Queue, RepeatMode};
pub use source::Source;


// Re-export common types
pub type TrackId = i64;
pub type AlbumId = i64;
pub type ArtistId = i64;
pub type GenreId = i64;
pub type PlaylistId = i64;
pub type SourceId = i64;
