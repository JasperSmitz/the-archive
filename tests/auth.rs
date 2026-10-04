const PASSWORD: &str = "test-only long password 2026";
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use std::time::{Duration, Instant};
use the_archive::{
    app::auth,
    cli,
    config::{self, Environment},
    storage::LocalStorage,
    web,
};
use tower::ServiceExt;
async fn router(pool: &PgPool, environment: Environment) -> (tempfile::TempDir, Router) {
    let d = tempfile::tempdir().unwrap();
    let s = LocalStorage::initialize(d.path()).await.unwrap();
    (
        d,
        web::router(
            pool.clone(),
            if environment == Environment::Production {
                "https://archive.example.test"
            } else {
                "http://127.0.0.1:3000"
            }
            .into(),
            s,
            environment,
        ),
    )
}
async fn request(
    r: &Router,
    method: &str,
    path: &str,
    cookie: &str,
    origin: Option<&str>,
    body: &str,
) -> Response {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/x-www-form-urlencoded");
    if !cookie.is_empty() {
        b = b.header("cookie", cookie);
    }
    if let Some(origin) = origin {
        b = b.header("origin", origin);
    }
    r.clone()
        .oneshot(b.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap()
}
fn login_form(name: &str, password: &str, next: &str) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .append_pair("username", name)
        .append_pair("password", password)
        .append_pair("next", next)
        .finish()
}
async fn html(r: Response) -> String {
    String::from_utf8(r.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
}
#[test]
fn validation_config_return_paths_and_limiter() {
    assert_eq!(auth::username("  Owner.One  ").unwrap(), "owner.one");
    for v in ["ab", "_owner", "a space", "éclair", "a/b"] {
        assert!(auth::username(v).is_err());
    }
    assert!(auth::username(&"a".repeat(65)).is_err());
    assert!(auth::password("short").is_err());
    assert!(auth::password(&"🐱".repeat(256)).is_ok());
    assert!(auth::password(&"🐱".repeat(257)).is_err());
    assert!(auth::password("long password\0input").is_err());
    assert!(auth::password("  long with spaces  ").is_ok());
    assert!(config::parse_environment(None).is_err());
    assert!(config::parse_environment(Some("Production")).is_err());
    assert_eq!(
        config::parse_environment(Some("production")).unwrap(),
        Environment::Production
    );
    for origin in [
        None,
        Some("http://example.test"),
        Some("https://example.test/path"),
        Some("https://user:pass@example.test"),
        Some("https://example.test?x=1"),
        Some("https://example.test#x"),
        Some("https:\\example.test"),
        Some(" https://example.test"),
    ] {
        assert!(config::validate_origin(Environment::Production, origin, "0.0.0.0:3000").is_err());
    }
    assert_eq!(
        config::validate_origin(
            Environment::Production,
            Some("https://example.test:443/"),
            "0.0.0.0:3000"
        )
        .unwrap(),
        "https://example.test"
    );
    for url in [
        "postgres://u:p@host/db",
        "postgres://u:p@host/db?sslmode=disable",
        "postgres://u:p@host/db?sslmode=prefer",
        "not a URL",
        "postgres://u:p@host/db?sslmode=verify-full&channel_binding=require",
    ] {
        assert!(config::validate_database(url, Some(Environment::Production)).is_err());
    }
    assert!(
        config::validate_database(
            "postgres://u:p@host/db?sslmode=verify-full",
            Some(Environment::Production)
        )
        .is_ok()
    );
    assert!(config::parse_maintenance(Some("yes")).is_err());
    for v in [
        "https://evil.test",
        "//evil.test",
        "/\\evil.test",
        "/%2fevil.test",
        "/images/../login",
        "/login",
        "/images\r\n",
        "/images#bad",
        "/%252f%252fevil",
        "/images/./1",
        "///evil",
    ] {
        assert!(web::auth::return_target(v).is_none(), "{v}");
    }
    assert_eq!(
        web::auth::return_target("/images?character=2&page=3").unwrap(),
        "/images?character=2&page=3"
    );
    let l = web::auth::LoginLimiter::new(2, Duration::from_secs(60));
    let now = Instant::now();
    assert!(l.allow_at(now));
    assert!(l.clone().allow_at(now));
    for _ in 0..10000 {
        assert!(!l.allow_at(now));
    }
    assert!(l.allow_at(now + Duration::from_secs(60)));
    assert!(
        cli::parse(&[
            "account".into(),
            "create".into(),
            "owner".into(),
            "plain-password".into()
        ])
        .is_err()
    );
    assert_eq!(
        cli::password_from_reader("  long password  \r\n".as_bytes()).unwrap(),
        "  long password  "
    );
    assert!(cli::password_from_reader("long password\nsecondline".as_bytes()).is_err());
    assert!(cli::password_from_reader(vec![b'a'; 4000].as_slice()).is_err());
}
#[sqlx::test(migrations = "./migrations")]
async fn accounts_and_persisted_sessions(pool: PgPool) {
    let a = auth::create(&pool, " Owner ", PASSWORD).await.unwrap();
    assert_eq!(a.username, "owner");
    assert!(!a.disabled);
    assert!(matches!(
        auth::create(&pool, "OWNER", PASSWORD).await,
        Err(the_archive::error::Error::Conflict(_))
    ));
    assert!(auth::create(&pool, "other", "short").await.is_err());
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM accounts WHERE id=$1")
        .bind(a.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
    assert!(!hash.contains(PASSWORD));
    for name in ["Owner", " owner", "ab", "has space"] {
        assert!(
            sqlx::query("INSERT INTO accounts(username,password_hash) VALUES($1,$2)")
                .bind(name)
                .bind(&hash)
                .execute(&pool)
                .await
                .is_err()
        );
    }
    assert!(
        sqlx::query(
            "INSERT INTO accounts(username,password_hash) VALUES('plain','plaintext-password')"
        )
        .execute(&pool)
        .await
        .is_err()
    );
    assert!(
        sqlx::query("INSERT INTO sessions(token_digest,account_id) VALUES($1,$2)")
            .bind(vec![0_u8; 31])
            .bind(a.id)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("INSERT INTO sessions(token_digest,account_id) VALUES($1,999999)")
            .bind(vec![0_u8; 32])
            .execute(&pool)
            .await
            .is_err()
    );

    assert!(
        auth::login(&pool, "owner", "incorrect long password")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        auth::login(&pool, "unknown", PASSWORD)
            .await
            .unwrap()
            .is_none()
    );
    let token = auth::login(&pool, "OWNER", PASSWORD)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(token.len(), 64);
    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_digest FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, auth::digest(&token).unwrap());
    assert_ne!(stored, token.as_bytes());
    let days: i64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM expires_at-created_at)::bigint FROM sessions",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(days, 604800);
    // A new pool/process needs no signing secret or in-memory session state.
    let second = sqlx::postgres::PgPoolOptions::new()
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    assert_eq!(
        auth::authenticate(&second, &token)
            .await
            .unwrap()
            .unwrap()
            .id,
        a.id
    );
    second.close().await;
    sqlx::query(
        "UPDATE sessions SET created_at=now()-interval '8 days',expires_at=now()-interval '1 day'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(auth::authenticate(&pool, &token).await.unwrap().is_none());
    assert_eq!(auth::cleanup(&pool).await.unwrap(), 1);
    sqlx::query("INSERT INTO sessions(token_digest,account_id,created_at,expires_at) SELECT decode(lpad(to_hex(g),64,'0'),'hex'),$1,now()-interval '8 days',now()-interval '1 day' FROM generate_series(1,150) g").bind(a.id).execute(&pool).await.unwrap();
    assert_eq!(auth::cleanup(&pool).await.unwrap(), 100);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sessions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        50
    );

    let token = auth::login(&pool, "owner", PASSWORD)
        .await
        .unwrap()
        .unwrap();
    auth::revoke(&pool, &token).await.unwrap();
    assert!(auth::authenticate(&pool, &token).await.unwrap().is_none());
    let token = auth::login(&pool, "owner", PASSWORD)
        .await
        .unwrap()
        .unwrap();
    auth::reset(&pool, "owner", "replacement long password")
        .await
        .unwrap();
    assert!(auth::authenticate(&pool, &token).await.unwrap().is_none());
    assert!(
        auth::login(&pool, "owner", PASSWORD)
            .await
            .unwrap()
            .is_none()
    );
    let token = auth::login(&pool, "owner", "replacement long password")
        .await
        .unwrap()
        .unwrap();
    auth::disable(&pool, "owner").await.unwrap();
    assert!(auth::authenticate(&pool, &token).await.unwrap().is_none());
    assert!(
        auth::login(&pool, "owner", "replacement long password")
            .await
            .unwrap()
            .is_none()
    );
    assert!(auth::list(&pool).await.unwrap()[0].disabled);
}
#[sqlx::test(migrations = "./migrations")]
async fn protected_http_login_logout_and_cookies(pool: PgPool) {
    auth::create(&pool, "owner", PASSWORD).await.unwrap();
    let (_d, r) = router(&pool, Environment::Development).await;
    for path in [
        "/",
        "/characters",
        "/characters?person=1",
        "/images",
        "/images/1",
    ] {
        let res = request(&r, "GET", path, "", None, "").await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert!(
            res.headers()["location"]
                .to_str()
                .unwrap()
                .starts_with("/login?")
        );
        assert_eq!(res.headers()["cache-control"], "private, no-store");
    }
    let res = request(&r, "GET", "/images/1/content", "", None, "").await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert!(!html(res).await.contains("<html"));
    for path in ["/people", "/images", "/characters/1/delete", "/logout"] {
        assert_eq!(
            request(
                &r,
                "POST",
                path,
                "",
                Some("http://127.0.0.1:3000"),
                "name=Private"
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM people")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM images")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        html(request(&r, "GET", "/healthz", "", None, "").await).await,
        "ok\n"
    );
    assert_eq!(
        request(&r, "GET", "/static/app.css", "", None, "")
            .await
            .status(),
        StatusCode::OK
    );
    let form = login_form("owner", PASSWORD, "/images");
    for origin in [None, Some("null"), Some("https://evil.test")] {
        assert_eq!(
            request(&r, "POST", "/login", "", origin, &form)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    for name in ["owner", "unknown"] {
        let res = request(
            &r,
            "POST",
            "/login",
            "",
            Some("http://127.0.0.1:3000"),
            &login_form(name, "incorrect long password", "/images"),
        )
        .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let h = html(res).await;
        assert!(h.contains("Invalid username or password."));
        assert!(!h.contains("incorrect long password"));
    }
    assert_eq!(
        request(
            &r,
            "POST",
            "/login",
            "",
            Some("http://127.0.0.1:3000"),
            &login_form("owner", PASSWORD, "//evil.test")
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = request(
        &r,
        "POST",
        "/login",
        "",
        Some("http://127.0.0.1:3000"),
        &form,
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(res.headers()["location"], "/images");
    let set = res.headers()["set-cookie"].to_str().unwrap();
    for flag in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=604800"] {
        assert!(set.contains(flag));
    }
    assert!(!set.contains("Secure") && !set.contains("Domain="));
    let cookie = set.split(';').next().unwrap().to_owned();
    assert_eq!(
        request(&r, "GET", "/characters", &cookie, None, "")
            .await
            .status(),
        StatusCode::OK
    );
    let res = request(&r, "GET", "/logout", &cookie, None, "").await;
    assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        request(
            &r,
            "POST",
            "/logout",
            &cookie,
            Some("https://evil.test"),
            ""
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let res = request(
        &r,
        "POST",
        "/logout",
        &cookie,
        Some("http://127.0.0.1:3000"),
        "",
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert!(
        res.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(
        request(&r, "GET", "/characters", &cookie, None, "")
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    let (_d, prod) = router(&pool, Environment::Production).await;
    let res = request(
        &prod,
        "POST",
        "/login",
        "",
        Some("https://archive.example.test"),
        &form,
    )
    .await;
    assert!(
        res.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("; Secure")
    );
    // New router instance retains the database session, with a fresh limiter only.
    let new_cookie = res.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let (_d, restarted) = router(&pool, Environment::Development).await;
    assert_eq!(
        request(&restarted, "GET", "/images", new_cookie, None, "")
            .await
            .status(),
        StatusCode::OK
    );
    auth::reset(&pool, "owner", "replacement long password")
        .await
        .unwrap();
    assert_eq!(
        request(&restarted, "GET", "/images", new_cookie, None, "")
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
}
#[sqlx::test(migrations = "./migrations")]
async fn login_rate_limit_and_maintenance(pool: PgPool) {
    let (_d, r) = router(&pool, Environment::Development).await;
    // Invalid local targets spend admission slots but avoid expensive password work.
    for _ in 0..20 {
        assert_eq!(
            request(
                &r,
                "POST",
                "/login",
                "",
                Some("http://127.0.0.1:3000"),
                "next=%2F%2Fevil.test"
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let res = request(&r, "POST", "/login", "", Some("http://127.0.0.1:3000"), "").await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.headers()["retry-after"], "60");
    let d = tempfile::tempdir().unwrap();
    let s = LocalStorage::initialize(d.path()).await.unwrap();
    let r = web::router_with_mode(
        pool.clone(),
        "http://127.0.0.1:3000".into(),
        s,
        Environment::Development,
        true,
    );
    for (method, path) in [
        ("GET", "/images"),
        ("POST", "/people"),
        ("POST", "/login"),
        ("POST", "/logout"),
    ] {
        assert_eq!(
            request(&r, method, path, "", Some("http://127.0.0.1:3000"), "")
                .await
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    assert_eq!(
        request(&r, "GET", "/healthz", "", None, "").await.status(),
        StatusCode::OK
    );
}
