use crate::models::{Association, Character, Filters, Record};
use askama::Template;
#[derive(Template)]
#[template(path = "page.html")]
pub struct Page {
    pub title: String,
    pub mode: String,
    pub kind: String,
    pub id: i64,
    pub name: String,
    pub key: String,
    pub description: String,
    pub franchise_id: i64,
    pub error: String,
    pub chosen_tag: String,
    pub chosen_person: String,
    pub chosen_type: String,
    pub raw_franchise: String,
    pub records: Vec<Record>,
    pub characters: Vec<Character>,
    pub franchises: Vec<Record>,
    pub people: Vec<Record>,
    pub types: Vec<Record>,
    pub tags: Vec<Record>,
    pub memberships: Vec<Record>,
    pub associations: Vec<Association>,
    pub filters: Filters,
}
impl Page {
    pub fn invalid_franchise(&self) -> bool {
        !self.raw_franchise.is_empty()
            && !self
                .franchises
                .iter()
                .any(|r| r.id.to_string() == self.raw_franchise)
    }
    pub fn invalid_tag(&self) -> bool {
        !self.chosen_tag.is_empty()
            && !self
                .tags
                .iter()
                .any(|r| r.id.to_string() == self.chosen_tag)
    }
    pub fn invalid_person(&self) -> bool {
        !self.chosen_person.is_empty()
            && !self
                .people
                .iter()
                .any(|r| r.id.to_string() == self.chosen_person)
    }
    pub fn invalid_type(&self) -> bool {
        !self.chosen_type.is_empty()
            && !self
                .types
                .iter()
                .any(|r| r.id.to_string() == self.chosen_type)
    }
    pub fn new(title: &str, mode: &str, kind: &str) -> Self {
        Self {
            title: title.into(),
            mode: mode.into(),
            kind: kind.into(),
            id: 0,
            name: String::new(),
            key: String::new(),
            description: String::new(),
            franchise_id: 0,
            error: String::new(),
            chosen_tag: String::new(),
            chosen_person: String::new(),
            chosen_type: String::new(),
            raw_franchise: String::new(),
            records: vec![],
            characters: vec![],
            franchises: vec![],
            people: vec![],
            types: vec![],
            tags: vec![],
            memberships: vec![],
            associations: vec![],
            filters: Filters::default(),
        }
    }
}
