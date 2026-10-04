use crate::{app::auth, config::Environment, web::handlers::failure};
use askama::Template;
use axum::{
    body::Body,
    extract::{Form, Query, State as Extract},
    http::{HeaderMap, Method, Request, StatusCode, header},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub const COOKIE: &str = "archive_session";
#[derive(Clone)]
pub struct AuthState {
    pub pool: sqlx::PgPool,
    pub environment: Environment,
    pub limiter: LoginLimiter,
}
#[derive(Clone)]
pub struct LoginLimiter {
    attempts: Arc<Mutex<VecDeque<Instant>>>,
    capacity: usize,
    window: Duration,
}
impl Default for LoginLimiter {
    fn default() -> Self {
        Self::new(20, Duration::from_secs(60))
    }
}
impl LoginLimiter {
    pub fn new(capacity: usize, window: Duration) -> Self {
        Self {
            attempts: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
            window,
        }
    }
    pub fn allow(&self) -> bool {
        self.allow_at(Instant::now())
    }
    pub fn allow_at(&self, now: Instant) -> bool {
        let Ok(mut attempts) = self.attempts.lock() else {
            return false;
        };
        while attempts
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= self.window)
        {
            attempts.pop_front();
        }
        if attempts.len() >= self.capacity {
            return false;
        }
        attempts.push_back(now);
        true
    }
}
/// Reject everything except printable ASCII local paths in known private route families.
/// Percent-encoded path bytes/backslashes are unnecessary for our numeric-ID routes.
pub fn return_target(value: &str) -> Option<String> {
    if value.len() > 2048
        || !value.is_ascii()
        || value
            .bytes()
            .any(|c| c.is_ascii_control() || c == b' ' || c == b'\\')
        || !value.starts_with('/')
        || value.starts_with("//")
        || value.contains('#')
    {
        return None;
    }
    let path = value.split('?').next()?;
    if path.contains('%') || path.contains(':') || path.split('/').any(|p| matches!(p, "." | ".."))
    {
        return None;
    }
    let first = path.split('/').nth(1).unwrap_or("");
    if !matches!(
        first,
        "" | "characters" | "images" | "people" | "franchises" | "tags" | "types"
    ) {
        return None;
    }
    Some(value.into())
}
pub fn token(headers: &HeaderMap) -> Option<&str> {
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        for cookie in value.to_str().ok()?.split(';') {
            let (k, v) = cookie.trim().split_once('=')?;
            if k == COOKIE {
                if found.is_some() {
                    return None;
                }
                found = Some(v);
            }
        }
    }
    found.filter(|v| auth::digest(v).is_some())
}
fn session_cookie(value: &str, environment: Environment, clear: bool) -> String {
    format!(
        "{COOKIE}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}{}",
        if clear { 0 } else { auth::SESSION_SECONDS },
        if environment == Environment::Production {
            "; Secure"
        } else {
            ""
        }
    )
}
pub async fn protect(
    Extract(state): Extract<AuthState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let account = match token(request.headers()) {
        Some(t) => match auth::authenticate(&state.pool, t).await {
            Ok(a) => a,
            Err(e) => return failure(e),
        },
        None => None,
    };
    if let Some(account) = account {
        request.extensions_mut().insert(account);
        return next.run(request).await;
    }
    if !matches!(*request.method(), Method::GET | Method::HEAD)
        || request.uri().path().ends_with("/content")
    {
        return (
            StatusCode::UNAUTHORIZED,
            "Please sign in to access the private archive.",
        )
            .into_response();
    }
    let target = return_target(&request.uri().to_string()).unwrap_or_else(|| "/characters".into());
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("next", &target)
        .finish();
    Redirect::to(&format!("/login?{query}")).into_response()
}
#[derive(Template)]
#[template(path = "login.html")]
struct LoginPage {
    error: String,
    username: String,
    next: String,
}
fn login_page(username: String, next: String, error: String, status: StatusCode) -> Response {
    match (LoginPage {
        error,
        username,
        next,
    })
    .render()
    {
        Ok(s) => (status, Html(s)).into_response(),
        Err(e) => {
            tracing::error!(error=%e,"login template failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
pub async fn login_form(Query(q): Query<HashMap<String, String>>) -> Response {
    let target = q.get("next").map(|v| return_target(v));
    if matches!(target, Some(None)) {
        return login_page(
            String::new(),
            "/characters".into(),
            "Invalid return path. Sign in to open the character catalog.".into(),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }
    login_page(
        String::new(),
        target.flatten().unwrap_or_else(|| "/characters".into()),
        String::new(),
        StatusCode::OK,
    )
}
#[derive(serde::Deserialize)]
pub struct LoginForm {
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    next: String,
}
pub async fn login(
    Extract(s): Extract<AuthState>,
    headers: HeaderMap,
    Form(f): Form<LoginForm>,
) -> Response {
    if !s.limiter.allow() {
        let mut response = login_page(
            f.username,
            "/characters".into(),
            "Too many sign-in attempts. Wait a minute and try again.".into(),
            StatusCode::TOO_MANY_REQUESTS,
        );
        response.headers_mut().insert(
            header::RETRY_AFTER,
            axum::http::HeaderValue::from_static("60"),
        );
        return response;
    }
    let next = if f.next.is_empty() {
        Some("/characters".into())
    } else {
        return_target(&f.next)
    };
    let Some(next) = next else {
        return login_page(
            f.username,
            "/characters".into(),
            "Invalid return path.".into(),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    };
    match auth::login(&s.pool, &f.username, &f.password).await {
        Ok(Some(token_value)) => {
            if let Some(old) = token(&headers)
                && let Err(e) = auth::revoke(&s.pool, old).await
            {
                tracing::warn!(error=?e,"previous login session revocation failed");
            }
            let cookie = session_cookie(&token_value, s.environment, false);
            ([(header::SET_COOKIE, cookie)], Redirect::to(&next)).into_response()
        }
        Ok(None) => login_page(
            f.username,
            next,
            "Invalid username or password.".into(),
            StatusCode::UNAUTHORIZED,
        ),
        Err(e) => failure(e),
    }
}
pub async fn logout(Extract(s): Extract<AuthState>, headers: HeaderMap) -> Response {
    if let Some(t) = token(&headers)
        && let Err(e) = auth::revoke(&s.pool, t).await
    {
        return failure(e);
    }
    (
        [(header::SET_COOKIE, session_cookie("", s.environment, true))],
        Redirect::to("/login"),
    )
        .into_response()
}
/// No private response is reusable by a shared cache, including redirects/errors.
pub async fn cache_policy(request: Request<Body>, next: Next) -> Response {
    let public_css = request.uri().path() == "/static/app.css";
    let content =
        request.uri().path().starts_with("/images/") && request.uri().path().ends_with("/content");
    let mut response = next.run(request).await;
    if !public_css {
        if !(content && response.status().is_success()) {
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("private, no-store"),
            );
        }
        response
            .headers_mut()
            .insert(header::VARY, axum::http::HeaderValue::from_static("Cookie"));
    }
    response
}
