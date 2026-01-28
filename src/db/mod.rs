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

    Ok(())
}
