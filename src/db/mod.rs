// Database layer

pub mod schema;
pub mod migrations;
pub mod queries;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Result;
use std::path::PathBuf;

pub type DbPool = Pool<SqliteConnectionManager>;

/// Initialize database connection pool
pub fn create_pool(db_path: PathBuf) -> Result<DbPool, r2d2::Error> {
    let manager = SqliteConnectionManager::file(db_path);
    let pool = Pool::builder()
        .max_size(5)
        .build(manager)?;

    Ok(pool)
}

/// Initialize database schema and FTS
pub fn initialize_database(pool: &DbPool) -> Result<()> {
    let conn = pool.get().map_err(|e| {
        rusqlite::Error::InvalidPath(PathBuf::from(format!("Pool error: {}", e)))
    })?;

    schema::initialize_schema(&conn)?;
    schema::initialize_fts(&conn)?;
    schema::initialize_fts_triggers(&conn)?;

    // Run database migrations
    migrations::run_migrations(&conn)?;

    Ok(())
}

/// Reset database by dropping all tables and recreating schema
pub fn reset_database(pool: &DbPool) -> Result<()> {
    let conn = pool.get().map_err(|e| {
        rusqlite::Error::InvalidPath(PathBuf::from(format!("Pool error: {}", e)))
    })?;

    // Drop all tables in reverse order of dependencies
    conn.execute_batch(
        "DROP TABLE IF EXISTS tracks_fts;
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
         DROP TRIGGER IF EXISTS tracks_au;"
    )?;

    // Recreate schema
    schema::initialize_schema(&conn)?;
    schema::initialize_fts(&conn)?;
    schema::initialize_fts_triggers(&conn)?;

    Ok(())
}
