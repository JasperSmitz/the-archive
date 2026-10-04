use crate::{
    app::catalog,
    error::Error,
    models::{Association, Kind, Record},
};
use sqlx::PgPool;
pub async fn tag(pool: &PgPool, character: i64, tag: i64, remove: bool) -> Result<(), Error> {
    catalog::get(pool, Kind::Characters, character).await?;
    catalog::get(pool, Kind::Tags, tag).await?;
    sqlx::query(if remove {
        "DELETE FROM character_tags WHERE character_id=$1 AND tag_id=$2"
    } else {
        "INSERT INTO character_tags(character_id,tag_id) VALUES($1,$2) ON CONFLICT DO NOTHING"
    })
    .bind(character)
    .bind(tag)
    .execute(pool)
    .await?;
    Ok(())
}
pub async fn associate(
    pool: &PgPool,
    character: i64,
    person: i64,
    kind: i64,
    remove: bool,
) -> Result<(), Error> {
    for (k, v) in [
        (Kind::Characters, character),
        (Kind::People, person),
        (Kind::Types, kind),
    ] {
        catalog::get(pool, k, v).await?;
    }
    sqlx::query(if remove {"DELETE FROM person_character_associations WHERE character_id=$1 AND person_id=$2 AND association_type_id=$3"}else{"INSERT INTO person_character_associations(character_id,person_id,association_type_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING"}).bind(character).bind(person).bind(kind).execute(pool).await?;
    Ok(())
}
pub async fn tags(pool: &PgPool, c: i64) -> Result<Vec<Record>, Error> {
    Ok(sqlx::query_as("SELECT t.id,t.name,''::text AS key,NULL::bigint AS franchise_id,NULL::text AS description FROM tags t JOIN character_tags ct ON ct.tag_id=t.id WHERE ct.character_id=$1 ORDER BY lower(t.name),t.id").bind(c).fetch_all(pool).await?)
}
pub async fn associations(pool: &PgPool, c: i64) -> Result<Vec<Association>, Error> {
    Ok(sqlx::query_as("SELECT a.person_id,a.association_type_id,p.name AS person,t.label FROM person_character_associations a JOIN people p ON p.id=a.person_id JOIN association_types t ON t.id=a.association_type_id WHERE a.character_id=$1 ORDER BY lower(p.name),lower(t.label),p.id,t.id").bind(c).fetch_all(pool).await?)
}
