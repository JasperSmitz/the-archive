#[derive(Debug)]
pub enum Error {
    Validation(Vec<(String, String)>),
    Missing,
    AuthInternal,
    TooLarge,
    Storage(std::io::Error),
    Task(tokio::task::JoinError),
    Conflict(String),
    Unexpected(sqlx::Error),
}
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        if matches!(e, sqlx::Error::RowNotFound) {
            return Self::Missing;
        }
        if let sqlx::Error::Database(d) = &e {
            match d.code().as_deref() {
                Some("23505") => {
                    let field = match d.constraint() {
                        Some("association_types_key_key") => "key",
                        Some("accounts_username_unique") => "username",
                        Some("characters_name_unique") => "name in this franchise",
                        _ => "name",
                    };
                    return Self::Conflict(format!("A record with this {field} already exists."));
                }
                Some("23503") => {
                    if matches!(
                        d.constraint(),
                        Some(
                            "images_uploaded_by_person_id_fkey"
                                | "person_character_associations_person_id_fkey"
                        )
                    ) {
                        return Self::Conflict("This person has character associations or image uploader attributions. Remove the associations and correct uploader attributions or delete those images before deleting the person.".into());
                    }
                    let record = match d.constraint() {
                        Some("characters_franchise_id_fkey") => "franchise",
                        Some("person_character_associations_person_id_fkey") => "person",
                        Some("person_character_associations_association_type_id_fkey") => {
                            "association type"
                        }
                        _ => "selected record",
                    };
                    return Self::Conflict(format!(
                        "The {record} is still in use, or no longer exists. Remove its relationships before deleting it."
                    ));
                }
                Some("23514") => {
                    return Self::Validation(vec![(
                        "form".into(),
                        "A value does not meet the field requirements.".into(),
                    )]);
                }
                _ => {}
            }
        }
        Self::Unexpected(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
