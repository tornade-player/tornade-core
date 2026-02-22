// Tornade Music Player - macOS FLAC Player
// Library exports

// Pedantic lints suppressed for this refactor phase.
// must_use_candidate/return_self_not_must_use: adding #[must_use] to every method is out of scope.
// missing_errors_doc/missing_panics_doc: doc completeness is out of scope for this refactor.
// doc_markdown: backtick style in doc comments is out of scope.
// items_after_statements: local module/use items inside functions are idiomatic in this codebase.
// manual_let_else: requires structural refactoring of error-return patterns.
// wildcard_imports: crate::services::* is intentional in ffi.rs.
// format_push_string: use of push_str(&format!(...)) is intentional in SQL builders.
// too_many_lines/too_many_arguments: large functions exist by design (SQL builders, CLI handlers).
// arc_with_non_send_sync: Arc<Mutex<PlayerService>> is our intentional FFI workaround.
// non_std_lazy_statics: once_cell dependency predates LazyLock stabilisation in this project.
// cast_sign_loss/possible_truncation/possible_wrap/precision_loss: SQLite stores all integers
//   as i64; conversions to u64/u32/usize/f64 are intentional and safe within our domain values.
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::return_self_not_must_use)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::items_after_statements)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::format_push_string)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::arc_with_non_send_sync)]
#![allow(clippy::non_std_lazy_statics)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_precision_loss)]
// unnecessary_debug_formatting: Path/PathBuf {:?} vs .display() — both are acceptable, the
//   refactor to .display() would require breaking uninlined format args in every log statement.
#![allow(clippy::unnecessary_debug_formatting)]
// should_implement_trait: internal `from_str` helpers on enums predate a FromStr impl;
//   renaming or implementing the trait is out of scope for this refactor.
#![allow(clippy::should_implement_trait)]
// map_entry: the clippy suggestion produces a borrow-after-move error for PathBuf values.
#![allow(clippy::map_entry)]
// unchecked_time_subtraction: Instant - Duration subtractions are safe in playback context
//   (durations are measured from actual playback events and cannot exceed elapsed time).
#![allow(clippy::unchecked_time_subtraction)]

pub mod app;
pub mod cli;
pub mod db;
pub mod ffi; // FFI bridge for Swift/Rust interop
pub mod models;
pub mod services;
pub mod utils;

#[cfg(test)]
pub mod test_helpers;
