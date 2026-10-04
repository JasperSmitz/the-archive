use crate::{
    app::{
        associations,
        catalog::{self, Input},
        characters,
    },
    error::Error,
    models::{Filters, Kind},
    web::{
        State,
        forms::{self, EntityForm, Membership},
        templates::Page,
    },
};
use askama::Template;
use axum::{
    extract::{Form, Path, Query, State as Extract},
    http::{Method, Request, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
fn render(p: Page, status: StatusCode) -> Response {
    match p.render() {
        Ok(s) => (status, Html(s)).into_response(),
        Err(e) => {
            tracing::error!(error=%e,"template failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
fn failure(e: Error) -> Response {
    let (status, message) = error_message(e);
    let mut p = Page::new("Unable to complete request", "error", "");
    p.error = message;
    render(p, status)
}
fn error_message(e: Error) -> (StatusCode, String) {
    match e {
        Error::Validation(fields) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            fields
                .into_iter()
                .map(|(f, e)| format!("{f}: {e}"))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        Error::Missing => (StatusCode::NOT_FOUND, "Record not found.".into()),
        Error::Conflict(s) => (StatusCode::CONFLICT, s),
        Error::Unexpected(e) => {
            tracing::error!(error=?e,"database operation failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "An unexpected error occurred. Please try again.".into(),
            )
        }
    }
}
fn kind(s: &str) -> Result<Kind, Error> {
    Kind::parse(s).ok_or(Error::Missing)
}
async fn options(s: &State, p: &mut Page) -> Result<(), Error> {
    p.franchises = catalog::list(&s.pool, Kind::Franchises).await?;
    p.people = catalog::list(&s.pool, Kind::People).await?;
    p.types = catalog::list(&s.pool, Kind::Types).await?;
    p.tags = catalog::list(&s.pool, Kind::Tags).await?;
    Ok(())
}
pub async fn home() -> Redirect {
    Redirect::to("/characters")
}
pub async fn list(
    Extract(s): Extract<State>,
    Path(k): Path<String>,
    Query(f): Query<Filters>,
) -> Response {
    let result = async {
        let k = kind(&k)?;
        let mut p = Page::new(k.path(), "list", k.path());
        if k == Kind::Characters {
            p.characters = characters::browse(&s.pool, &f).await?;
            p.filters = f;
            options(&s, &mut p).await?;
        } else {
            p.records = catalog::list(&s.pool, k).await?;
        }
        Ok::<_, Error>(p)
    }
    .await;
    match result {
        Ok(p) => render(p, StatusCode::OK),
        Err(e) => failure(e),
    }
}
async fn form_page(
    s: &State,
    k: Kind,
    id: Option<i64>,
    input: Option<Input>,
) -> Result<Page, Error> {
    let mut p = Page::new(
        if id.is_some() {
            "Edit record"
        } else {
            "Create record"
        },
        "form",
        k.path(),
    );
    p.id = id.unwrap_or(0);
    let input = if let Some(i) = input {
        i
    } else if let Some(id) = id {
        let r = catalog::get(&s.pool, k, id).await?;
        Input {
            name: r.name,
            key: r.key,
            description: r.description.unwrap_or_default(),
            franchise_id: r.franchise_id,
        }
    } else {
        Input::default()
    };
    p.name = input.name;
    p.key = if k == Kind::Types && id.is_some() {
        catalog::get(&s.pool, k, id.unwrap_or(0)).await?.key
    } else {
        input.key
    };
    p.description = input.description;
    p.franchise_id = input.franchise_id.unwrap_or(0);
    options(s, &mut p).await?;
    Ok(p)
}
pub async fn new(Extract(s): Extract<State>, Path(k): Path<String>) -> Response {
    match kind(&k) {
        Ok(k) => match form_page(&s, k, None, None).await {
            Ok(p) => render(p, StatusCode::OK),
            Err(e) => failure(e),
        },
        Err(e) => failure(e),
    }
}
pub async fn edit(Extract(s): Extract<State>, Path((k, id)): Path<(String, i64)>) -> Response {
    match kind(&k) {
        Ok(k) => match form_page(&s, k, Some(id), None).await {
            Ok(p) => render(p, StatusCode::OK),
            Err(e) => failure(e),
        },
        Err(e) => failure(e),
    }
}
async fn save(s: State, k: String, id: Option<i64>, form: EntityForm) -> Response {
    let k = match kind(&k) {
        Ok(k) => k,
        Err(e) => return failure(e),
    };
    let result = match form.input() {
        Ok(input) => catalog::save(&s.pool, k, id, &input).await,
        Err(e) => Err(e),
    };
    match result {
        Ok(v) => Redirect::to(&format!("/{}/{v}", k.path())).into_response(),
        Err(e) => {
            if matches!(e, Error::Missing | Error::Unexpected(_)) {
                return failure(e);
            }
            let (status, message) = error_message(e);
            match form_page(&s, k, id, Some(form.values())).await {
                Ok(mut p) => {
                    p.error = message;
                    p.raw_franchise = form.franchise_id;
                    render(p, status)
                }
                Err(e) => failure(e),
            }
        }
    }
}
pub async fn create(
    Extract(s): Extract<State>,
    Path(k): Path<String>,
    Form(i): Form<EntityForm>,
) -> Response {
    save(s, k, None, i).await
}
pub async fn update(
    Extract(s): Extract<State>,
    Path((k, id)): Path<(String, i64)>,
    Form(i): Form<EntityForm>,
) -> Response {
    save(s, k, Some(id), i).await
}
async fn record_page(s: &State, k: Kind, id: i64, mode: &str) -> Result<Page, Error> {
    let r = catalog::get(&s.pool, k, id).await?;
    let mut p = Page::new(&r.name, mode, k.path());
    p.id = id;
    p.name = r.name;
    p.key = r.key;
    p.description = r.description.unwrap_or_default();
    p.franchise_id = r.franchise_id.unwrap_or(0);
    if k == Kind::Characters {
        options(s, &mut p).await?;
        p.memberships = associations::tags(&s.pool, id).await?;
        p.associations = associations::associations(&s.pool, id).await?;
    }
    Ok(p)
}
async fn show(s: State, k: String, id: i64, mode: &str) -> Response {
    match kind(&k) {
        Ok(k) => match record_page(&s, k, id, mode).await {
            Ok(p) => render(p, StatusCode::OK),
            Err(e) => failure(e),
        },
        Err(e) => failure(e),
    }
}
pub async fn detail(Extract(s): Extract<State>, Path((k, id)): Path<(String, i64)>) -> Response {
    show(s, k, id, "detail").await
}
pub async fn confirm(Extract(s): Extract<State>, Path((k, id)): Path<(String, i64)>) -> Response {
    show(s, k, id, "delete").await
}
pub async fn delete(Extract(s): Extract<State>, Path((k, id)): Path<(String, i64)>) -> Response {
    let k = match kind(&k) {
        Ok(k) => k,
        Err(e) => return failure(e),
    };
    match catalog::delete(&s.pool, k, id).await {
        Ok(()) => Redirect::to(&format!("/{}", k.path())).into_response(),
        Err(e) => {
            let (status, message) = error_message(e);
            match record_page(&s, k, id, "delete").await {
                Ok(mut p) => {
                    p.error = message;
                    render(p, status)
                }
                Err(e) => failure(e),
            }
        }
    }
}
async fn membership(s: State, id: i64, m: Membership, is_tag: bool) -> Response {
    let result = async {
        if is_tag {
            associations::tag(
                &s.pool,
                id,
                forms::required_id("tag_id", &m.tag_id)?,
                m.remove,
            )
            .await
        } else {
            associations::associate(
                &s.pool,
                id,
                forms::required_id("person_id", &m.person_id)?,
                forms::required_id("association_type_id", &m.association_type_id)?,
                m.remove,
            )
            .await
        }
    }
    .await;
    match result {
        Ok(()) => Redirect::to(&format!("/characters/{id}")).into_response(),
        Err(e) => {
            if matches!(e, Error::Missing | Error::Unexpected(_)) {
                return failure(e);
            }
            let (status, message) = error_message(e);
            match record_page(&s, Kind::Characters, id, "detail").await {
                Ok(mut p) => {
                    p.error = message;
                    p.chosen_tag = m.tag_id;
                    p.chosen_person = m.person_id;
                    p.chosen_type = m.association_type_id;
                    render(p, status)
                }
                Err(e) => failure(e),
            }
        }
    }
}
pub async fn tag(
    Extract(s): Extract<State>,
    Path(id): Path<i64>,
    Form(m): Form<Membership>,
) -> Response {
    membership(s, id, m, true).await
}
pub async fn associate(
    Extract(s): Extract<State>,
    Path(id): Path<i64>,
    Form(m): Form<Membership>,
) -> Response {
    membership(s, id, m, false).await
}
pub async fn same_origin(
    Extract(origin): Extract<String>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        let h = request.headers();
        let is_origin = h.contains_key("origin");
        let source = h.get("origin").or_else(|| h.get("referer"));
        let valid = source
            .and_then(|v| v.to_str().ok())
            .and_then(|v| url::Url::parse(v).ok())
            .is_some_and(|u| {
                u.origin().ascii_serialization() == origin
                    && u.username().is_empty()
                    && u.password().is_none()
                    && (!is_origin
                        || (u.path() == "/" && u.query().is_none() && u.fragment().is_none()))
            });
        if !valid {
            return (
                StatusCode::FORBIDDEN,
                "Unsafe requests require a matching Origin or Referer header.",
            )
                .into_response();
        }
    }
    next.run(request).await
}
