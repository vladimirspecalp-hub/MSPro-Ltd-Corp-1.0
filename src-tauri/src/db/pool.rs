//! SQLite pool helpers.
//!
//! We intentionally maintain TWO Rust-side pools alongside the JS-side pool
//! that `tauri-plugin-sql` runs:
//!
//!   • `WritePool` — read+write, used by Tauri commands that mutate state
//!     (create_post, send_chat_message). Single shared sqlx pool.
//!
//!   • `ReadonlyPool` — read-only, opened with `SQLITE_OPEN_READ_ONLY`. Used
//!     ONLY by the External Agent gateway's `sql/query` RPC method. Even if
//!     the SQL validator someday misses a forbidden keyword, the connection
//!     itself refuses writes — defence in depth.
//!
//! All three pools share the same `app.db` file. Concurrent access is safe
//! because `tauri-plugin-sql` enables WAL mode by default (one writer +
//! many readers, no lock conflicts in normal use).

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct WritePool(pub SqlitePool);
pub struct ReadonlyPool(pub SqlitePool);

pub async fn open_write_pool(db_path: &Path) -> Result<WritePool, sqlx::Error> {
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(false)
        .read_only(false)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await?;
    Ok(WritePool(pool))
}

pub async fn open_readonly_pool(db_path: &Path) -> Result<ReadonlyPool, sqlx::Error> {
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(false)
        .read_only(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await?;
    Ok(ReadonlyPool(pool))
}
