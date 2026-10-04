use axum::{
    Router,
    body::{Body, Bytes},
    http::{HeaderMap, Request, StatusCode},
    response::IntoResponse,
    routing::patch,
};
use ed25519_dalek::{Signer, SigningKey};
use http_body_util::BodyExt;
use image::{ExtendedColorType, ImageEncoder};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use the_archive::{
    app::{
        associations,
        catalog::{self, Input},
        images::{self, Metadata, Upload},
        retrieval::{self, Resolution},
    },
    config::Environment,
    discord::{
        self,
        client::{Client, Failure},
        commands,
        config::{Config, snowflake},
        responses, verification,
    },
    models::{Filters, Kind},
    storage::LocalStorage,
    web,
};
use tower::ServiceExt;
const ORIGIN: &str = "https://archive.example.test";
fn key() -> SigningKey {
    SigningKey::from_bytes(&[71; 32])
}
fn config() -> Config {
    Config {
        application: 111,
        guild: 222,
        users: [333, 334].into(),
        key: key().verifying_key(),
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn signed(body: &[u8], timestamp: i64) -> HeaderMap {
    let timestamp = timestamp.to_string();
    let signed = [timestamp.as_bytes(), body].concat();
    let mut headers = HeaderMap::new();
    headers.insert("x-signature-timestamp", timestamp.parse().unwrap());
    headers.insert(
        "x-signature-ed25519",
        hex(&key().sign(&signed).to_bytes()).parse().unwrap(),
    );
    headers
}
fn request(value: &Value) -> Request<Body> {
    let bytes = serde_json::to_vec(value).unwrap();
    let mut r = Request::builder()
        .method("POST")
        .uri("/discord/interactions")
        .body(Body::from(bytes.clone()))
        .unwrap();
    *r.headers_mut() = signed(&bytes, chrono::Utc::now().timestamp());
    r
}
async fn json_response(r: axum::response::Response) -> Value {
    assert_eq!(r.status(), StatusCode::OK);
    serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
fn command(name: &str, options: Value) -> Value {
    json!({"id":"1000","application_id":"111","type":2,"guild_id":"222","member":{"user":{"id":"333"}},"token":"test-only-interaction-token","data":{"type":1,"name":name,"options":options}})
}
fn option(name: &str, value: &str) -> Value {
    json!({"name":name,"type":3,"value":value})
}
#[test]
fn signatures_and_timestamp_policy() {
    let raw = b" {\"type\" : 1, \"application_id\": \"111\"}\n";
    let now = 1700000000;
    let headers = signed(raw, now);
    assert!(verification::verify(&key().verifying_key(), &headers, raw, now).is_ok());
    assert!(verification::verify(&key().verifying_key(), &headers, b"{}", now).is_err());
    assert!(verification::verify(&key().verifying_key(), &headers, raw, now + 301).is_err());
    assert!(verification::verify(&key().verifying_key(), &headers, raw, now - 31).is_err());
    assert!(verification::verify(&key().verifying_key(), &headers, raw, now + 300).is_ok());
    assert!(verification::verify(&key().verifying_key(), &headers, raw, now - 30).is_ok());
    let mut altered = headers.clone();
    altered.insert(
        "x-signature-timestamp",
        (now + 1).to_string().parse().unwrap(),
    );
    assert!(verification::verify(&key().verifying_key(), &altered, raw, now).is_err());
    for malformed in ["", "ab", &"z".repeat(128), &"0".repeat(128)] {
        let mut h = headers.clone();
        h.insert("x-signature-ed25519", malformed.parse().unwrap());
        assert!(verification::verify(&key().verifying_key(), &h, raw, now).is_err());
    }
    assert!(verification::verify(&key().verifying_key(), &HeaderMap::new(), raw, now).is_err());
    let mut h = headers.clone();
    h.append("x-signature-timestamp", now.to_string().parse().unwrap());
    assert!(verification::verify(&key().verifying_key(), &h, raw, now).is_err());
}
#[test]
fn manifest_parser_configuration_and_replay_bounds() {
    let specs = commands::manifest();
    assert_eq!(specs.len(), 4);
    for spec in specs {
        let options = spec
            .options
            .iter()
            .map(|o| {
                if o.kind == 3 {
                    option(o.name, "Link")
                } else {
                    json!({"name":o.name,"type":4,"value":100000})
                }
            })
            .collect::<Vec<_>>();
        assert!(commands::parse(&json!({"name":spec.name,"type":1,"options":options})).is_ok());
        assert!(commands::parse(&json!({"name":spec.name,"type":2,"options":options})).is_err());
    }
    for data in [
        json!({"name":"character","type":1}),
        json!({"name":"images","type":1,"options":[option("character","Link"),option("tag","anything")]}),
        json!({"name":"images","type":1,"options":[option("character","Link"),json!({"name":"page","type":4,"value":0})]}),
        json!({"name":"character","type":1,"options":[option("character","Link"),option("character","Venti")]}),
        json!({"name":"images","type":1,"options":[option("character","Link"),json!({"name":"page","type":4,"value":1.5})]}),
    ] {
        assert!(commands::parse(&data).is_err());
    }
    assert!(retrieval::offset(100001, 10).is_err());
    assert!(retrieval::offset(i64::MAX, 10).is_err());
    assert_eq!(retrieval::offset(100000, 10).unwrap(), 999990);
    assert!(Config::from_values(|_| None).unwrap().is_none());
    let mut values = HashMap::from([
        ("DISCORD_ENABLED", "true".into()),
        ("DISCORD_APPLICATION_ID", "111".into()),
        ("DISCORD_GUILD_ID", "222".into()),
        ("DISCORD_PUBLIC_KEY", hex(key().verifying_key().as_bytes())),
        ("DISCORD_ALLOWED_USER_IDS", "333, 334".into()),
    ]);
    assert_eq!(
        Config::from_values(|n| values.get(n).cloned())
            .unwrap()
            .unwrap()
            .users
            .len(),
        2
    );
    for (name, value) in [
        ("DISCORD_ENABLED", "yes"),
        ("DISCORD_APPLICATION_ID", "1.0"),
        ("DISCORD_GUILD_ID", "0"),
        ("DISCORD_PUBLIC_KEY", "00"),
        ("DISCORD_ALLOWED_USER_IDS", ""),
    ] {
        let mut bad = values.clone();
        bad.insert(name, value.into());
        assert!(Config::from_values(|n| bad.get(n).cloned()).is_err());
    }
    values.remove("DISCORD_PUBLIC_KEY");
    assert!(Config::from_values(|n| values.get(n).cloned()).is_err());
    for id in ["", "0", "-1", "1e10", "18446744073709551616"] {
        assert!(snowflake(id).is_err());
    }
    let mut cache = discord::Replay::new(2);
    let now = Instant::now();
    assert_eq!(cache.admit(1, now), discord::ReplayResult::New);
    assert_eq!(cache.admit(1, now), discord::ReplayResult::Duplicate);
    assert_eq!(cache.admit(2, now), discord::ReplayResult::New);
    assert_eq!(cache.admit(3, now), discord::ReplayResult::Full);
    assert_eq!(
        cache.admit(1, now + Duration::from_secs(330)),
        discord::ReplayResult::Duplicate
    );
    assert_eq!(
        cache.admit(1, now + Duration::from_secs(361)),
        discord::ReplayResult::New
    );
}
#[tokio::test]
async fn signed_ping_denials_and_browser_isolation_without_database() {
    // A closed pool proves these paths cannot query, even accidentally.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:9/unused")
        .unwrap();
    pool.close().await;
    let root = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(root.path()).await.unwrap();
    for maintenance in [false, true] {
        let runtime = discord::Runtime::new(
            config(),
            pool.clone(),
            storage.clone(),
            ORIGIN.into(),
            maintenance,
            Client::new().unwrap(),
        );
        let router = web::router_with_discord(
            pool.clone(),
            ORIGIN.into(),
            storage.clone(),
            Environment::Production,
            maintenance,
            Some(runtime.clone()),
        );
        assert_eq!(
            json_response(
                router
                    .clone()
                    .oneshot(request(&json!({"type":1,"application_id":"111"})))
                    .await
                    .unwrap()
            )
            .await,
            json!({"type":1})
        );
        let mut denied = command("characters", json!([]));
        denied["guild_id"] = json!("999");
        let mut denial =
            json_response(router.clone().oneshot(request(&denied)).await.unwrap()).await;
        assert_eq!(denial["data"]["flags"], 64);
        assert_eq!(denial["data"]["allowed_mentions"]["parse"], json!([]));
        let generic = denial["data"]["content"].clone();
        for changes in [
            json!({"application_id":"999"}),
            json!({"guild_id":null}),
            json!({"member":{"user":{"id":"999"}}}),
            json!({"member":null}),
            json!({"type":3}),
            json!({"application_id":111.0}),
        ] {
            let mut p = command("characters", json!([]));
            for (k, v) in changes.as_object().unwrap() {
                p[k] = v.clone();
            }
            denial = json_response(router.clone().oneshot(request(&p)).await.unwrap()).await;
            assert_eq!(denial["data"]["content"], generic);
        }
        let bad = Request::builder()
            .method("POST")
            .uri("/discord/interactions")
            .body(Body::from("{}"))
            .unwrap();
        assert_eq!(router.clone().oneshot(bad).await.unwrap().status(), 401);
        if maintenance {
            let result = json_response(
                router
                    .clone()
                    .oneshot(request(&command("characters", json!([]))))
                    .await
                    .unwrap(),
            )
            .await;
            assert!(
                result["data"]["content"]
                    .as_str()
                    .unwrap()
                    .contains("maintenance")
            );
        } else {
            for data in [
                json!({"name":"not-supported","type":1}),
                json!({"name":"character","type":2}),
                json!({"name":"characters","type":1,"options":[option("tag","ignored")]}),
            ] {
                let mut invalid = command("characters", json!([]));
                invalid["data"] = data;
                let result =
                    json_response(router.clone().oneshot(request(&invalid)).await.unwrap()).await;
                assert_eq!(result["type"], 4);
                assert_eq!(result["data"]["flags"], 64);
                assert_eq!(runtime.active_jobs(), 0);
            }
            for uri in [
                "/login",
                "/logout",
                "/characters",
                "/discord/interactions/edit",
            ] {
                let response = router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri(uri)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), 403);
            }
            for uri in ["/characters", "/images", "/characters/1", "/images/1"] {
                assert_eq!(
                    router
                        .clone()
                        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                        .await
                        .unwrap()
                        .status(),
                    303
                );
            }
            assert_eq!(
                router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri("/images/1/content")
                            .body(Body::empty())
                            .unwrap()
                    )
                    .await
                    .unwrap()
                    .status(),
                401
            );
        }
        assert_eq!(
            router
                .oneshot(
                    Request::builder()
                        .uri("/healthz")
                        .body(Body::empty())
                        .unwrap()
                )
                .await
                .unwrap()
                .status(),
            200
        );
        runtime.shutdown().await;
    }
    let disabled = web::router(pool, ORIGIN.into(), storage, Environment::Production);
    assert_eq!(
        disabled
            .oneshot(request(&json!({"type":1,"application_id":"111"})))
            .await
            .unwrap()
            .status(),
        404
    );
}

async fn record(pool: &PgPool, kind: Kind, name: &str, franchise: Option<i64>) -> i64 {
    catalog::save(
        pool,
        kind,
        None,
        &Input {
            name: name.into(),
            franchise_id: franchise,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}
struct Fixture {
    link: i64,
    venti: i64,
    other_link: i64,
    alice: i64,
    bob: i64,
    zelda: i64,
    kin: i64,
    favorite: i64,
}
async fn fixture(pool: &PgPool) -> Fixture {
    let alice = record(pool, Kind::People, "Alice", None).await;
    let bob = record(pool, Kind::People, "Bob", None).await;
    let zelda = record(pool, Kind::Franchises, "Zelda", None).await;
    let genshin = record(pool, Kind::Franchises, "Genshin", None).await;
    let link = record(pool, Kind::Characters, "Link", Some(zelda)).await;
    let venti = record(pool, Kind::Characters, "Venti", Some(genshin)).await;
    let other_link = record(pool, Kind::Characters, "Link", Some(genshin)).await;
    let kin = retrieval::exact(pool, Kind::Types, "kin")
        .await
        .unwrap()
        .unwrap();
    let favorite = retrieval::exact(pool, Kind::Types, "favorite")
        .await
        .unwrap()
        .unwrap();
    associations::associate(pool, link, alice, kin, false)
        .await
        .unwrap();
    associations::associate(pool, link, bob, favorite, false)
        .await
        .unwrap();
    associations::associate(pool, venti, alice, favorite, false)
        .await
        .unwrap();
    associations::associate(pool, venti, alice, kin, false)
        .await
        .unwrap();
    Fixture {
        link,
        venti,
        other_link,
        alice,
        bob,
        zelda,
        kin,
        favorite,
    }
}
fn png(color: u8) -> Vec<u8> {
    let mut bytes = vec![];
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[color, 2, 3], 1, 1, ExtendedColorType::Rgb8)
        .unwrap();
    bytes
}
async fn artwork(
    pool: &PgPool,
    storage: &LocalStorage,
    f: &Fixture,
    color: u8,
    characters: Vec<i64>,
) -> i64 {
    images::upload(
        pool,
        storage,
        Upload {
            bytes: png(color),
            original_filename: format!("Artwork {color}.png"),
            metadata: Metadata {
                uploader_id: f.alice,
                character_ids: characters,
                artist: "Artist @everyone [test]".into(),
                ..Default::default()
            },
        },
    )
    .await
    .unwrap()
    .id
}
#[sqlx::test]
async fn exact_queries_same_row_pagination_and_read_only(pool: PgPool) {
    let f = fixture(&pool).await;
    match retrieval::resolve(&pool, " link ", None).await.unwrap() {
        Resolution::Ambiguous { candidates, more } => {
            assert_eq!(candidates.len(), 2);
            assert!(!more);
        }
        _ => panic!("must disambiguate"),
    }
    for selector in ["Link".into(), format!("#{}", f.link)] {
        match retrieval::resolve(&pool, &selector, Some(" ZELDA "))
            .await
            .unwrap()
        {
            Resolution::Found(c) => assert_eq!(c.id, f.link),
            _ => panic!("expected exact character"),
        }
    }
    assert!(matches!(
        retrieval::resolve(&pool, &format!("#{}", f.link), Some("Genshin"))
            .await
            .unwrap(),
        Resolution::Unknown
    ));
    assert!(matches!(
        retrieval::resolve(&pool, "Link", Some("unknown"))
            .await
            .unwrap(),
        Resolution::Unknown
    ));
    for name in ["%_", "Éowyn", "#not-an-id"] {
        let id = record(&pool, Kind::Characters, name, Some(f.zelda)).await;
        match retrieval::resolve(&pool, name, None).await.unwrap() {
            Resolution::Found(c) => assert_eq!(c.id, id),
            _ => panic!("literal exact name"),
        }
    }
    assert!(matches!(
        retrieval::resolve(&pool, "%", None).await.unwrap(),
        Resolution::Unknown
    ));
    assert!(retrieval::resolve(&pool, "#0", None).await.is_err());
    assert!(
        retrieval::resolve(&pool, "#999999999999999999999", None)
            .await
            .is_err()
    );
    let (rows, _) = retrieval::characters(
        &pool,
        &Filters {
            person: Some(f.alice),
            association_type: Some(f.favorite),
            ..Default::default()
        },
        1,
    )
    .await
    .unwrap();
    assert_eq!(rows.iter().map(|c| c.id).collect::<Vec<_>>(), vec![f.venti]);
    for filters in [
        Filters {
            person: Some(f.alice),
            ..Default::default()
        },
        Filters {
            association_type: Some(f.favorite),
            ..Default::default()
        },
    ] {
        let (rows, _) = retrieval::characters(&pool, &filters, 1).await.unwrap();
        assert_eq!(rows.len(), 2);
    }
    assert!(
        retrieval::characters(
            &pool,
            &Filters {
                person: Some(999999),
                ..Default::default()
            },
            1
        )
        .await
        .is_err()
    );
    for i in 0..15 {
        record(
            &pool,
            Kind::Characters,
            &format!("Page{i:02}"),
            Some(f.zelda),
        )
        .await;
    }
    let (first, more) = retrieval::characters(&pool, &Filters::default(), 1)
        .await
        .unwrap();
    assert_eq!(first.len(), 10);
    assert!(more);
    let (second, _) = retrieval::characters(&pool, &Filters::default(), 2)
        .await
        .unwrap();
    assert!(first.iter().all(|a| second.iter().all(|b| a.id != b.id)));
    let root = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(root.path()).await.unwrap();
    let shared = artwork(&pool, &storage, &f, 1, vec![f.link, f.venti]).await;
    for color in 2..8 {
        artwork(&pool, &storage, &f, color, vec![f.link]).await;
    }
    assert_eq!(retrieval::image_count(&pool, f.link).await.unwrap(), 7);
    assert_eq!(retrieval::image_count(&pool, f.venti).await.unwrap(), 1);
    let (first, more) = retrieval::images(&pool, f.link, 1).await.unwrap();
    assert_eq!(first.len(), 5);
    assert!(more);
    assert!(first.windows(2).all(|p| p[0].id > p[1].id));
    let (second, more) = retrieval::images(&pool, f.link, 2).await.unwrap();
    assert_eq!(second.len(), 2);
    assert!(!more);
    assert!(second.iter().any(|i| i.id == shared));
    assert_eq!(
        retrieval::random_image(&pool, f.venti)
            .await
            .unwrap()
            .unwrap()
            .id,
        shared
    );
    assert!(
        retrieval::random_image(&pool, f.other_link)
            .await
            .unwrap()
            .is_none()
    );
    let before = counts(&pool).await;
    for name in ["character", "images", "random-image"] {
        let cmd = commands::parse(
            &command(name, json!([option("character", &format!("#{}", f.link))]))["data"],
        )
        .unwrap();
        let r = responses::execute(&pool, &storage, ORIGIN, &cmd, 100000)
            .await
            .unwrap();
        payload_bounds(&r.payload);
        assert_eq!(r.payload["allowed_mentions"]["parse"], json!([]));
    }
    let listed = commands::parse(&command("characters", json!([]))["data"]).unwrap();
    let listed = responses::execute(&pool, &storage, ORIGIN, &listed, 100000)
        .await
        .unwrap();
    payload_bounds(&listed.payload);
    assert_eq!(
        listed.payload["embeds"][0]["fields"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    assert!(
        commands::parse(
            &command(
                "images",
                json!([
                    option("character", "Link"),
                    json!({"name":"page","type":4,"value":2})
                ])
            )["data"]
        )
        .is_err()
    );
    let cmd = commands::parse(&command("characters", json!([option("person", "missing")]))["data"])
        .unwrap();
    assert!(
        responses::execute(&pool, &storage, ORIGIN, &cmd, 100)
            .await
            .is_err()
    );
    assert_eq!(counts(&pool).await, before);
    assert_ne!(f.alice, f.bob);
    assert_ne!(f.kin, f.favorite);
}
async fn counts(pool: &PgPool) -> Vec<i64> {
    let mut counts = vec![];
    for table in [
        "people",
        "franchises",
        "characters",
        "association_types",
        "person_character_associations",
        "images",
        "image_characters",
        "tags",
        "character_tags",
    ] {
        counts.push(
            sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(pool)
                .await
                .unwrap(),
        );
    }
    counts
}
fn payload_bounds(p: &Value) {
    assert!(p["content"].as_str().unwrap().encode_utf16().count() <= 2000);
    let mut total = 0;
    for e in p["embeds"].as_array().unwrap() {
        for (key, max) in [("title", 256), ("description", 4096)] {
            let n = e[key].as_str().unwrap().encode_utf16().count();
            assert!(n <= max);
            total += n;
        }
        for f in e["fields"].as_array().unwrap() {
            for (key, max) in [("name", 256), ("value", 1024)] {
                let n = f[key].as_str().unwrap().encode_utf16().count();
                assert!(n <= max);
                total += n;
            }
        }
    }
    assert!(total <= 6000);
}

struct Delivery {
    headers: HeaderMap,
    body: Bytes,
    path: String,
    method: axum::http::Method,
}
struct Fake {
    client: Client,
    events: tokio::sync::mpsc::Receiver<Delivery>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fake {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[derive(Clone)]
struct FakeState {
    events: tokio::sync::mpsc::Sender<Delivery>,
    statuses: Arc<std::sync::Mutex<std::collections::VecDeque<u16>>>,
    blocked: Option<Arc<tokio::sync::Semaphore>>,
}
async fn fake_handler(
    axum::extract::State(state): axum::extract::State<FakeState>,
    req: axum::extract::Request,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();
    let body = axum::body::to_bytes(body, 10 * 1024 * 1024).await.unwrap();
    state
        .events
        .send(Delivery {
            headers: parts.headers,
            body,
            path: parts.uri.path().into(),
            method: parts.method,
        })
        .await
        .unwrap();
    if let Some(blocked) = &state.blocked {
        let _permit = blocked.acquire().await.unwrap();
    }
    let status = state.statuses.lock().unwrap().pop_front().unwrap_or(200);
    (
        StatusCode::from_u16(status).unwrap(),
        axum::Json(json!({"retry_after":0,"id":"555"})),
    )
        .into_response()
}
async fn fake_api(statuses: Vec<u16>, blocked: Option<Arc<tokio::sync::Semaphore>>) -> Fake {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Client::local_test_server(listener.local_addr().unwrap()).unwrap();
    let (events, receiver) = tokio::sync::mpsc::channel(32);
    let router = Router::new()
        .route("/api/v10/{*path}", patch(fake_handler).post(fake_handler))
        .with_state(FakeState {
            events,
            statuses: Arc::new(std::sync::Mutex::new(statuses.into())),
            blocked,
        });
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Fake {
        client,
        events: receiver,
        task,
    }
}
async fn delivery(fake: &mut Fake) -> Delivery {
    tokio::time::timeout(Duration::from_secs(5), fake.events.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn decode(d: Delivery) -> (Value, Option<(String, String, Vec<u8>)>) {
    decode_at(d, "test-only-interaction-token").await
}
async fn decode_at(d: Delivery, token: &str) -> (Value, Option<(String, String, Vec<u8>)>) {
    assert_eq!(d.method, axum::http::Method::PATCH);
    assert_eq!(
        d.path,
        format!("/api/v10/webhooks/111/{token}/messages/@original")
    );
    let content_type = d.headers["content-type"].to_str().unwrap();
    if content_type.starts_with("application/json") {
        return (serde_json::from_slice(&d.body).unwrap(), None);
    }
    use axum::extract::FromRequest;
    let mut r = Request::new(Body::from(d.body));
    *r.headers_mut() = d.headers;
    let mut parts = axum::extract::Multipart::from_request(r, &())
        .await
        .unwrap();
    let mut payload = None;
    let mut file = None;
    while let Some(field) = parts.next_field().await.unwrap() {
        if field.name() == Some("payload_json") {
            payload = Some(serde_json::from_slice(&field.bytes().await.unwrap()).unwrap());
        } else {
            assert_eq!(field.name(), Some("files[0]"));
            assert!(file.is_none());
            let filename = field.file_name().unwrap().to_owned();
            let mime = field.content_type().unwrap().to_owned();
            file = Some((filename, mime, field.bytes().await.unwrap().to_vec()));
        }
    }
    (payload.unwrap(), file)
}
#[sqlx::test]
async fn signed_full_retrieval_and_attachment_workflow(pool: PgPool) {
    let f = fixture(&pool).await;
    let root = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(root.path()).await.unwrap();
    let id = artwork(&pool, &storage, &f, 33, vec![f.link, f.venti]).await;
    let before = counts(&pool).await;
    let mut fake = fake_api(vec![], None).await;
    let runtime = discord::Runtime::new(
        config(),
        pool.clone(),
        storage.clone(),
        ORIGIN.into(),
        false,
        fake.client.clone(),
    );
    let router = web::router_with_discord(
        pool.clone(),
        ORIGIN.into(),
        storage.clone(),
        Environment::Production,
        false,
        Some(runtime.clone()),
    );
    for (n, name) in ["character", "images", "random-image", "characters"]
        .iter()
        .enumerate()
    {
        let options = if *name == "characters" {
            json!([
                option("person", " Alice "),
                option("association", " FAVORITE ")
            ])
        } else {
            json!([option("character", "Venti")])
        };
        let mut p = command(name, options);
        p["id"] = json!((1000 + n).to_string());
        let deferred = json_response(router.clone().oneshot(request(&p)).await.unwrap()).await;
        assert_eq!(deferred["type"], 5);
        assert!(deferred["data"].get("flags").is_none());
        let (result, file) = decode(delivery(&mut fake).await).await;
        payload_bounds(&result);
        assert_eq!(result["allowed_mentions"]["parse"], json!([]));
        if *name == "images" || *name == "random-image" || *name == "character" {
            let (filename, mime, bytes) = file.unwrap();
            assert_eq!(mime, "image/png");
            assert_eq!(bytes, png(33));
            assert_eq!(filename, format!("archive-image-{id}.png"));
            assert_eq!(result["attachments"][0]["id"], 0);
            let desc = result["embeds"][0]["description"].as_str().unwrap();
            if *name != "character" {
                assert!(desc.contains(&format!("/images?character={}", f.venti)));
            }
            assert!(!desc.contains("&page="));
            assert!(desc.contains(&format!("preview: image #{id} only")));
        } else {
            assert!(file.is_none());
        }
        if *name == "characters" {
            let fields = result["embeds"][0]["fields"].as_array().unwrap();
            assert_eq!(fields.len(), 1);
            assert!(fields[0]["name"].as_str().unwrap().contains("Venti"));
        }
    }
    let mut second_owner = command("character", json!([option("character", "Venti")]));
    second_owner["id"] = json!("1004");
    second_owner["member"]["user"]["id"] = json!("334");
    assert_eq!(
        json_response(
            router
                .clone()
                .oneshot(request(&second_owner))
                .await
                .unwrap()
        )
        .await["type"],
        5
    );
    let (second_owner, _) = decode(delivery(&mut fake).await).await;
    assert!(
        second_owner["embeds"][0]["title"]
            .as_str()
            .unwrap()
            .contains("Venti")
    );
    let duplicate = json_response(
        router
            .clone()
            .oneshot(request(&command(
                "character",
                json!([option("character", "Venti")]),
            )))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(duplicate["type"], 4);
    assert!(
        duplicate["data"]["content"]
            .as_str()
            .unwrap()
            .contains("already accepted")
    );
    assert_eq!(counts(&pool).await, before);
    assert_eq!(
        router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/images/{id}/content"))
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/characters")
                    .header("origin", ORIGIN)
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/discord/interactions")
                    .header("content-length", (1024 * 1024 + 1).to_string())
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        413
    );
    runtime.shutdown().await;
    assert_eq!(runtime.active_jobs(), 0);
}
#[sqlx::test]
async fn attachment_actual_bounds_page_fallback_and_payload_limits(pool: PgPool) {
    let f = fixture(&pool).await;
    let root = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(root.path()).await.unwrap();
    let good = artwork(&pool, &storage, &f, 10, vec![f.link, f.venti]).await;
    let missing = artwork(&pool, &storage, &f, 11, vec![f.link]).await;
    let missing = images::get(&pool, missing).await.unwrap();
    storage.remove(&missing.storage_key).await.unwrap();
    let good_image = images::get(&pool, good).await.unwrap();
    assert!(responses::preview(&storage, &good_image, 1).await.is_err());
    assert!(responses::preview(&storage, &good_image, 0).await.is_err());
    let cmd = commands::parse(
        &command(
            "images",
            json!([option("character", &format!("#{}", f.link))]),
        )["data"],
    )
    .unwrap();
    let r = responses::execute(&pool, &storage, ORIGIN, &cmd, 100000)
        .await
        .unwrap();
    assert!(r.attachment.is_none());
    assert!(
        r.payload["embeds"][0]["fields"][0]["name"]
            .as_str()
            .unwrap()
            .contains(&missing.id.to_string())
    );
    assert_eq!(
        r.payload["embeds"][0]["fields"].as_array().unwrap().len(),
        3
    );
    assert!(
        r.payload["embeds"][0]["description"]
            .as_str()
            .unwrap()
            .contains("Stored file is missing")
    );
    assert!(
        !r.payload["components"][0]["components"][1]["disabled"]
            .as_bool()
            .unwrap()
    );
    // Replace the generated file with a larger regular file: byte_size alone must not suffice.
    tokio::fs::write(root.path().join(&good_image.storage_key), vec![7; 1001])
        .await
        .unwrap();
    assert!(
        responses::preview(&storage, &good_image, 1000)
            .await
            .is_err()
    );
    let random =
        commands::parse(&command("random-image", json!([option("character", "Venti")]))["data"])
            .unwrap();
    let r = responses::execute(&pool, &storage, ORIGIN, &random, 1000)
        .await
        .unwrap();
    assert!(r.attachment.is_none());
    assert!(
        r.payload["embeds"][0]["description"]
            .as_str()
            .unwrap()
            .contains("No preview delivered")
    );
    assert!(
        r.payload["embeds"][0]["fields"][0]["name"]
            .as_str()
            .unwrap()
            .contains(&good.to_string())
    );
    sqlx::query("UPDATE characters SET description=$1 WHERE id=$2")
        .bind("😀 <@333> **text** https://evil.example/ ".repeat(250))
        .bind(f.link)
        .execute(&pool)
        .await
        .unwrap();
    let c = commands::parse(
        &command(
            "character",
            json!([option("character", &format!("#{}", f.link))]),
        )["data"],
    )
    .unwrap();
    let r = responses::execute(&pool, &storage, ORIGIN, &c, 1000)
        .await
        .unwrap();
    payload_bounds(&r.payload);
    assert!(!r.payload.to_string().contains("<@333>"));
    assert!(r.payload.to_string().contains('…'));
    assert!(!r.payload.to_string().contains("https://evil.example"));
    // More than five exact-name candidates are never silently resolved.
    for n in 0..6 {
        let franchise = record(&pool, Kind::Franchises, &format!("Other{n}"), None).await;
        record(&pool, Kind::Characters, "Link", Some(franchise)).await;
    }
    match retrieval::resolve(&pool, "Link", None).await.unwrap() {
        Resolution::Ambiguous { candidates, more } => {
            assert_eq!(candidates.len(), 5);
            assert!(more);
        }
        _ => panic!("expected ambiguity"),
    }
}
#[sqlx::test]
async fn fast_deferral_bounded_admission_replay_and_drain(pool: PgPool) {
    // Hold an exclusive table lock so domain work is deliberately blocked, independently of timing.
    fixture(&pool).await;
    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE characters IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(root.path()).await.unwrap();
    let mut fake = fake_api(vec![], None).await;
    let runtime = discord::Runtime::new(
        config(),
        pool.clone(),
        storage.clone(),
        ORIGIN.into(),
        false,
        fake.client.clone(),
    );
    let router = web::router_with_discord(
        pool,
        ORIGIN.into(),
        storage,
        Environment::Production,
        false,
        Some(runtime.clone()),
    );
    for n in 0..4 {
        let mut p = command("characters", json!([]));
        p["id"] = json!((2000 + n).to_string());
        let response =
            tokio::time::timeout(Duration::from_secs(1), router.clone().oneshot(request(&p)))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            runtime.active_jobs(),
            n as usize,
            "No job starts before the deferred body is consumed"
        );
        assert_eq!(json_response(response).await["type"], 5);
    }
    assert_eq!(runtime.active_jobs(), 4);
    let mut busy = command("characters", json!([]));
    busy["id"] = json!("3000");
    assert!(json_response(router.clone().oneshot(request(&busy)).await.unwrap()).await["data"]["content"].as_str().unwrap().contains("busy"));
    assert!(fake.events.try_recv().is_err());
    lock.rollback().await.unwrap();
    for _ in 0..4 {
        delivery(&mut fake).await;
    }
    runtime.shutdown().await;
    assert_eq!(runtime.active_jobs(), 0);
}
#[tokio::test]
async fn outbound_retries_registration_and_safe_errors() {
    let mut fake = fake_api(vec![429, 200], None).await;
    let payload = responses::message("Private test");
    fake.client
        .edit(111, "test-only-interaction-token", &payload, None)
        .await
        .unwrap();
    assert_eq!(delivery(&mut fake).await.method, axum::http::Method::PATCH);
    assert_eq!(delivery(&mut fake).await.method, axum::http::Method::PATCH);
    let mut failure = fake_api(vec![500], None).await;
    let err = failure
        .client
        .edit(111, "test-only-interaction-token", &payload, None)
        .await
        .unwrap_err();
    assert!(matches!(err, Failure::Status(500)));
    assert!(!err.to_string().contains("test-only-interaction-token"));
    delivery(&mut failure).await;
    assert!(failure.events.try_recv().is_err());
    let mut registration = fake_api(vec![], None).await;
    let names = registration
        .client
        .register(111, 222, "test-only-bot-token")
        .await
        .unwrap();
    assert_eq!(names.len(), 4);
    for spec in commands::manifest() {
        let d = delivery(&mut registration).await;
        assert_eq!(d.method, axum::http::Method::POST);
        assert_eq!(d.path, "/api/v10/applications/111/guilds/222/commands");
        assert_eq!(
            serde_json::from_slice::<Value>(&d.body).unwrap(),
            serde_json::to_value(spec).unwrap()
        );
    }
    assert!(Client::local_test_server("192.0.2.1:80".parse().unwrap()).is_err());
    let blocked = Arc::new(tokio::sync::Semaphore::new(0));
    let mut stalled = fake_api(vec![], Some(blocked.clone())).await;
    let started = Instant::now();
    let err = stalled
        .client
        .edit(111, "test-only-interaction-token", &payload, None)
        .await
        .unwrap_err();
    assert!(matches!(err, Failure::Timeout));
    assert!(started.elapsed() < Duration::from_secs(15));
    delivery(&mut stalled).await;
    assert!(stalled.events.try_recv().is_err());
    blocked.add_permits(1);
}

#[test]
fn offline_cli_and_enabled_startup_validation() {
    let cwd = tempfile::tempdir().unwrap();
    let mut print = std::process::Command::new(env!("CARGO_BIN_EXE_the-archive"));
    print
        .current_dir(cwd.path())
        .env_clear()
        .env("DATABASE_URL", "invalid")
        .env("IMAGE_STORAGE_DIR", "/unavailable")
        .env("DISCORD_ENABLED", "true")
        .args(["discord", "commands", "print"]);
    let output = print.output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        serde_json::to_value(commands::manifest()).unwrap()
    );
    let mut serve = std::process::Command::new(env!("CARGO_BIN_EXE_the-archive"));
    serve
        .current_dir(cwd.path())
        .env_clear()
        .env("APP_ENV", "development")
        .env("DATABASE_URL", "postgres://unused@127.0.0.1:9/unused")
        .env("IMAGE_STORAGE_DIR", "/unavailable")
        .env("DISCORD_ENABLED", "true");
    let output = serve.output().unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("DISCORD_APPLICATION_ID is required"));
    assert!(!error.contains("Database connection failed"));
}

#[sqlx::test]
async fn failed_delivery_attempts_error_edit_and_releases_admission(pool: PgPool) {
    fixture(&pool).await;
    let root = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(root.path()).await.unwrap();
    let mut fake = fake_api(vec![500, 200], None).await;
    let runtime = discord::Runtime::new(
        config(),
        pool.clone(),
        storage.clone(),
        ORIGIN.into(),
        false,
        fake.client.clone(),
    );
    let router = web::router_with_discord(
        pool,
        ORIGIN.into(),
        storage,
        Environment::Production,
        false,
        Some(runtime.clone()),
    );
    assert_eq!(
        json_response(
            router
                .oneshot(request(&command("characters", json!([]))))
                .await
                .unwrap()
        )
        .await["type"],
        5
    );
    let (first, _) = decode(delivery(&mut fake).await).await;
    assert!(!first["embeds"].as_array().unwrap().is_empty());
    let (error, file) = decode(delivery(&mut fake).await).await;
    assert!(file.is_none());
    assert!(error["content"].as_str().unwrap().contains("rerun"));
    assert_eq!(error["allowed_mentions"]["parse"], json!([]));
    runtime.shutdown().await;
    assert_eq!(runtime.active_jobs(), 0);
}

#[sqlx::test]
async fn shutdown_cancels_blocked_jobs_and_closes_admission(pool: PgPool) {
    fixture(&pool).await;
    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE characters IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(root.path()).await.unwrap();
    let mut fake = fake_api(vec![], None).await;
    let runtime = discord::Runtime::new(
        config(),
        pool.clone(),
        storage.clone(),
        ORIGIN.into(),
        false,
        fake.client.clone(),
    );
    let router = web::router_with_discord(
        pool,
        ORIGIN.into(),
        storage,
        Environment::Production,
        false,
        Some(runtime.clone()),
    );
    assert_eq!(
        json_response(
            router
                .clone()
                .oneshot(request(&command("characters", json!([]))))
                .await
                .unwrap()
        )
        .await["type"],
        5
    );
    tokio::time::timeout(Duration::from_secs(20), runtime.shutdown())
        .await
        .expect("Bounded shutdown must cancel the deliberately locked query");
    assert_eq!(runtime.active_jobs(), 0);
    assert!(fake.events.try_recv().is_err());
    let mut after = command("characters", json!([]));
    after["id"] = json!("1001");
    assert!(
        json_response(router.oneshot(request(&after)).await.unwrap()).await["data"]["content"]
            .as_str()
            .unwrap()
            .contains("busy")
    );
    lock.rollback().await.unwrap();
}

#[path = "support/discord_viewer.rs"]
mod viewer_tests;
