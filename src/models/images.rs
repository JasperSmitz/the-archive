use chrono::{DateTime, Utc};
use sqlx::FromRow;
#[derive(Clone, Debug, FromRow)]
pub struct Image {
    pub id: i64,
    pub storage_key: String,
    pub original_filename: String,
    pub content_type: String,
    pub byte_size: i64,
    pub sha256: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub uploaded_by_person_id: i64,
    pub uploader_name: String,
    pub source_url: Option<String>,
    pub artist: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
impl Image {
    pub fn archived_at(&self) -> String {
        self.created_at.format("%Y-%m-%d %H:%M UTC").to_string()
    }
}
pub struct Gallery {
    pub images: Vec<Image>,
    pub page: i64,
    pub has_next: bool,
    pub character_id: Option<i64>,
}
