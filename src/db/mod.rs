//! SQLite persistence layer: connection pool, schema initialisation, and migrations.
//!
//! The database is a single SQLite file whose location is determined by
//! [`crate::utils::AppPaths::database_path`]. Access is managed through an
//! `r2d2` connection pool (max 3 connections) so services can acquire connections
//! on demand without blocking each other for long.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use tornade_core::db;
//! use tornade_core::utils::AppPaths;
//!
//! let paths = AppPaths::new().expect("app paths");
//! let pool  = db::create_pool(paths.database_path()).expect("pool");
//! db::initialize_database(&pool).expect("schema");
//! ```

pub mod migrations;
pub mod queries;
pub mod schema;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Result;
use std::path::PathBuf;

pub type DbPool = Pool<SqliteConnectionManager>;

/// Convert an r2d2 pool error to a rusqlite error for functions whose return type is `rusqlite::Result`.
fn pool_err(e: &r2d2::Error) -> rusqlite::Error {
    rusqlite::Error::InvalidPath(PathBuf::from(e.to_string()))
}

/// Create an `r2d2` SQLite connection pool for the database at `db_path`.
///
/// Opens (or creates) the file at `db_path` and configures each connection
/// with a 2 MB page-cache limit (`PRAGMA cache_size = -2000`). The pool is
/// capped at 3 concurrent connections.
pub fn create_pool(db_path: PathBuf) -> Result<DbPool, r2d2::Error> {
    // Limit SQLite page cache to 2 MB per connection (default is ~8 MB).
    // With 3 connections: 3 × 2 MB = 6 MB vs the default 3 × 8 MB = 24 MB.
    let manager = SqliteConnectionManager::file(db_path)
        .with_init(|conn| conn.execute_batch("PRAGMA cache_size = -2000;"));
    let pool = Pool::builder().max_size(3).build(manager)?;

    Ok(pool)
}

/// Apply the base schema, FTS5 virtual tables, and all pending migrations.
///
/// Safe to call on an existing database — the schema is created with
/// `CREATE TABLE IF NOT EXISTS` guards and migrations are applied idempotently.
pub fn initialize_database(pool: &DbPool) -> Result<()> {
    let conn = pool.get().map_err(|e| pool_err(&e))?;

    schema::initialize_schema(&conn)?;
    schema::initialize_fts(&conn)?;
    schema::initialize_fts_triggers(&conn)?;

    // Run database migrations
    migrations::run_migrations(&conn)?;

    Ok(())
}

/// Drop all tables and recreate a clean schema.
///
/// **Destructive** — all library data is permanently lost. Intended for use in
/// tests and the "Reset Library" UI action only.
pub fn reset_database(pool: &DbPool) -> Result<()> {
    let conn = pool.get().map_err(|e| pool_err(&e))?;

    // Drop all tables in reverse order of dependencies
    conn.execute_batch(
        "DROP TABLE IF EXISTS tracks_fts;
         DROP TABLE IF EXISTS albums_fts;
         DROP TABLE IF EXISTS artists_fts;
         DROP TABLE IF EXISTS playlist_tracks;
         DROP TABLE IF EXISTS playlists;
         DROP TABLE IF EXISTS track_genres;
         DROP TABLE IF EXISTS tracks;
         DROP TABLE IF EXISTS albums;
         DROP TABLE IF EXISTS genres;
         DROP TABLE IF EXISTS artists;
         DROP TABLE IF EXISTS sources;
         DROP TABLE IF EXISTS app_state;
         DROP TRIGGER IF EXISTS tracks_ai;
         DROP TRIGGER IF EXISTS tracks_ad;
         DROP TRIGGER IF EXISTS tracks_au;
         DROP TRIGGER IF EXISTS albums_ai;
         DROP TRIGGER IF EXISTS albums_ad;
         DROP TRIGGER IF EXISTS albums_au;
         DROP TRIGGER IF EXISTS artists_ai;
         DROP TRIGGER IF EXISTS artists_ad;
         DROP TRIGGER IF EXISTS artists_au;",
    )?;

    // Recreate schema
    schema::initialize_schema(&conn)?;
    schema::initialize_fts(&conn)?;
    schema::initialize_fts_triggers(&conn)?;

    Ok(())
}
