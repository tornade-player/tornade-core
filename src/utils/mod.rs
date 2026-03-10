//! Cross-cutting utility helpers: application paths, state persistence,
//! M3U playlist I/O, and mutex extensions.

pub mod app_state;
pub mod m3u;
pub mod mutex_ext;
pub mod paths;
// TODO: media_keys module depends on old cacao UI - will be handled natively in SwiftUI
// pub mod media_keys;

pub use app_state::{PersistedState, clear_state, load_state, save_state};
pub use mutex_ext::MutexExt;
pub use paths::AppPaths;
// pub use media_keys::{MediaKey, start_media_key_monitoring, stop_media_key_monitoring, handle_media_key};
