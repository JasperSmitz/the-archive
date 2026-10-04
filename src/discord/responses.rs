use super::{client::Attachment, commands::Command, viewer};
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
    json!({"content":truncate(text,1800),"allowed_mentions":{"parse":[]},"embeds":[],"attachments":[],"components":[]})
}
pub fn immediate(text: &str) -> Value {
    let mut data = message(text);
    data["flags"] = json!(64);
    json!({"type":4,"data":data})
}
fn reply(title: &str, description: &str, fields: Vec<Value>) -> Reply {
    // At most ten fields: 10*(64+420)+900+128 = 5,868, under the 6,000 embed total.
    Reply {
        payload: json!({"content":"The Librarian · Archive retrieval","allowed_mentions":{"parse":[]},"attachments":[],"components":[],"embeds":[{"title":plain(title,128),"description":truncate(description,900),"fields":fields.into_iter().take(10).collect::<Vec<_>>()}]}),
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
        let mut result = reply(&character.name, &description, fields);
        if let Some(image) = retrieval::random_image(pool, character.id).await? {
            result.payload["embeds"][0]["fields"]
                .as_array_mut()
                .ok_or(Error::Missing)?
                .extend(image_fields(origin, &image));
            attach(storage, &mut result, &image, cap).await;
        } else {
            result.payload["embeds"][0]["fields"]
                .as_array_mut()
                .ok_or(Error::Missing)?
                .push(field("Artwork", "No associated artwork yet.".into()));
        }
        return Ok(result);
    }
    if command.name == "images" {
        return artwork_reply(
            storage,
            origin,
            retrieval::artwork_view(pool, character.id, None).await?,
            cap,
        )
        .await;
    }
    let image = retrieval::random_image(pool, character.id).await?;
    let mut result = reply(
        "Random artwork",
        &format!(
            "{} — {}\nCharacter gallery: {}\n{}",
            plain(&character.name, 60),
            plain(&character.franchise, 60),
            link(origin, &format!("/images?character={}", character.id)),
            if image.is_some() {
                "One random image from this character's complete archived memberships."
            } else {
                "No archived images on this page."
            }
        ),
        image
            .as_ref()
            .map(|image| image_fields(origin, image))
            .unwrap_or_default(),
    );
    if let Some(image) = image {
        attach(storage, &mut result, &image, cap).await;
    }
    Ok(result)
}
fn image_fields(origin: &str, image: &Image) -> Vec<Value> {
    // Keep attribution/dimensions separate from long protected URLs, so neither
    // disappears merely because a configured hostname approaches its bound.
    vec![
        field(
            &format!("Image #{}", image.id),
            format!(
                "{}\nArtist: {} · {}×{}",
                plain(&image.original_filename, 55),
                plain(image.artist.as_deref().unwrap_or("Not recorded"), 40),
                image.width,
                image.height
            ),
        ),
        field_link(
            "Image detail",
            "Website login required",
            link(origin, &format!("/images/{}", image.id)),
        ),
    ]
}
async fn attach(storage: &LocalStorage, reply: &mut Reply, image: &Image, cap: u64) {
    let notice = match preview(storage, image, cap).await {
        Ok(a) => {
            reply.payload["attachments"] = json!([{"id":0,"filename":a.filename,"description":"Original archived artwork selected by The Librarian"}]);
            reply.payload["embeds"][0]["image"] =
                json!({"url":format!("attachment://{}",a.filename)});
            reply.attachment = Some(a);
            format!("\nAttached preview: image #{} only.", image.id)
        }
        Err(reason) => format!(
            "\nNo preview delivered: {reason}. Selected image #{} retained; use its protected detail link.",
            image.id
        ),
    };
    let text = reply.payload["embeds"][0]["description"]
        .as_str()
        .unwrap_or("");
    reply.payload["embeds"][0]["description"] = json!(format!(
        "{}{notice}",
        truncate(text, 900usize.saturating_sub(notice.encode_utf16().count()))
    ));
}
async fn artwork_reply(
    storage: &LocalStorage,
    origin: &str,
    view: retrieval::ArtworkView,
    cap: u64,
) -> Result<Reply, Error> {
    let position = if view.image.is_some() {
        format!(
            "Image {} of {} · newest first. {}{}Counts can change as the Archive is edited.",
            view.position,
            view.total,
            if !view.previous { "Newest end. " } else { "" },
            if !view.next { "Oldest end. " } else { "" }
        )
    } else {
        "No archived images for this character.".into()
    };
    let mut result = reply(
        "Artwork viewer",
        &format!(
            "{} — {} · #{}\nCharacter gallery: {}\n{position}",
            plain(&view.character.name, 60),
            plain(&view.character.franchise, 60),
            view.character.id,
            link(origin, &format!("/images?character={}", view.character.id))
        ),
        view.image
            .as_ref()
            .map(|image| image_fields(origin, image))
            .unwrap_or_default()
            .into_iter()
            .chain(std::iter::once(field_link(
                "Character record",
                "Full curated record",
                link(origin, &format!("/characters/{}", view.character.id)),
            )))
            .collect(),
    );
    if let Some(image) = view.image {
        result.payload["components"] =
            viewer::controls(view.character.id, image.id, view.previous, view.next);
        attach(storage, &mut result, &image, cap).await;
    }
    Ok(result)
}
pub async fn navigate(
    pool: &PgPool,
    storage: &LocalStorage,
    origin: &str,
    navigation: &viewer::Navigation,
    cap: u64,
) -> Result<Reply, Error> {
    let view = retrieval::artwork_view(
        pool,
        navigation.character,
        Some((navigation.image, navigation.direction)),
    )
    .await?;
    artwork_reply(storage, origin, view, cap).await
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
