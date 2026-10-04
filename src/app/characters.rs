use crate::{
    app::catalog,
    error::Error,
    models::{Character, Filters, Kind},
};
use sqlx::PgPool;
pub async fn browse(pool: &PgPool, f: &Filters) -> Result<Vec<Character>, Error> {
    for (kind, value) in [
        (Kind::Franchises, f.franchise),
        (Kind::People, f.person),
        (Kind::Types, f.association_type),
        (Kind::Tags, f.tag),
    ] {
        if let Some(v) = value {
            catalog::get(pool, kind, v).await?;
        }
    }
    Ok(sqlx::query_as("SELECT c.id,c.name,f.name AS franchise FROM characters c JOIN franchises f ON f.id=c.franchise_id WHERE ($1::bigint IS NULL OR c.franchise_id=$1) AND (($2::bigint IS NULL AND $3::bigint IS NULL) OR EXISTS (SELECT 1 FROM person_character_associations a WHERE a.character_id=c.id AND ($2::bigint IS NULL OR a.person_id=$2) AND ($3::bigint IS NULL OR a.association_type_id=$3))) AND ($4::bigint IS NULL OR EXISTS (SELECT 1 FROM character_tags t WHERE t.character_id=c.id AND t.tag_id=$4)) ORDER BY lower(c.name),c.name,c.id").bind(f.franchise).bind(f.person).bind(f.association_type).bind(f.tag).fetch_all(pool).await?)
}
