use serde::Deserialize;
use sqlx::FromRow;
#[derive(Clone, Debug, FromRow)]
pub struct Record {
    pub id: i64,
    pub name: String,
    pub key: String,
    pub franchise_id: Option<i64>,
    pub description: Option<String>,
}
#[derive(Debug, FromRow)]
pub struct Character {
    pub id: i64,
    pub name: String,
    pub franchise: String,
}
#[derive(Debug, Default, Deserialize)]
pub struct Filters {
    #[serde(default, deserialize_with = "optional_id")]
    pub franchise: Option<i64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub person: Option<i64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub association_type: Option<i64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub tag: Option<i64>,
}
#[derive(Debug, FromRow)]
pub struct Association {
    pub person_id: i64,
    pub association_type_id: i64,
    pub person: String,
    pub label: String,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    People,
    Franchises,
    Tags,
    Types,
    Characters,
}
impl Kind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "people" => Some(Self::People),
            "franchises" => Some(Self::Franchises),
            "tags" => Some(Self::Tags),
            "types" => Some(Self::Types),
            "characters" => Some(Self::Characters),
            _ => None,
        }
    }
    pub fn path(self) -> &'static str {
        match self {
            Self::People => "people",
            Self::Franchises => "franchises",
            Self::Tags => "tags",
            Self::Types => "types",
            Self::Characters => "characters",
        }
    }
    pub fn table(self) -> &'static str {
        if self == Self::Types {
            "association_types"
        } else {
            self.path()
        }
    }
}

pub fn optional_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let s = String::deserialize(d)?;
    if s.is_empty() {
        Ok(None)
    } else {
        s.parse().map(Some).map_err(serde::de::Error::custom)
    }
}
