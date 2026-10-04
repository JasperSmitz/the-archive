use crate::{
    app::{
        catalog, characters,
        images::{self, Metadata, Upload, validation::MAX_FILE_BYTES},
    },
    error::Error,
    models::{Character, Filters, Kind, Record, images::Image},
    web::{
        State, forms,
        handlers::{error_message, failure},
    },
};
use askama::Template;
use axum::{
    body::Body,
    extract::multipart::MultipartRejection,
    extract::{Form, Multipart, Path, Query, State as Extract},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use std::collections::HashMap;
use tokio_util::io::ReaderStream;
pub const REQUEST_LIMIT: usize = MAX_FILE_BYTES + 64 * 1024;
const FIELD_LIMIT: usize = 16 * 1024;
#[derive(Default, Clone)]
pub struct ImageForm {
    pub uploader: String,
    pub source_url: String,
    pub artist: String,
    pub characters: Vec<String>,
}
impl ImageForm {
    fn set(&mut self, key: &str, value: String) -> Result<(), Error> {
        match key {
            "uploader" => self.uploader = value,
            "source_url" => self.source_url = value,
            "artist" => self.artist = value,
            "characters" => self.characters.push(value),
            _ => return Err(invalid("form", "Unexpected upload field.")),
        }
        Ok(())
    }
    fn metadata(&self) -> Result<Metadata, Error> {
        Ok(Metadata {
            uploader_id: forms::required_id("uploader", &self.uploader)?,
            source_url: self.source_url.clone(),
            artist: self.artist.clone(),
            character_ids: self
                .characters
                .iter()
                .map(|s| forms::required_id("characters", s))
                .collect::<Result<_, _>>()?,
        })
    }
}
fn invalid(field: &str, message: &str) -> Error {
    Error::Validation(vec![(field.into(), message.into())])
}
#[derive(Template)]
#[template(path = "images/gallery.html")]
struct GalleryPage {
    title: String,
    error: String,
    notice: String,
    images: Vec<Image>,
    characters: Vec<Character>,
    selected: String,
    page: i64,
    previous: i64,
    next: i64,
    has_next: bool,
}
#[derive(Template)]
#[template(path = "images/detail.html")]
struct DetailPage {
    title: String,
    error: String,
    notice: String,
    image: Image,
    characters: Vec<Character>,
}
#[derive(Template)]
#[template(path = "images/form.html")]
struct FormPage {
    title: String,
    error: String,
    id: i64,
    values: ImageForm,
    people: Vec<Record>,
    characters: Vec<Character>,
}
#[derive(Template)]
#[template(path = "images/delete.html")]
struct DeletePage {
    title: String,
    error: String,
    image: Image,
}
fn render(page: impl Template, status: StatusCode) -> Response {
    match page.render() {
        Ok(s) => (status, Html(s)).into_response(),
        Err(e) => {
            tracing::error!(error=%e,"image template failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to render image page.",
            )
                .into_response()
        }
    }
}
async fn form(
    s: &State,
    id: i64,
    values: ImageForm,
    error: String,
    status: StatusCode,
) -> Response {
    let options = async {
        let people = catalog::list(&s.pool, Kind::People).await?;
        let characters = characters::browse(&s.pool, &Filters::default()).await?;
        Ok::<_, Error>((people, characters))
    }
    .await;
    match options {
        Ok((people, characters)) => render(
            FormPage {
                title: if id == 0 {
                    "Upload artwork"
                } else {
                    "Edit image metadata"
                }
                .into(),
                error,
                id,
                values,
                people,
                characters,
            },
            status,
        ),
        Err(e) => failure(e),
    }
}
pub async fn gallery(
    Extract(s): Extract<State>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let result=async{
    let page=match q.get("page"){None=>1,Some(p)=>forms::required_id("page",p)?};
    let character_id=forms::optional_id("character",q.get("character").map(String::as_str).unwrap_or(""))?;
    let gallery=images::gallery(&s.pool,page,character_id).await?;
    let next=page.checked_add(1).ok_or_else(||invalid("page","Page number is too large."))?;
    let notice=match q.get("deleted").map(String::as_str){Some("1")=>"Image deleted.",Some("cleanup")=>"Image record deleted. The file could not be removed; run orphan cleanup while uploads are stopped.",_=>""};
    Ok::<_,Error>(GalleryPage{title:"Images".into(),error:String::new(),notice:notice.into(),images:gallery.images,characters:characters::browse(&s.pool,&Filters::default()).await?,selected:character_id.map(|v|v.to_string()).unwrap_or_default(),page,previous:page-1,next,has_next:gallery.has_next})
 }.await;
    match result {
        Ok(p) => render(p, StatusCode::OK),
        Err(e) => failure(e),
    }
}
pub async fn detail(
    Extract(s): Extract<State>,
    Path(raw_id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(id) = raw_id.parse::<i64>().ok().filter(|v| *v > 0) else {
        return failure(Error::Missing);
    };
    let result=async{let image=images::get(&s.pool,id).await?;let characters=images::memberships(&s.pool,id).await?;Ok::<_,Error>(DetailPage{title:image.original_filename.clone(),error:String::new(),notice:if q.get("duplicate").is_some_and(|v|v=="1"){"Already archived: identical bytes are stored here. Existing metadata and character memberships were kept. Use Edit to add or correct them.".into()}else{String::new()},image,characters})}.await;
    match result {
        Ok(p) => render(p, StatusCode::OK),
        Err(e) => failure(e),
    }
}
pub async fn new(Extract(s): Extract<State>) -> Response {
    form(&s, 0, ImageForm::default(), String::new(), StatusCode::OK).await
}
pub async fn edit(Extract(s): Extract<State>, Path(raw_id): Path<String>) -> Response {
    let Some(id) = raw_id.parse::<i64>().ok().filter(|v| *v > 0) else {
        return failure(Error::Missing);
    };
    let result = async {
        let image = images::get(&s.pool, id).await?;
        let characters = images::memberships(&s.pool, id).await?;
        Ok::<_, Error>(ImageForm {
            uploader: image.uploaded_by_person_id.to_string(),
            source_url: image.source_url.unwrap_or_default(),
            artist: image.artist.unwrap_or_default(),
            characters: characters.iter().map(|c| c.id.to_string()).collect(),
        })
    }
    .await;
    match result {
        Ok(values) => form(&s, id, values, String::new(), StatusCode::OK).await,
        Err(e) => failure(e),
    }
}
pub async fn update(
    Extract(s): Extract<State>,
    Path(raw_id): Path<String>,
    Form(fields): Form<Vec<(String, String)>>,
) -> Response {
    let Some(id) = raw_id.parse::<i64>().ok().filter(|v| *v > 0) else {
        return failure(Error::Missing);
    };
    let mut values = ImageForm::default();
    let result = async {
        images::get(&s.pool, id).await?;
        for (k, v) in fields {
            values.set(&k, v)?;
        }
        let m = values.metadata()?;
        images::update(&s.pool, id, &m).await
    }
    .await;
    match result {
        Ok(()) => Redirect::to(&format!("/images/{id}")).into_response(),
        Err(e) => {
            if matches!(e, Error::Missing) {
                return failure(e);
            }
            let (status, message) = error_message(e);
            form(&s, id, values, message, status).await
        }
    }
}
async fn collect(
    multipart: &mut Multipart,
    values: &mut ImageForm,
) -> Result<(Vec<u8>, String), Error> {
    let mut file = None;
    let mut validation = None;
    let mut field_bytes = 0_usize;
    let mut count = 0_usize;
    while let Some(mut part) = multipart.next_field().await.map_err(multipart_error)? {
        count += 1;
        if count > 1024 {
            return Err(invalid("form", "Too many form fields."));
        }
        let name = part.name().unwrap_or("").to_owned();
        let filename = part.file_name().map(str::to_owned);
        if filename.is_some() || name == "file" {
            if file.is_some() {
                validation = Some(invalid("file", "Upload exactly one image at a time."));
            }
            if name != "file" {
                validation = Some(invalid("file", "Use the image file control."));
            }
            let mut bytes = vec![];
            while let Some(chunk) = part.chunk().await.map_err(multipart_error)? {
                if chunk.len() > MAX_FILE_BYTES.saturating_sub(bytes.len()) {
                    return Err(Error::TooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            if file.is_none() {
                file = Some((bytes, filename.unwrap_or_default()));
            }
        } else {
            let mut bytes = vec![];
            while let Some(chunk) = part.chunk().await.map_err(multipart_error)? {
                if chunk.len() > FIELD_LIMIT.saturating_sub(bytes.len())
                    || chunk.len() > (64 * 1024_usize).saturating_sub(field_bytes)
                {
                    return Err(invalid("form", "Form fields are too large."));
                }
                field_bytes += chunk.len();
                bytes.extend_from_slice(&chunk);
            }
            let value = String::from_utf8(bytes)
                .map_err(|_| invalid("form", "Form text must be valid UTF-8."))?;
            if let Err(e) = values.set(&name, value) {
                validation = Some(e);
            }
        }
    }
    if let Some(e) = validation {
        return Err(e);
    }
    file.ok_or_else(|| invalid("file", "Select an image file."))
}
fn multipart_error(e: axum::extract::multipart::MultipartError) -> Error {
    if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
        Error::TooLarge
    } else {
        invalid(
            "form",
            "Unable to read the upload. Select the file again and retry.",
        )
    }
}
pub async fn upload(
    Extract(s): Extract<State>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Response {
    let mut values = ImageForm::default();
    let result = async {
        let mut multipart =
            multipart.map_err(|_| invalid("form", "Use the upload form to submit one image."))?;
        let (bytes, original_filename) = collect(&mut multipart, &mut values).await?;
        let metadata = values.metadata()?;
        images::upload(
            &s.pool,
            &s.storage,
            Upload {
                bytes,
                original_filename,
                metadata,
            },
        )
        .await
    }
    .await;
    match result {
        Ok(a) => Redirect::to(&format!(
            "/images/{}{}",
            a.id,
            if a.duplicate { "?duplicate=1" } else { "" }
        ))
        .into_response(),
        Err(e) => {
            let (status, message) = error_message(e);
            form(
                &s,
                0,
                values,
                format!("{message} Select the image file again before submitting."),
                status,
            )
            .await
        }
    }
}
pub async fn confirm(Extract(s): Extract<State>, Path(raw_id): Path<String>) -> Response {
    let Some(id) = raw_id.parse::<i64>().ok().filter(|v| *v > 0) else {
        return failure(Error::Missing);
    };
    match images::get(&s.pool, id).await {
        Ok(image) => render(
            DeletePage {
                title: "Delete image".into(),
                error: String::new(),
                image,
            },
            StatusCode::OK,
        ),
        Err(e) => failure(e),
    }
}
pub async fn delete(Extract(s): Extract<State>, Path(raw_id): Path<String>) -> Response {
    let Some(id) = raw_id.parse::<i64>().ok().filter(|v| *v > 0) else {
        return failure(Error::Missing);
    };
    match images::delete(&s.pool, &s.storage, id).await {
        Ok(deleted) => Redirect::to(if deleted.cleanup_issue {
            "/images?deleted=cleanup"
        } else {
            "/images?deleted=1"
        })
        .into_response(),
        Err(e) => failure(e),
    }
}
pub async fn content(Extract(s): Extract<State>, Path(raw_id): Path<String>) -> Response {
    let Some(id) = raw_id.parse::<i64>().ok().filter(|v| *v > 0) else {
        return failure(Error::Missing);
    };
    let image = match images::get(&s.pool, id).await {
        Ok(i) => i,
        Err(e) => return failure(e),
    };
    let file = match s.storage.open(&image.storage_key).await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(image_id=id,error=%e,"archived image file unavailable");
            return (StatusCode::INTERNAL_SERVER_ERROR,"The archived image file is unavailable. Its metadata is retained; check local storage or restore the file from backup.").into_response();
        }
    };
    let body = Body::from_stream(ReaderStream::new(file));
    (
        [
            (header::CONTENT_TYPE, image.content_type),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
            (header::CACHE_CONTROL, "private, no-cache".into()),
        ],
        body,
    )
        .into_response()
}
