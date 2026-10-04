//! Browser form decoding lives here; application inputs remain HTTP-independent.
use crate::{app::catalog::Input, error::Error};
#[derive(Default, serde::Deserialize)]
pub struct EntityForm {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub franchise_id: String,
}
impl EntityForm {
    pub fn input(&self) -> Result<Input, Error> {
        Ok(Input {
            name: self.name.clone(),
            key: self.key.clone(),
            description: self.description.clone(),
            franchise_id: optional_id("franchise_id", &self.franchise_id)?,
        })
    }
    pub fn values(&self) -> Input {
        Input {
            name: self.name.clone(),
            key: self.key.clone(),
            description: self.description.clone(),
            franchise_id: self.franchise_id.parse().ok(),
        }
    }
}
pub fn optional_id(field: &str, value: &str) -> Result<Option<i64>, Error> {
    if value.is_empty() {
        Ok(None)
    } else {
        value
            .parse::<i64>()
            .ok()
            .filter(|v| *v > 0)
            .map(Some)
            .ok_or_else(|| Error::Validation(vec![(field.into(), "Choose a valid record.".into())]))
    }
}
pub fn required_id(field: &str, value: &str) -> Result<i64, Error> {
    optional_id(field, value)?
        .ok_or_else(|| Error::Validation(vec![(field.into(), "Choose a record.".into())]))
}
#[derive(Default, serde::Deserialize)]
pub struct Membership {
    #[serde(default)]
    pub tag_id: String,
    #[serde(default)]
    pub person_id: String,
    #[serde(default)]
    pub association_type_id: String,
    #[serde(default)]
    pub remove: bool,
}
