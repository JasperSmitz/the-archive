use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Serialize)]
pub struct OptionSpec {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "type")]
    pub kind: u8,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value: Option<i64>,
}
#[derive(Serialize)]
pub struct Spec {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "type")]
    pub kind: u8,
    pub options: Vec<OptionSpec>,
}
fn string(name: &'static str, required: bool) -> OptionSpec {
    OptionSpec {
        name,
        description: match name {
            "character" => "Exact character name or #ArchiveID",
            "franchise" => "Exact franchise name",
            "person" => "Exact person name",
            _ => "Association lookup key (e.g. kin, favorite)",
        },
        kind: 3,
        required,
        max_length: Some(if name == "association" { 64 } else { 200 }),
        min_value: None,
        max_value: None,
    }
}
fn page() -> OptionSpec {
    OptionSpec {
        name: "page",
        description: "Page number (default 1)",
        kind: 4,
        required: false,
        max_length: None,
        min_value: Some(1),
        max_value: Some(100_000),
    }
}
pub fn manifest() -> Vec<Spec> {
    vec![
        Spec {
            name: "character",
            description: "Retrieve an exact curated character",
            kind: 1,
            options: vec![string("character", true), string("franchise", false)],
        },
        Spec {
            name: "images",
            description: "Browse a character's artwork, five per page",
            kind: 1,
            options: vec![
                string("character", true),
                string("franchise", false),
                page(),
            ],
        },
        Spec {
            name: "random-image",
            description: "Pick one artwork from this character's archive",
            kind: 1,
            options: vec![string("character", true), string("franchise", false)],
        },
        Spec {
            name: "characters",
            description: "Browse characters with exact relational filters",
            kind: 1,
            options: vec![
                string("person", false),
                string("association", false),
                string("franchise", false),
                page(),
            ],
        },
    ]
}
pub struct Command {
    pub name: &'static str,
    pub values: BTreeMap<String, Value>,
    pub page: i64,
}
impl Command {
    pub fn text(&self, name: &str) -> Option<&str> {
        self.values.get(name).and_then(Value::as_str)
    }
}
pub fn parse(data: &Value) -> Result<Command, &'static str> {
    if data.get("type").and_then(Value::as_u64) != Some(1) {
        return Err("Unsupported command type.");
    }
    let manifest = manifest();
    let spec = manifest
        .iter()
        .find(|s| Some(s.name) == data.get("name").and_then(Value::as_str))
        .ok_or("Unsupported command.")?;
    let options = match data.get("options") {
        None => &[][..],
        Some(Value::Array(a)) => a.as_slice(),
        _ => return Err("Invalid options."),
    };
    if options.len() > spec.options.len() {
        return Err("Invalid options.");
    }
    let mut values = BTreeMap::new();
    for option in options {
        let name = option
            .get("name")
            .and_then(Value::as_str)
            .ok_or("Invalid option.")?;
        let expected = spec
            .options
            .iter()
            .find(|s| s.name == name)
            .ok_or("Unsupported option.")?;
        if option.get("type").and_then(Value::as_u64) != Some(u64::from(expected.kind))
            || option.get("options").is_some()
            || option.get("focused").is_some()
        {
            return Err("Invalid option type.");
        }
        let value = option.get("value").ok_or("Missing option value.")?;
        if expected.kind == 3 {
            let s = value.as_str().ok_or("Expected text.")?.trim();
            if s.is_empty()
                || s.chars().count() > usize::from(expected.max_length.unwrap_or(200))
                || s.contains('\0')
            {
                return Err("Use nonempty bounded text options.");
            }
        } else if !value.as_i64().is_some_and(|p| (1..=100_000).contains(&p)) {
            return Err("Use a page from 1 to 100,000.");
        }
        if values.insert(name.into(), value.clone()).is_some() {
            return Err("Repeated option.");
        }
    }
    if spec
        .options
        .iter()
        .any(|o| o.required && !values.contains_key(o.name))
    {
        return Err("Missing required character option.");
    }
    let page = values.get("page").and_then(Value::as_i64).unwrap_or(1);
    Ok(Command {
        name: spec.name,
        values,
        page,
    })
}
