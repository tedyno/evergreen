//! SQLite pool + migrations.

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

pub async fn connect(db_url: &str) -> anyhow::Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(db_url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    // The DB holds (encrypted) Apple credentials + session — owner-only, so other
    // local accounts can't even read the ciphertext.
    #[cfg(unix)]
    if let Some(path) = db_url.strip_prefix("sqlite://").map(|s| s.split('?').next().unwrap_or(s)) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(pool)
}
