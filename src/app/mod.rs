pub mod associations;
pub mod catalog;
pub mod characters;
use crate::error::Error;
pub fn text(field: &str, value: &str, max: usize) -> Result<String, Error> {
    let v = value.trim();
    if v.is_empty() || v.chars().count() > max || v.contains('\0') {
        return Err(Error::Validation(vec![(
            field.into(),
            format!("Enter 1–{max} characters."),
        )]));
    }
    Ok(v.into())
}
pub fn id(value: i64) -> Result<i64, Error> {
    if value < 1 {
        Err(Error::Validation(vec![(
            "selection".into(),
            "Choose a valid record.".into(),
        )]))
    } else {
        Ok(value)
    }
}
