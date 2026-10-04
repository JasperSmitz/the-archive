use super::{client::Attachment, commands::Command};
use crate::{
    app::retrieval::{self, Resolution},
    error::Error,
    models::{Filters, Kind, images::Image},
    storage::LocalStorage,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::io::AsyncReadExt;
pub const FILE_CAP: u64 = 8 * 1024 * 1024;
pub struct Reply {
    pub payload: Value,
    pub attachment: Option<Attachment>,
}
pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut count = 0;
    let mut out = String::new();
    for c in s.chars() {
        if count + c.len_utf16() > max {
            break;
        }
        count += c.len_utf16();
        out.push(c);
    }
    if out.len() < s.len() {
        while out.encode_utf16().count() + 1 > max {
            out.pop();
        }
        out.push('…');
    }
    out
}
/// Curated text is plain text, not Discord formatting or a mention/link instruction.
pub fn plain(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars().take(max) {
        match c {
            '<' => out.push('‹'),
            '>' => out.push('›'),
            '@' => out.push('＠'),
            '\\' | '*' | '_' | '~' | '`' | '|' | '[' | ']' | '(' | ')' | '#' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() && c != '\n' => out.push(' '),
            c => out.push(c),
        }
    }
    out = out.replace("://", ":\u{200b}//");
    let mut bounded = truncate(&out, max);
    if s.chars().count() > max && !bounded.ends_with('…') {
        bounded = truncate(&format!("{bounded}…"), max);
    }
    bounded
}
pub fn message(text: &str) -> Value {
    json!({"content":truncate(text,1800),"allowed_mentions":{"parse":[]},"embeds":[],"attachments":[]})
}
pub fn immediate(text: &str) -> Value {
    let mut data = message(text);
    data["flags"] = json!(64);
    json!({"type":4,"data":data})
}
fn reply(title: &str, description: &str, fields: Vec<Value>) -> Reply {
    // At most ten fields: 10*(64+420)+900+128 = 5,868, under the 6,000 embed total.
    Reply {
        payload: json!({"content":"The Librarian · Archive retrieval","allowed_mentions":{"parse":[]},"attachments":[],"embeds":[{"title":plain(title,128),"description":truncate(description,900),"fields":fields.into_iter().take(10).collect::<Vec<_>>()}]}),
        attachment: None,
    }
}
fn field(name: &str, value: String) -> Value {
    json!({"name":plain(name,64),"value":truncate(&value,420),"inline":false})
}
fn field_link(name: &str, text: &str, url: String) -> Value {
    let available = 419usize.saturating_sub(url.encode_utf16().count());
    field(name, format!("{}\n{url}", truncate(text, available)))
}
fn link(origin: &str, path: &str) -> String {
    format!("<{origin}{path}>")
}
fn error(text: &str) -> Reply {
    Reply {
        payload: message(text),
        attachment: None,
    }
}
async fn filter(
    pool: &PgPool,
    command: &Command,
    name: &str,
    kind: Kind,
) -> Result<Option<i64>, Error> {
    if let Some(value) = command.text(name) {
        let value = if kind == Kind::Types {
            value.trim().to_ascii_lowercase()
        } else {
            value.to_owned()
        };
        retrieval::exact(pool, kind, &value)
            .await?
            .map(Some)
            .ok_or_else(|| {
                Error::Validation(vec![(
                    name.into(),
                    format!("Unknown {name}; use an exact existing name or association key."),
                )])
            })
    } else {
        Ok(None)
    }
}
pub async fn execute(
    pool: &PgPool,
    storage: &LocalStorage,
    origin: &str,
    command: &Command,
    cap: u64,
) -> Result<Reply, Error> {
    if command.name == "characters" {
        let f = Filters {
            person: filter(pool, command, "person", Kind::People).await?,
            association_type: filter(pool, command, "association", Kind::Types).await?,
            franchise: filter(pool, command, "franchise", Kind::Franchises).await?,
            tag: None,
        };
        let (rows, more) = retrieval::characters(pool, &f, command.page).await?;
        let fields = rows
            .iter()
            .map(|c| {
                field(
                    &format!("#{} {}", c.id, c.name),
                    format!(
                        "{}\n{}",
                        plain(&c.franchise, 60),
                        link(origin, &format!("/characters/{}", c.id))
                    ),
                )
            })
            .collect();
        let description = if rows.is_empty() {
            "No characters match these exact filters.".into()
        } else {
            format!(
                "Page {} · {}{}",
                command.page,
                rows.len(),
                if more && command.page < retrieval::MAX_PAGE {
                    format!(
                        " entries. Rerun with the same filters and page:{} for more.",
                        command.page + 1
                    )
                } else {
                    " entries. End of results.".into()
                }
            )
        };
        return Ok(reply("Characters", &description, fields));
    }
    let selector = command.text("character").unwrap_or("");
    let character = match retrieval::resolve(pool, selector, command.text("franchise")).await? {
        Resolution::Found(c) => c,
        Resolution::Unknown => {
            return Ok(error(
                "No character matches that exact selector and franchise. Check the name, franchise, or #ArchiveID.",
            ));
        }
        Resolution::Ambiguous { candidates, more } => {
            let fields = candidates
                .iter()
                .map(|c| {
                    field(
                        &format!("#{} {}", c.id, c.name),
                        format!(
                            "{}\n{}",
                            plain(&c.franchise, 60),
                            link(origin, &format!("/characters/{}", c.id))
                        ),
                    )
                })
                .collect();
            return Ok(reply(
                "Character name is ambiguous",
                if more {
                    "More matches exist. Rerun with franchise or #ArchiveID; these are the first five candidates."
                } else {
                    "Rerun with franchise or #ArchiveID to choose a record."
                },
                fields,
            ));
        }
    };
    if command.name == "character" {
        let (associations, more) = retrieval::associations(pool, character.id).await?;
        let association_text = if associations.is_empty() {
            "No person associations.".into()
        } else {
            let mut text = associations
                .iter()
                .map(|a| format!("{} / {}", plain(&a.person, 45), plain(&a.label, 45)))
                .collect::<Vec<_>>()
                .join("\n");
            if more || text.encode_utf16().count() > 370 {
                text = truncate(&text, 370);
                text.push_str("\nMore/truncated associations; see full record.");
            }
            text
        };
        let description = character
            .description
            .as_deref()
            .map(|d| {
                format!(
                    "{}\nDescription excerpt; see the full record if truncated.",
                    plain(d, 650)
                )
            })
            .unwrap_or_else(|| "No curated description yet.".into());
        let fields = vec![
            field(
                "Franchise and full record",
                format!(
                    "{} · #{}\n{}",
                    plain(&character.franchise, 60),
                    character.id,
                    link(origin, &format!("/characters/{}", character.id))
                ),
            ),
            field("Person / association", association_text),
        ];
        return Ok(reply(&character.name, &description, fields));
    }
    let (images, more, total) = if command.name == "random-image" {
        (
            retrieval::random_image(pool, character.id)
                .await?
                .into_iter()
                .collect::<Vec<_>>(),
            false,
            None,
        )
    } else {
        let (images, more) = retrieval::images(pool, character.id, command.page).await?;
        (
            images,
            more,
            Some(retrieval::image_count(pool, character.id).await?),
        )
    };
    let fields = images
        .iter()
        .map(|i| {
            field_link(
                &format!("Image #{}", i.id),
                &format!(
                    "{}\nArtist: {} · {}×{}",
                    plain(&i.original_filename, 55),
                    plain(i.artist.as_deref().unwrap_or("Not recorded"), 40),
                    i.width,
                    i.height,
                ),
                link(origin, &format!("/images/{}", i.id)),
            )
        })
        .collect();
    let mut description = format!(
        "{} — {}\nCharacter gallery: {}\n",
        plain(&character.name, 60),
        plain(&character.franchise, 60),
        link(origin, &format!("/images?character={}", character.id))
    );
    if images.is_empty() {
        description.push_str("No archived images on this page.");
    } else if let Some(total) = total {
        description.push_str(&format!(
            "Page {} · {total} archived images. ",
            command.page
        ));
        if more && command.page < retrieval::MAX_PAGE {
            description.push_str(&format!(
                "Rerun with the same selector and page:{} for more. ",
                command.page + 1
            ));
        }
    } else {
        description
            .push_str("One random image from this character's complete archived memberships. ");
    }
    let cap = cap.min(FILE_CAP);
    let mut attachment = None;
    let mut reasons = std::collections::BTreeSet::new();
    for image in &images {
        match preview(storage, image, cap).await {
            Ok(a) => {
                description.push_str(&format!("\nAttached preview: image #{} only.", image.id));
                attachment = Some(a);
                break;
            }
            Err(reason) => {
                reasons.insert(reason);
            }
        }
    }
    if !images.is_empty() && attachment.is_none() {
        description.push_str(&format!(
            "\nNo preview delivered: {}. Use the protected detail links.",
            reasons.into_iter().collect::<Vec<_>>().join("; ")
        ));
    }
    let mut result = reply(
        if command.name == "random-image" {
            "Random artwork"
        } else {
            "Artwork"
        },
        &description,
        fields,
    );
    if let Some(a) = attachment {
        result.payload["attachments"] = json!([{"id":0,"filename":a.filename,"description":"Original archived artwork selected by The Librarian"}]);
        result.payload["embeds"][0]["image"] = json!({"url":format!("attachment://{}",a.filename)});
        result.attachment = Some(a);
    }
    Ok(result)
}
/// Both metadata and the actual read are bounded; never read a replaced file unboundedly.
pub async fn preview(
    storage: &LocalStorage,
    image: &Image,
    cap: u64,
) -> Result<Attachment, &'static str> {
    let cap = cap.min(FILE_CAP);
    if image.byte_size <= 0 || image.byte_size as u64 > cap {
        return Err("File exceeds delivery limit");
    }
    let extension = match image.content_type.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => return Err("Unsupported stored type"),
    };
    let bytes = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let file = storage.open(&image.storage_key).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "Stored file is missing"
            } else {
                "Stored file cannot be read"
            }
        })?;
        let mut bytes = Vec::new();
        file.take(cap + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "File unreadable")?;
        if bytes.is_empty() {
            return Err("Stored file is empty");
        }
        if bytes.len() as u64 > cap {
            return Err("File exceeds delivery limit");
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| "File read timed out")??;
    Ok(Attachment {
        bytes,
        filename: format!("archive-image-{}.{}", image.id, extension),
        content_type: image.content_type.clone(),
    })
}
