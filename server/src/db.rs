//! SQLite pool + migrations.

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

pub async fn connect(db_url: &str) -> anyhow::Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(db_url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
