mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use image::{ExtendedColorType, ImageEncoder};
use serde_json::Value;
use sqlx::PgPool;
use tempfile::TempDir;
use the_archive::{
    app::{
        catalog::{self, Input},
        images,
    },
    config::Environment,
    models::Kind,
    storage::LocalStorage,
    web,
};
use tower::ServiceExt;

fn fixture(format: &str, color: u8) -> Vec<u8> {
    let mut bytes = vec![];
    let pixels = [color, 20, 30];
    match format {
        "jpg" => image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .write_image(&pixels, 1, 1, ExtendedColorType::Rgb8)
            .unwrap(),
        "png" => image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&pixels, 1, 1, ExtendedColorType::Rgb8)
            .unwrap(),
        "webp" => image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
            .write_image(&pixels, 1, 1, ExtendedColorType::Rgb8)
            .unwrap(),
        _ => unreachable!(),
    }
    bytes
}
async fn setup(pool: &PgPool) -> (TempDir, LocalStorage, i64, Vec<i64>) {
    let dir = TempDir::new().unwrap();
    let storage = LocalStorage::initialize(dir.path()).await.unwrap();
    let mut ids = vec![];
    for (kind, name, franchise_id) in [
        (Kind::People, "Uploader <private>", None),
        (Kind::Franchises, "Franchise", None),
    ] {
        ids.push(
            catalog::save(
                pool,
                kind,
                None,
                &Input {
                    name: name.into(),
                    franchise_id,
                    ..Default::default()
                },
            )
            .await
            .unwrap(),
        );
    }
    let mut characters = vec![];
    for name in ["Link <safe>", "Venti"] {
        characters.push(
            catalog::save(
                pool,
                Kind::Characters,
                None,
                &Input {
                    name: name.into(),
                    franchise_id: Some(ids[1]),
                    ..Default::default()
                },
            )
            .await
            .unwrap(),
        );
    }
    (dir, storage, ids[0], characters)
}
fn fields(uploader: i64, characters: &[i64]) -> Vec<(&'static str, String)> {
    let mut fields = vec![
        ("uploader", uploader.to_string()),
        ("artist", "  Shared <artist>  ".into()),
        ("source_url", "https://example.com/art?x=1&y=2".into()),
    ];
    fields.extend(characters.iter().map(|c| ("characters", c.to_string())));
    fields
}
fn multipart(fields: &[(&str, String)], files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend(
            format!("--bulk\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                .as_bytes(),
        );
    }
    for (name, bytes) in files {
        // Deliberately misleading MIME declaration. Validation must follow bytes.
        body.extend(format!("--bulk\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: image/svg+xml\r\n\r\n").as_bytes());
        body.extend(bytes);
        body.extend(b"\r\n");
    }
    body.extend(b"--bulk--\r\n");
    body
}
fn request(body: impl Into<Body>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/images")
        .header("origin", "http://127.0.0.1:3000")
        .header("accept", "application/json")
        .header("content-type", "multipart/form-data; boundary=bulk")
        .body(body.into())
        .unwrap()
}
async fn json(response: Response, status: StatusCode) -> Value {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.len() <= 8192);
    serde_json::from_slice(&bytes).unwrap()
}
async fn count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM images")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn shared_uploads_partial_success_duplicates_and_lost_response(pool: PgPool) {
    let (dir, storage, uploader, characters) = setup(&pool).await;
    let client = common::client(&pool, storage.clone()).await;
    // An existing uncategorized image keeps its old metadata even when submitted with a batch's choices.
    let existing = images::upload(
        &pool,
        &storage,
        images::Upload {
            bytes: fixture("png", 1),
            original_filename: "earlier.png".into(),
            metadata: images::Metadata {
                uploader_id: uploader,
                artist: "Original artist".into(),
                source_url: String::new(),
                character_ids: vec![],
            },
        },
    )
    .await
    .unwrap();
    let shared = fields(uploader, &characters);
    // IDs above JavaScript's safe integer range must remain exact decimal strings.
    sqlx::query("SELECT setval(pg_get_serial_sequence('images','id'),9007199254740992)")
        .execute(&pool)
        .await
        .unwrap();
    let mut uploaded_ids = vec![];
    for (format, color) in [("jpg", 2), ("png", 3), ("webp", 4)] {
        let result = json(
            client
                .clone()
                .oneshot(request(multipart(
                    &shared,
                    &[("misleading.svg", fixture(format, color))],
                )))
                .await
                .unwrap(),
            StatusCode::CREATED,
        )
        .await;
        assert_eq!(result["status"], "uploaded");
        let id: i64 = result["id"].as_str().unwrap().parse().unwrap();
        uploaded_ids.push(id);
        let image = images::get(&pool, id).await.unwrap();
        assert_eq!(image.artist.as_deref(), Some("Shared <artist>"));
        assert_eq!(image.uploaded_by_person_id, uploader);
        assert_eq!(
            images::memberships(&pool, id)
                .await
                .unwrap()
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            characters
        );
        assert_eq!(
            tokio::fs::read(dir.path().join(image.storage_key))
                .await
                .unwrap(),
            fixture(format, color)
        );
        // Ordinary per-file validation errors do not undo earlier uploads.
        for bytes in [b"corrupt PNG".to_vec(), b"<svg/>".to_vec()] {
            let result = json(
                client
                    .clone()
                    .oneshot(request(multipart(&shared, &[("bad.png", bytes)])))
                    .await
                    .unwrap(),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
            assert_eq!(result["status"], "failed");
            assert!(result["message"].is_string());
        }
    }
    let other_uploader = catalog::save(
        &pool,
        Kind::People,
        None,
        &Input {
            name: "Another uploader".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let duplicate_fields = vec![
        ("uploader", other_uploader.to_string()),
        ("artist", "Changed artist".into()),
        ("source_url", String::new()),
    ];
    for (bytes, id) in [
        (fixture("png", 1), existing.id),
        (fixture("png", 3), uploaded_ids[1]),
    ] {
        let duplicate = json(
            client
                .clone()
                .oneshot(request(multipart(
                    &duplicate_fields,
                    &[("changed.png", bytes)],
                )))
                .await
                .unwrap(),
            StatusCode::OK,
        )
        .await;
        assert_eq!(duplicate["status"], "duplicate");
        assert_eq!(duplicate["id"], id.to_string());
    }
    assert!(
        images::memberships(&pool, existing.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        images::get(&pool, existing.id)
            .await
            .unwrap()
            .artist
            .as_deref(),
        Some("Original artist")
    );
    assert_eq!(
        images::memberships(&pool, uploaded_ids[1])
            .await
            .unwrap()
            .len(),
        2
    );
    for id in [existing.id, uploaded_ids[1]] {
        assert_eq!(
            images::get(&pool, id).await.unwrap().uploaded_by_person_id,
            uploader
        );
    }
    assert_eq!(
        images::get(&pool, uploaded_ids[1])
            .await
            .unwrap()
            .artist
            .as_deref(),
        Some("Shared <artist>")
    );
    assert_eq!(
        images::get(&pool, uploaded_ids[1])
            .await
            .unwrap()
            .source_url
            .as_deref(),
        Some("https://example.com/art?x=1&y=2")
    );
    let uncategorized = json(
        client
            .clone()
            .oneshot(request(multipart(
                &fields(uploader, &[]),
                &[("uncategorized.png", fixture("png", 10))],
            )))
            .await
            .unwrap(),
        StatusCode::CREATED,
    )
    .await;
    let uncategorized_id = uncategorized["id"]
        .as_str()
        .unwrap()
        .parse::<i64>()
        .unwrap();
    assert!(
        images::memberships(&pool, uncategorized_id)
            .await
            .unwrap()
            .is_empty()
    );
    // Simulate losing a successful response, then explicitly retry the same File.
    let body = multipart(&shared, &[("unconfirmed.png", fixture("png", 9))]);
    let lost = client.clone().oneshot(request(body.clone())).await.unwrap();
    assert_eq!(lost.status(), StatusCode::CREATED);
    drop(lost);
    let retry = json(
        client.clone().oneshot(request(body)).await.unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(retry["status"], "duplicate");
    assert_eq!(count(&pool).await, 6);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 6);

    // No Accept opt-in: normal single upload still redirects. q=0 also remains HTML.
    for accept in ["text/html", "application/json;q=0", "*/*"] {
        let mut req = request(multipart(&shared, &[("same.png", fixture("png", 3))]));
        req.headers_mut().insert("accept", accept.parse().unwrap());
        let response = client.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers()["location"],
            format!("/images/{}?duplicate=1", uploaded_ids[1])
        );
    }
    // Existing editor remains an ordinary HTML form and updates memberships.
    let response = client
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/images/{}/edit", uploaded_ids[0]))
                .header("origin", "http://127.0.0.1:3000")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "uploader={uploader}&artist=Corrected&characters={}",
                    characters[1]
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        images::memberships(&pool, uploaded_ids[0]).await.unwrap()[0].id,
        characters[1]
    );
    assert_eq!(
        images::memberships(&pool, uploaded_ids[0])
            .await
            .unwrap()
            .len(),
        1
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn bulk_page_routes_and_request_protection(pool: PgPool) {
    let (_dir, storage, uploader, characters) = setup(&pool).await;
    let client = common::client(&pool, storage.clone()).await;
    let body = multipart(
        &fields(uploader, &characters),
        &[("a.png", fixture("png", 7))],
    );
    let unauthenticated = client
        .router
        .clone()
        .oneshot(request(body.clone()))
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    let mut wrong_origin = request(body.clone());
    wrong_origin
        .headers_mut()
        .insert("origin", "https://wrong.example".parse().unwrap());
    assert_eq!(
        client.clone().oneshot(wrong_origin).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let maintenance = web::router_with_mode(
        pool.clone(),
        "http://127.0.0.1:3000".into(),
        storage,
        Environment::Development,
        true,
    );
    let mut req = request(body.clone());
    req.headers_mut()
        .insert("cookie", client.cookie.parse().unwrap());
    assert_eq!(
        maintenance.oneshot(req).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(count(&pool).await, 0);
    let logged_out = client
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/images/bulk")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logged_out.status(), StatusCode::SEE_OTHER);
    let page = client
        .clone()
        .oneshot(
            Request::builder()
                .uri("/images/bulk")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = String::from_utf8(
        page.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(
        html.contains("Uploader &lt;private&gt;") || html.contains("Uploader &#60;private&#62;")
    );
    assert!(html.contains("Link &lt;safe&gt;") || html.contains("Link &#60;safe&#62;"));
    assert!(html.contains("<noscript>"));
    assert!(html.contains("/images/new"));
    assert!(html.contains("/static/bulk-upload.js"));
    assert!(!html.contains("Uploader <private>"));
    for path in ["/images/not-an-id", "/images/bulk/content"] {
        assert_eq!(
            client
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    let script = client
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/static/bulk-upload.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(script.status(), StatusCode::OK);
    assert!(
        script.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/javascript")
    );
    let content = client
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/images/1/content")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(content.status(), StatusCode::UNAUTHORIZED);
    sqlx::query("UPDATE sessions SET created_at=now()-interval '8 days', expires_at=now()-interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        client.oneshot(request(body)).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(count(&pool).await, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn metadata_each_request_one_file_and_storage_failure(pool: PgPool) {
    let (dir, storage, uploader, characters) = setup(&pool).await;
    let client = common::client(&pool, storage).await;
    for bad_fields in [
        fields(0, &characters),
        fields(999999, &characters),
        fields(uploader, &[999999]),
        vec![
            ("uploader", uploader.to_string()),
            ("artist", "a".repeat(201)),
        ],
        vec![
            ("uploader", uploader.to_string()),
            ("source_url", "https://user:secret@example.com".into()),
        ],
    ] {
        let result = json(
            client
                .clone()
                .oneshot(request(multipart(
                    &bad_fields,
                    &[("a.png", fixture("png", 5))],
                )))
                .await
                .unwrap(),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
        assert_eq!(result["status"], "failed");
        assert!(!result["message"].as_str().unwrap().contains("secret"));
    }
    let response = client
        .clone()
        .oneshot(request(multipart(
            &fields(uploader, &characters),
            &[("a.png", fixture("png", 5)), ("b.png", fixture("png", 6))],
        )))
        .await
        .unwrap();
    assert!(
        json(response, StatusCode::UNPROCESSABLE_ENTITY).await["message"]
            .as_str()
            .unwrap()
            .contains("exactly one")
    );
    assert_eq!(count(&pool).await, 0);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    // Break only this test's storage root after initialization; no real archive involved.
    std::fs::remove_dir(dir.path()).unwrap();
    std::fs::write(dir.path(), b"unusable directory").unwrap();
    let result = json(
        client
            .oneshot(request(multipart(
                &fields(uploader, &[]),
                &[("a.png", fixture("png", 5))],
            )))
            .await
            .unwrap(),
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert_eq!(
        result["message"],
        "The image storage operation failed. Please try again."
    );
    assert_eq!(count(&pool).await, 0);
    assert!(!result.to_string().contains(dir.path().to_str().unwrap()));
    std::fs::remove_file(dir.path()).unwrap();
    std::fs::create_dir(dir.path()).unwrap();
}

struct NativeServer(std::process::Child);
impl Drop for NativeServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl NativeServer {
    async fn stop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(self.0.id() as libc::pid_t, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        self.0.kill().unwrap();
        for _ in 0..100 {
            if let Some(status) = self.0.try_wait().unwrap() {
                #[cfg(unix)]
                assert!(status.success());
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("Native bulk server did not stop");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn native_bulk_upload_edit_restart_and_delete_smoke(pool: PgPool) {
    let (dir, _, uploader, characters) = setup(&pool).await;
    the_archive::app::auth::create(&pool, "native-owner", common::PASSWORD)
        .await
        .unwrap();
    let mut database = url::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    database.set_path(pool.connect_options().get_database().unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let origin = format!("http://{address}");
    let cwd = TempDir::new().unwrap();
    let start = || {
        NativeServer(
            std::process::Command::new(env!("CARGO_BIN_EXE_the-archive"))
                .current_dir(cwd.path())
                .env_clear()
                .env("APP_ENV", "development")
                .env("DATABASE_URL", database.as_str())
                .env("LISTEN_ADDR", address.to_string())
                .env("APP_ORIGIN", &origin)
                .env("IMAGE_STORAGE_DIR", dir.path())
                .env("DISCORD_ENABLED", "false")
                .env("MAINTENANCE_MODE", "false")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let ready = async || {
        for _ in 0..100 {
            if let Ok(response) = http.get(format!("{origin}/healthz")).send().await
                && response.status() == StatusCode::OK
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("Native bulk server did not start");
    };
    let mut server = start();
    ready().await;
    let login = http
        .post(format!("{origin}/login"))
        .header("origin", &origin)
        .form(&[("username", "native-owner"), ("password", common::PASSWORD)])
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let bulk = http
        .get(format!("{origin}/images/bulk"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(bulk.status(), StatusCode::OK);
    assert!(bulk.text().await.unwrap().contains("bulk-controls"));
    let script = http
        .get(format!("{origin}/static/bulk-upload.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(script.status(), StatusCode::OK);
    assert!(
        script
            .text()
            .await
            .unwrap()
            .contains("export class UploadQueue")
    );
    let mut image_id = String::new();
    for expected in ["uploaded", "duplicate"] {
        let mut form = reqwest::multipart::Form::new()
            .text("uploader", uploader.to_string())
            .text("artist", "Shared artist");
        for character in &characters {
            form = form.text("characters", character.to_string());
        }
        form = form.part(
            "file",
            reqwest::multipart::Part::bytes(fixture("png", 33)).file_name("shared.png"),
        );
        let response = http
            .post(format!("{origin}/images"))
            .header("origin", &origin)
            .header("cookie", &cookie)
            .header("accept", "application/json")
            .multipart(form)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if expected == "uploaded" {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            }
        );
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["status"], expected);
        if image_id.is_empty() {
            image_id = body["id"].as_str().unwrap().to_owned();
        }
        assert_eq!(body["id"], image_id);
    }
    assert_eq!(
        images::memberships(&pool, image_id.parse().unwrap())
            .await
            .unwrap()
            .len(),
        2
    );
    for character in &characters {
        let gallery = http
            .get(format!("{origin}/images?character={character}"))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap();
        assert!(gallery.text().await.unwrap().contains("shared.png"));
    }
    let edit = http
        .post(format!("{origin}/images/{image_id}/edit"))
        .header("origin", &origin)
        .header("cookie", &cookie)
        .form(&[
            ("uploader", uploader.to_string()),
            ("artist", "Corrected".into()),
            ("characters", characters[1].to_string()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(edit.status(), StatusCode::SEE_OTHER);
    server.stop().await;
    let mut server = start();
    ready().await;
    let detail = http
        .get(format!("{origin}/images/{image_id}"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    assert!(detail.text().await.unwrap().contains("Corrected"));
    let content = http
        .get(format!("{origin}/images/{image_id}/content"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(content.status(), StatusCode::OK);
    assert_eq!(content.headers()["content-type"], "image/png");
    assert_eq!(content.bytes().await.unwrap().as_ref(), fixture("png", 33));
    assert_eq!(
        http.get(format!("{origin}/images/{image_id}/content"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let delete = http
        .post(format!("{origin}/images/{image_id}/delete"))
        .header("origin", &origin)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::SEE_OTHER);
    assert_eq!(count(&pool).await, 0);
    assert_eq!(
        catalog::list(&pool, Kind::Characters).await.unwrap().len(),
        2
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    server.stop().await;
}
