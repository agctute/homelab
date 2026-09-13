use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;

/// Has this torrent already been routed (successfully or not)?
/// Keyed by qBittorrent's info hash so retries don't reprocess the same content.
pub async fn is_processed(pool: &SqlitePool, hash: &str) -> Result<bool> {
    let row = sqlx::query_as::<_, (String,)>("SELECT hash FROM processed_torrents WHERE hash = ?1")
        .bind(hash)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

pub async fn mark_processed(
    pool: &SqlitePool,
    hash: &str,
    name: &str,
    app: &str,
    status: &str,
    detail: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO processed_torrents (hash, name, app, status, detail, processed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(hash) DO UPDATE SET
             name = excluded.name,
             app = excluded.app,
             status = excluded.status,
             detail = excluded.detail,
             processed_at = excluded.processed_at",
    )
    .bind(hash)
    .bind(name)
    .bind(app)
    .bind(status)
    .bind(detail)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}
