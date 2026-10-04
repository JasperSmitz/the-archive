use crate::{
    app::{id, text},
    error::Error,
    models::{Kind, Record},
};
use sqlx::PgPool;
#[derive(Default, Clone)]
pub struct Input {
    pub name: String,
    pub key: String,
    pub description: String,
    pub franchise_id: Option<i64>,
}
fn columns(k: Kind) -> &'static str {
    match k {
        Kind::Types => {
            "id, label AS name, key, NULL::bigint AS franchise_id, NULL::text AS description"
        }
        Kind::Characters => "id, name, ''::text AS key, franchise_id, description",
        _ => "id, name, ''::text AS key, NULL::bigint AS franchise_id, NULL::text AS description",
    }
}
pub async fn list(pool: &PgPool, k: Kind) -> Result<Vec<Record>, Error> {
    let column = if k == Kind::Types { "label" } else { "name" };
    Ok(sqlx::query_as(&format!(
        "SELECT {} FROM {} ORDER BY lower({column}), {column}, id",
        columns(k),
        k.table()
    ))
    .fetch_all(pool)
    .await?)
}
pub async fn get(pool: &PgPool, k: Kind, value: i64) -> Result<Record, Error> {
    id(value)?;
    sqlx::query_as(&format!(
        "SELECT {} FROM {} WHERE id=$1",
        columns(k),
        k.table()
    ))
    .bind(value)
    .fetch_optional(pool)
    .await?
    .ok_or(Error::Missing)
}
pub async fn save(
    pool: &PgPool,
    k: Kind,
    existing: Option<i64>,
    input: &Input,
) -> Result<i64, Error> {
    if let Some(v) = existing {
        get(pool, k, v).await?;
    }
    let name = text("name", &input.name, if k == Kind::Tags { 80 } else { 200 })?;
    let key = input.key.trim();
    if k == Kind::Types
        && existing.is_none()
        && (key.is_empty()
            || key.len() > 64
            || !key.as_bytes()[0].is_ascii_lowercase()
            || !key.split('_').all(|s| {
                !s.is_empty()
                    && s.bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            }))
    {
        return Err(Error::Validation(vec![(
            "key".into(),
            "Use lowercase ASCII snake_case, up to 64 characters, starting with a letter.".into(),
        )]));
    }
    let desc = input.description.trim();
    if desc.chars().count() > 10000 || desc.contains('\0') {
        return Err(Error::Validation(vec![(
            "description".into(),
            "Use at most 10,000 characters.".into(),
        )]));
    }
    let description = if desc.is_empty() { None } else { Some(desc) };
    if k == Kind::Characters {
        let fid = input.franchise_id.ok_or_else(|| {
            Error::Validation(vec![("franchise_id".into(), "Choose a franchise.".into())])
        })?;
        get(pool, Kind::Franchises, fid)
            .await
            .map_err(|e| match e {
                Error::Missing => Error::Validation(vec![(
                    "franchise_id".into(),
                    "Choose an existing franchise.".into(),
                )]),
                e => e,
            })?;
        return Ok(if let Some(v) = existing {
            sqlx::query_scalar("UPDATE characters SET name=$1,franchise_id=$2,description=$3,updated_at=now() WHERE id=$4 RETURNING id").bind(name).bind(fid).bind(description).bind(v).fetch_one(pool).await?
        } else {
            sqlx::query_scalar("INSERT INTO characters(name,franchise_id,description) VALUES($1,$2,$3) RETURNING id").bind(name).bind(fid).bind(description).fetch_one(pool).await?
        });
    }
    let column = if k == Kind::Types { "label" } else { "name" };
    Ok(if let Some(v) = existing {
        sqlx::query_scalar(&format!(
            "UPDATE {} SET {column}=$1,updated_at=now() WHERE id=$2 RETURNING id",
            k.table()
        ))
        .bind(name)
        .bind(v)
        .fetch_one(pool)
        .await?
    } else if k == Kind::Types {
        sqlx::query_scalar("INSERT INTO association_types(key,label) VALUES($1,$2) RETURNING id")
            .bind(key)
            .bind(name)
            .fetch_one(pool)
            .await?
    } else {
        sqlx::query_scalar(&format!(
            "INSERT INTO {}(name) VALUES($1) RETURNING id",
            k.table()
        ))
        .bind(name)
        .fetch_one(pool)
        .await?
    })
}
pub async fn delete(pool: &PgPool, k: Kind, value: i64) -> Result<(), Error> {
    id(value)?;
    let n = sqlx::query(&format!("DELETE FROM {} WHERE id=$1", k.table()))
        .bind(value)
        .execute(pool)
        .await?
        .rows_affected();
    if n == 0 { Err(Error::Missing) } else { Ok(()) }
}
