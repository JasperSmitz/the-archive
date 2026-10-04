//! Bounded exact retrieval shared by non-browser interfaces. No protocol dependencies.
use crate::{
    app::text,
    error::Error,
    models::{Association, Character, Filters, Kind, images::Image},
};
use sqlx::{FromRow, PgPool};

pub const MAX_PAGE: i64 = 100_000;
pub fn offset(page: i64, size: i64) -> Result<i64, Error> {
    if !(1..=MAX_PAGE).contains(&page) || !(1..=100).contains(&size) {
        return Err(Error::Validation(vec![(
            "page".into(),
            "Use a page from 1 to 100,000.".into(),
        )]));
    }
    (page - 1).checked_mul(size).ok_or(Error::TooLarge)
}
#[derive(Debug, FromRow)]
pub struct Detail {
    pub id: i64,
    pub name: String,
    pub franchise: String,
    pub description: Option<String>,
}
#[derive(Debug)]
pub enum Resolution {
    Found(Detail),
    Unknown,
    Ambiguous { candidates: Vec<Detail>, more: bool },
}
/// Equality, deliberately not LIKE or Rust Unicode folding, agrees with DB uniqueness.
pub async fn exact(pool: &PgPool, kind: Kind, name: &str) -> Result<Option<i64>, Error> {
    if kind == Kind::Characters {
        return Err(Error::Validation(vec![(
            "reference".into(),
            "Character names require franchise-aware resolution.".into(),
        )]));
    }
    let name = text(
        "reference",
        name,
        if kind == Kind::Types { 64 } else { 200 },
    )?;
    let column = if kind == Kind::Types { "key" } else { "name" };
    Ok(sqlx::query_scalar(&format!(
        "SELECT id FROM {} WHERE lower({column})=lower($1)",
        kind.table()
    ))
    .bind(name)
    .fetch_optional(pool)
    .await?)
}
pub async fn resolve(
    pool: &PgPool,
    selector: &str,
    franchise: Option<&str>,
) -> Result<Resolution, Error> {
    let selector = text("character", selector, 200)?;
    let franchise_id = if let Some(name) = franchise {
        match exact(pool, Kind::Franchises, name).await? {
            Some(id) => Some(id),
            None => return Ok(Resolution::Unknown),
        }
    } else {
        None
    };
    let digits = selector
        .strip_prefix('#')
        .filter(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()));
    let id = digits
        .map(|s| {
            s.parse::<i64>().ok().filter(|v| *v > 0).ok_or_else(|| {
                Error::Validation(vec![(
                    "character".into(),
                    "Use a positive Archive ID after #.".into(),
                )])
            })
        })
        .transpose()?;
    let mut candidates: Vec<Detail> = sqlx::query_as("SELECT c.id,c.name,f.name AS franchise,c.description FROM characters c JOIN franchises f ON f.id=c.franchise_id WHERE ($1::bigint IS NULL OR c.franchise_id=$1) AND (($2::bigint IS NOT NULL AND c.id=$2) OR ($2::bigint IS NULL AND lower(c.name)=lower($3))) ORDER BY lower(f.name),f.name,c.id LIMIT 6")
        .bind(franchise_id).bind(id).bind(selector).fetch_all(pool).await?;
    Ok(match candidates.len() {
        0 => Resolution::Unknown,
        1 => Resolution::Found(candidates.remove(0)),
        _ => {
            let more = candidates.len() > 5;
            candidates.truncate(5);
            Resolution::Ambiguous { candidates, more }
        }
    })
}
pub async fn associations(pool: &PgPool, id: i64) -> Result<(Vec<Association>, bool), Error> {
    crate::app::id(id)?;
    let mut rows: Vec<Association> = sqlx::query_as("SELECT a.person_id,a.association_type_id,p.name AS person,t.label FROM person_character_associations a JOIN people p ON p.id=a.person_id JOIN association_types t ON t.id=a.association_type_id WHERE a.character_id=$1 ORDER BY lower(p.name),lower(t.label),p.id,t.id LIMIT 11").bind(id).fetch_all(pool).await?;
    let more = rows.len() > 10;
    rows.truncate(10);
    Ok((rows, more))
}
pub async fn characters(
    pool: &PgPool,
    f: &Filters,
    page: i64,
) -> Result<(Vec<Character>, bool), Error> {
    let offset = offset(page, 10)?;
    for (kind, id) in [
        (Kind::People, f.person),
        (Kind::Types, f.association_type),
        (Kind::Franchises, f.franchise),
    ] {
        if let Some(id) = id {
            crate::app::catalog::get(pool, kind, id).await?;
        }
    }
    let mut rows: Vec<Character> = sqlx::query_as("SELECT c.id,c.name,f.name AS franchise FROM characters c JOIN franchises f ON f.id=c.franchise_id WHERE ($1::bigint IS NULL OR c.franchise_id=$1) AND (($2::bigint IS NULL AND $3::bigint IS NULL) OR EXISTS (SELECT 1 FROM person_character_associations a WHERE a.character_id=c.id AND ($2::bigint IS NULL OR a.person_id=$2) AND ($3::bigint IS NULL OR a.association_type_id=$3))) ORDER BY lower(c.name),c.name,c.id LIMIT 11 OFFSET $4")
        .bind(f.franchise).bind(f.person).bind(f.association_type).bind(offset).fetch_all(pool).await?;
    let more = rows.len() > 10;
    rows.truncate(10);
    Ok((rows, more))
}
const IMAGE_COLUMNS: &str = "i.id,i.storage_key,i.original_filename,i.content_type,i.byte_size,i.sha256,i.width,i.height,i.uploaded_by_person_id,p.name AS uploader_name,i.source_url,i.artist,i.created_at,i.updated_at";
pub async fn image_count(pool: &PgPool, character: i64) -> Result<i64, Error> {
    crate::app::id(character)?;
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM image_characters WHERE character_id=$1")
            .bind(character)
            .fetch_one(pool)
            .await?,
    )
}
pub async fn images(pool: &PgPool, character: i64, page: i64) -> Result<(Vec<Image>, bool), Error> {
    crate::app::id(character)?;
    let offset = offset(page, 5)?;
    let mut rows: Vec<Image> = sqlx::query_as(&format!("SELECT {IMAGE_COLUMNS} FROM images i JOIN people p ON p.id=i.uploaded_by_person_id JOIN image_characters ic ON ic.image_id=i.id WHERE ic.character_id=$1 ORDER BY i.created_at DESC,i.id DESC LIMIT 6 OFFSET $2")).bind(character).bind(offset).fetch_all(pool).await?;
    let more = rows.len() > 5;
    rows.truncate(5);
    Ok((rows, more))
}
pub async fn random_image(pool: &PgPool, character: i64) -> Result<Option<Image>, Error> {
    crate::app::id(character)?;
    Ok(sqlx::query_as(&format!("SELECT {IMAGE_COLUMNS} FROM images i JOIN people p ON p.id=i.uploaded_by_person_id JOIN image_characters ic ON ic.image_id=i.id WHERE ic.character_id=$1 ORDER BY random() LIMIT 1")).bind(character).fetch_optional(pool).await?)
}
