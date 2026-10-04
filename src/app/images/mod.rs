pub mod validation;
use crate::{
    app::{catalog, id, text},
    error::Error,
    models::{
        Character, Kind,
        images::{Gallery, Image},
    },
    storage::{LocalStorage, ORPHAN_GRACE, Orphan},
};
use sqlx::PgPool;
use std::collections::{BTreeSet, HashSet};
use tokio::sync::Semaphore;
use validation::{Limits, Validated};
pub const PAGE_SIZE: i64 = 24;
static DECODING: Semaphore = Semaphore::const_new(2);
const COLUMNS: &str = "i.id,i.storage_key,i.original_filename,i.content_type,i.byte_size,i.sha256,i.width,i.height,i.uploaded_by_person_id,p.name AS uploader_name,i.source_url,i.artist,i.created_at,i.updated_at";
#[derive(Clone, Default, Debug)]
pub struct Metadata {
    pub uploader_id: i64,
    pub source_url: String,
    pub artist: String,
    pub character_ids: Vec<i64>,
}
struct Normalized {
    uploader: i64,
    source: Option<String>,
    artist: Option<String>,
    characters: Vec<i64>,
}
pub struct Upload {
    pub bytes: Vec<u8>,
    pub original_filename: String,
    pub metadata: Metadata,
}
#[derive(Debug)]
pub struct Archived {
    pub id: i64,
    pub duplicate: bool,
}
#[derive(Debug)]
pub struct Deleted {
    pub cleanup_issue: bool,
}
fn field(field: &str, message: &str) -> Error {
    Error::Validation(vec![(field.into(), message.into())])
}
fn optional(field: &str, value: &str, max: usize) -> Result<Option<String>, Error> {
    if value.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(text(field, value, max)?))
    }
}
async fn normalize(pool: &PgPool, m: &Metadata) -> Result<Normalized, Error> {
    let artist = optional("artist", &m.artist, 200)?;
    let source = optional("source_url", &m.source_url, 2048)?;
    if let Some(s) = &source {
        let url = url::Url::parse(s).map_err(|_| {
            field(
                "source_url",
                "Enter an absolute HTTP or HTTPS URL with a host and no credentials.",
            )
        })?;
        let authority = s
            .split_once("://")
            .map(|(_, s)| s.split(['/', '?', '#']).next().unwrap_or(""));
        let invalid_authority = authority.is_none_or(|s| s.is_empty() || s.contains('@'));
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || invalid_authority
            || s.contains('\\')
            || s.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(field(
                "source_url",
                "Enter an absolute HTTP or HTTPS URL with a host and no credentials.",
            ));
        }
    }
    if m.uploader_id < 1
        || sqlx::query_scalar::<_, i64>("SELECT id FROM people WHERE id=$1")
            .bind(m.uploader_id)
            .fetch_optional(pool)
            .await?
            .is_none()
    {
        return Err(field(
            "uploader",
            "Choose an existing person for uploader attribution.",
        ));
    }
    let characters: Vec<i64> = m
        .character_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if characters.iter().any(|v| *v < 1) {
        return Err(field("characters", "Choose existing characters."));
    }
    let found: Vec<i64> = sqlx::query_scalar("SELECT id FROM characters WHERE id=ANY($1)")
        .bind(&characters)
        .fetch_all(pool)
        .await?;
    if found.len() != characters.len() {
        return Err(field(
            "characters",
            "A selected character no longer exists. Review the selections.",
        ));
    }
    Ok(Normalized {
        uploader: m.uploader_id,
        source,
        artist,
        characters,
    })
}
pub async fn get(pool: &PgPool, value: i64) -> Result<Image, Error> {
    id(value)?;
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM images i JOIN people p ON p.id=i.uploaded_by_person_id WHERE i.id=$1"
    ))
    .bind(value)
    .fetch_optional(pool)
    .await?
    .ok_or(Error::Missing)
}
pub async fn memberships(pool: &PgPool, value: i64) -> Result<Vec<Character>, Error> {
    Ok(sqlx::query_as("SELECT c.id,c.name,f.name AS franchise FROM characters c JOIN franchises f ON f.id=c.franchise_id JOIN image_characters ic ON ic.character_id=c.id WHERE ic.image_id=$1 ORDER BY lower(c.name),c.name,c.id").bind(value).fetch_all(pool).await?)
}
pub async fn gallery(
    pool: &PgPool,
    page: i64,
    character_id: Option<i64>,
) -> Result<Gallery, Error> {
    if page < 1 {
        return Err(field("page", "Use a positive page number."));
    }
    let offset = page
        .checked_sub(1)
        .and_then(|v| v.checked_mul(PAGE_SIZE))
        .ok_or_else(|| field("page", "Page number is too large."))?;
    if let Some(c) = character_id {
        catalog::get(pool, Kind::Characters, c).await?;
    }
    let mut rows:Vec<Image>=sqlx::query_as(&format!("SELECT {COLUMNS} FROM images i JOIN people p ON p.id=i.uploaded_by_person_id WHERE ($1::bigint IS NULL OR EXISTS (SELECT 1 FROM image_characters ic WHERE ic.image_id=i.id AND ic.character_id=$1)) ORDER BY i.created_at DESC,i.id DESC LIMIT $2 OFFSET $3")).bind(character_id).bind(PAGE_SIZE+1).bind(offset).fetch_all(pool).await?;
    let has_next = rows.len() > PAGE_SIZE as usize;
    rows.truncate(PAGE_SIZE as usize);
    Ok(Gallery {
        images: rows,
        page,
        has_next,
        character_id,
    })
}
async fn discard(storage: &LocalStorage, key: &str) {
    if let Err(e) = storage.remove(key).await {
        tracing::error!(key,error=%e,"failed to clean up uncommitted image; run orphan cleanup");
    }
}
pub async fn upload(
    pool: &PgPool,
    storage: &LocalStorage,
    input: Upload,
) -> Result<Archived, Error> {
    let normalized = normalize(pool, &input.metadata).await?;
    let filename = validation::filename(&input.original_filename);
    let permit = DECODING
        .acquire()
        .await
        .map_err(|_| field("file", "Image decoding is unavailable."))?;
    let (bytes, validated) = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let validated = validation::inspect(&input.bytes, Limits::default())?;
        Ok::<_, Error>((input.bytes, validated))
    })
    .await
    .map_err(Error::Task)??;
    if let Some(id) = sqlx::query_scalar("SELECT id FROM images WHERE sha256=$1")
        .bind(&validated.sha256)
        .fetch_optional(pool)
        .await?
    {
        return Ok(Archived {
            id,
            duplicate: true,
        });
    }
    let key = storage
        .write(&bytes, validated.extension)
        .await
        .map_err(Error::Storage)?;
    let result = persist(
        pool,
        &key,
        &filename,
        bytes.len() as i64,
        &validated,
        normalized,
    )
    .await;
    match result {
        Ok(Some(id)) => Ok(Archived {
            id,
            duplicate: false,
        }),
        Ok(None) => {
            discard(storage, &key).await;
            let existing = sqlx::query_scalar("SELECT id FROM images WHERE sha256=$1")
                .bind(&validated.sha256)
                .fetch_optional(pool)
                .await?;
            existing
                .map(|id| Archived {
                    id,
                    duplicate: true,
                })
                .ok_or_else(|| {
                    Error::Conflict(
                        "The duplicate image was removed during upload. Try again.".into(),
                    )
                })
        }
        Err(PersistenceError::BeforeCommit(e)) => {
            discard(storage, &key).await;
            Err(e)
        }
        Err(PersistenceError::Commit(e)) => {
            if matches!(e, sqlx::Error::Database(_)) {
                // PostgreSQL explicitly rejected COMMIT; it did not publish this record.
                discard(storage, &key).await;
            } else {
                // A network/protocol failure can occur while COMMIT is still in flight.
                // A second connection seeing no row is NOT proof that it cannot commit later.
                // Retain this file until an offline, grace-period-protected reconciliation.
                tracing::error!(key,error=?e,"ambiguous image commit outcome; retained file; inspect metadata and run orphan cleanup if needed");
            }
            Err(e.into())
        }
    }
}
enum PersistenceError {
    BeforeCommit(Error),
    Commit(sqlx::Error),
}
impl From<sqlx::Error> for PersistenceError {
    fn from(e: sqlx::Error) -> Self {
        Self::BeforeCommit(e.into())
    }
}
async fn persist(
    pool: &PgPool,
    key: &str,
    filename: &str,
    size: i64,
    v: &Validated,
    m: Normalized,
) -> Result<Option<i64>, PersistenceError> {
    let mut tx = pool.begin().await?;
    let value:Option<i64>=sqlx::query_scalar("INSERT INTO images(storage_key,original_filename,content_type,byte_size,sha256,width,height,uploaded_by_person_id,source_url,artist) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT (sha256) DO NOTHING RETURNING id").bind(key).bind(filename).bind(v.content_type).bind(size).bind(&v.sha256).bind(v.width as i32).bind(v.height as i32).bind(m.uploader).bind(m.source).bind(m.artist).fetch_optional(&mut *tx).await?;
    if let Some(id) = value {
        for c in m.characters {
            sqlx::query("INSERT INTO image_characters(image_id,character_id) VALUES($1,$2)")
                .bind(id)
                .bind(c)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await.map_err(PersistenceError::Commit)?;
    } else {
        tx.rollback().await?;
    }
    Ok(value)
}
pub async fn update(pool: &PgPool, value: i64, input: &Metadata) -> Result<(), Error> {
    get(pool, value).await?;
    let m = normalize(pool, input).await?;
    let mut tx = pool.begin().await?;
    let affected=sqlx::query("UPDATE images SET uploaded_by_person_id=$1,source_url=$2,artist=$3,updated_at=now() WHERE id=$4").bind(m.uploader).bind(m.source).bind(m.artist).bind(value).execute(&mut *tx).await?.rows_affected();
    if affected == 0 {
        return Err(Error::Missing);
    }
    sqlx::query("DELETE FROM image_characters WHERE image_id=$1")
        .bind(value)
        .execute(&mut *tx)
        .await?;
    for c in m.characters {
        sqlx::query("INSERT INTO image_characters(image_id,character_id) VALUES($1,$2)")
            .bind(value)
            .bind(c)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
pub async fn delete(pool: &PgPool, storage: &LocalStorage, value: i64) -> Result<Deleted, Error> {
    id(value)?;
    let key: String = sqlx::query_scalar("DELETE FROM images WHERE id=$1 RETURNING storage_key")
        .bind(value)
        .fetch_optional(pool)
        .await?
        .ok_or(Error::Missing)?;
    let cleanup_issue = if let Err(e) = storage.remove(&key).await {
        tracing::error!(key,error=%e,"image record deleted, file removal failed; run orphan cleanup");
        true
    } else {
        false
    };
    Ok(Deleted { cleanup_issue })
}
pub async fn cleanup(
    pool: &PgPool,
    storage: &LocalStorage,
    apply: bool,
) -> Result<Vec<Orphan>, Error> {
    let keys: Vec<String> = sqlx::query_scalar("SELECT storage_key FROM images")
        .fetch_all(pool)
        .await?;
    storage
        .cleanup(
            &keys.into_iter().collect::<HashSet<_>>(),
            apply,
            ORPHAN_GRACE,
        )
        .await
        .map_err(Error::Storage)
}
