use super::*;
use the_archive::{app::retrieval::Direction, discord::viewer};

fn component(id: u64, character: i64, image: i64, direction: Direction) -> Value {
    json!({"id":id.to_string(),"application_id":"111","type":3,"guild_id":"222","channel_id":"444",
        "member":{"user":{"id":"333"}},"token":format!("click-token-{id}"),"attachment_size_limit":100000,
        "data":{"component_type":2,"custom_id":viewer::custom_id(character,image,direction)},
        "message":{"id":"555","channel_id":"444","application_id":"111","webhook_id":"111",
            "author":{"id":"111","bot":true},"flags":0,"components":viewer::controls(character,image,true,true)}})
}
fn runtime(
    pool: &PgPool,
    storage: &LocalStorage,
    client: Client,
    maintenance: bool,
) -> (Arc<discord::Runtime>, Router) {
    let runtime = discord::Runtime::new(
        config(),
        pool.clone(),
        storage.clone(),
        ORIGIN.into(),
        maintenance,
        client,
    );
    let router = web::router_with_discord(
        pool.clone(),
        ORIGIN.into(),
        storage.clone(),
        Environment::Production,
        maintenance,
        Some(runtime.clone()),
    );
    (runtime, router)
}
async fn idle(runtime: &discord::Runtime) {
    for _ in 0..1000 {
        if runtime.active_jobs() == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("Viewer job did not settle");
}
fn assert_private(d: Delivery, token: &str, contains: &str) {
    assert_eq!(d.method, axum::http::Method::POST);
    assert_eq!(d.path, format!("/api/v10/webhooks/111/{token}"));
    let body: Value = serde_json::from_slice(&d.body).unwrap();
    assert_eq!(body["flags"], 64);
    assert_eq!(body["allowed_mentions"]["parse"], json!([]));
    assert!(body["content"].as_str().unwrap().contains(contains));
    assert_eq!(body["attachments"], json!([]));
}
#[test]
fn viewer_parser_manifest_and_message_identity() {
    let good = component(1001, 1, 2, Direction::Next);
    assert!(viewer::parse(&good, 111).is_some());
    assert!(viewer::custom_id(i64::MAX, i64::MAX, Direction::Previous).len() <= 100);
    for (path, value) in [
        ("/data/component_type", json!(3)),
        ("/message/application_id", json!("112")),
        ("/message/author/id", json!("112")),
        ("/message/author/bot", json!(false)),
        ("/message/webhook_id", json!("112")),
        ("/message/channel_id", json!("445")),
        ("/message/id", json!(0)),
        ("/message/flags", json!(64)),
        ("/message/flags", json!(32768)),
        ("/message/components/0/type", json!(2)),
        (
            "/message/components/0/components/0/custom_id",
            json!("av1:2:2:p"),
        ),
        ("/message/components/0/components/1/disabled", json!(true)),
        ("/message/components/0/components/1/style", json!(5)),
        (
            "/message/components/0/components/1/disabled",
            json!("false"),
        ),
    ] {
        let mut bad = good.clone();
        *bad.pointer_mut(path).unwrap() = value;
        assert!(viewer::parse(&bad, 111).is_none(), "{path}");
    }
    for id in [
        "",
        "av2:1:2:n",
        "av1:0:2:n",
        "av1:1:-2:n",
        "av1:1:2:wrap",
        "av1:1:2:n:extra",
        "av1:01:2:n",
        "av1:9223372036854775808:2:n",
        &"a".repeat(101),
    ] {
        let mut bad = good.clone();
        bad["data"]["custom_id"] = json!(id);
        assert!(viewer::parse(&bad, 111).is_none());
    }
    // Discord may omit the false default; malformed values still fail.
    let mut default = good.clone();
    default["message"]["components"][0]["components"][1]
        .as_object_mut()
        .unwrap()
        .remove("disabled");
    assert!(viewer::parse(&default, 111).is_some());
    let mut nested = good.clone();
    nested.as_object_mut().unwrap().remove("channel_id");
    nested["channel"] = json!({"id":"444"});
    assert!(viewer::parse(&nested, 111).is_some());
    nested["channel"]["id"] = json!("445");
    assert!(viewer::parse(&nested, 111).is_none());

    let spec = commands::manifest()
        .into_iter()
        .find(|s| s.name == "images")
        .unwrap();
    assert_eq!(
        spec.options.iter().map(|s| s.name).collect::<Vec<_>>(),
        vec!["character", "franchise"]
    );
    assert!(
        commands::parse(
            &command(
                "images",
                json!([
                    option("character", "Link"),
                    json!({"name":"page","type":4,"value":1})
                ])
            )["data"]
        )
        .is_err()
    );
    assert!(
        commands::parse(
            &command(
                "characters",
                json!([json!({"name":"page","type":4,"value":2})])
            )["data"]
        )
        .is_ok()
    );
}
#[sqlx::test]
async fn keyset_order_endpoints_membership_and_canonical_changes(pool: PgPool) {
    let f = fixture(&pool).await;
    let dir = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(dir.path()).await.unwrap();
    let empty = retrieval::artwork_view(&pool, f.link, None).await.unwrap();
    assert!(empty.image.is_none());
    assert_eq!(empty.total, 0);
    assert!(!empty.previous && !empty.next);
    let a = artwork(&pool, &storage, &f, 1, vec![f.link, f.venti]).await;
    let single = retrieval::artwork_view(&pool, f.venti, None).await.unwrap();
    assert_eq!(single.image.unwrap().id, a);
    assert!(!single.previous && !single.next);
    let b = artwork(&pool, &storage, &f, 2, vec![f.link]).await;
    let c = artwork(&pool, &storage, &f, 3, vec![f.link]).await;
    sqlx::query("UPDATE images SET created_at='2025-01-01T00:00:00Z'")
        .execute(&pool)
        .await
        .unwrap();
    let initial = retrieval::artwork_view(&pool, f.link, None).await.unwrap();
    assert_eq!(initial.image.unwrap().id, c);
    assert_eq!((initial.position, initial.total), (1, 3));
    assert!(!initial.previous && initial.next);
    for (cursor, direction, expected, position) in [
        (c, Direction::Next, b, 2),
        (b, Direction::Next, a, 3),
        (a, Direction::Next, a, 3),
        (a, Direction::Previous, b, 2),
        (b, Direction::Previous, c, 1),
        (c, Direction::Previous, c, 1),
    ] {
        let view = retrieval::artwork_view(&pool, f.link, Some((cursor, direction)))
            .await
            .unwrap();
        assert_eq!(view.image.unwrap().id, expected);
        assert_eq!(view.position, position);
        assert_eq!(view.previous, position > 1);
        assert_eq!(view.next, position < 3);
    }
    // Same cursor is the same move, not a per-user incrementing offset.
    assert_eq!(
        retrieval::artwork_view(&pool, f.link, Some((c, Direction::Next)))
            .await
            .unwrap()
            .image
            .unwrap()
            .id,
        b
    );
    let newest = artwork(&pool, &storage, &f, 4, vec![f.link]).await;
    assert_eq!(
        retrieval::artwork_view(&pool, f.link, Some((c, Direction::Previous)))
            .await
            .unwrap()
            .image
            .unwrap()
            .id,
        newest
    );
    sqlx::query("DELETE FROM images WHERE id=$1")
        .bind(b)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        retrieval::artwork_view(&pool, f.link, Some((c, Direction::Next)))
            .await
            .unwrap()
            .image
            .unwrap()
            .id,
        a
    );
    assert!(
        retrieval::artwork_view(&pool, f.venti, Some((c, Direction::Next)))
            .await
            .is_err()
    );
    sqlx::query("DELETE FROM image_characters WHERE image_id=$1 AND character_id=$2")
        .bind(c)
        .bind(f.link)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        retrieval::artwork_view(&pool, f.link, Some((c, Direction::Next)))
            .await
            .is_err()
    );
    sqlx::query("DELETE FROM images WHERE id=$1")
        .bind(c)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        retrieval::artwork_view(&pool, f.link, Some((c, Direction::Next)))
            .await
            .is_err()
    );
    sqlx::query("DELETE FROM characters WHERE id=$1")
        .bind(f.link)
        .execute(&pool)
        .await
        .unwrap();
    assert!(retrieval::artwork_view(&pool, f.link, None).await.is_err());
    assert_eq!(images::get(&pool, a).await.unwrap().id, a);
}
#[sqlx::test]
async fn character_random_artwork_preserves_curated_data_and_never_resamples(pool: PgPool) {
    let f = fixture(&pool).await;
    let dir = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(dir.path()).await.unwrap();
    sqlx::query("UPDATE characters SET description='A curated bard description.' WHERE id=$1")
        .bind(f.venti)
        .execute(&pool)
        .await
        .unwrap();
    let chosen = artwork(&pool, &storage, &f, 10, vec![f.venti, f.link]).await;
    artwork(&pool, &storage, &f, 11, vec![f.link]).await;
    let cmd = commands::parse(&command("character", json!([option("character", "Venti")]))["data"])
        .unwrap();
    let before = counts(&pool).await;
    let reply = responses::execute(&pool, &storage, ORIGIN, &cmd, 100000)
        .await
        .unwrap();
    assert_eq!(reply.attachment.unwrap().bytes, png(10));
    assert!(
        reply.payload["embeds"][0]["description"]
            .as_str()
            .unwrap()
            .contains("A curated bard description.")
    );
    let fields = reply.payload["embeds"][0]["fields"].to_string();
    assert!(fields.contains("Alice") && fields.contains("Favorite") && fields.contains("Kin"));
    assert!(fields.contains(&format!("/images/{chosen}")));
    payload_bounds(&reply.payload);
    // Only Venti's chosen member exceeds the reported cap; unrelated deliverable artwork must not replace it.
    let reply = responses::execute(&pool, &storage, ORIGIN, &cmd, 1)
        .await
        .unwrap();
    assert!(reply.attachment.is_none());
    assert!(
        reply.payload["embeds"][0]["description"]
            .as_str()
            .unwrap()
            .contains("File exceeds delivery limit")
    );
    assert!(
        reply.payload["embeds"][0]["fields"]
            .to_string()
            .contains(&chosen.to_string())
    );
    let selected = images::get(&pool, chosen).await.unwrap();
    storage.remove(&selected.storage_key).await.unwrap();
    let reply = responses::execute(&pool, &storage, ORIGIN, &cmd, 100000)
        .await
        .unwrap();
    assert!(reply.attachment.is_none());
    assert!(
        reply.payload["embeds"][0]["description"]
            .as_str()
            .unwrap()
            .contains("Stored file is missing")
    );
    let empty = commands::parse(
        &command(
            "character",
            json!([option("character", &format!("#{}", f.other_link))]),
        )["data"],
    )
    .unwrap();
    let reply = responses::execute(&pool, &storage, ORIGIN, &empty, 100000)
        .await
        .unwrap();
    assert!(reply.attachment.is_none());
    assert!(
        reply
            .payload
            .to_string()
            .contains("No associated artwork yet")
    );
    assert!(
        reply
            .payload
            .to_string()
            .contains("No curated description yet")
    );
    let empty = commands::parse(
        &command(
            "images",
            json!([option("character", &format!("#{}", f.other_link))]),
        )["data"],
    )
    .unwrap();
    let reply = responses::execute(&pool, &storage, ORIGIN, &empty, 100000)
        .await
        .unwrap();
    assert_eq!(reply.payload["components"], json!([]));
    let ambiguous =
        commands::parse(&command("character", json!([option("character", "Link")]))["data"])
            .unwrap();
    assert!(
        responses::execute(&pool, &storage, ORIGIN, &ambiguous, 100000)
            .await
            .unwrap()
            .payload
            .to_string()
            .contains("ambiguous")
    );
    let long_origin = format!(
        "https://{}.{}.{}.{}.example",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(50)
    );
    sqlx::query("UPDATE characters SET name=$1 WHERE id=$2")
        .bind("Long".repeat(50))
        .bind(f.venti)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE franchises SET name=$1 WHERE id=(SELECT franchise_id FROM characters WHERE id=$2)",
    )
    .bind("Franchise".repeat(22))
    .bind(f.venti)
    .execute(&pool)
    .await
    .unwrap();
    let long = commands::parse(
        &command(
            "images",
            json!([option("character", &format!("#{}", f.venti))]),
        )["data"],
    )
    .unwrap();
    let reply = responses::execute(&pool, &storage, &long_origin, &long, 1)
        .await
        .unwrap();
    payload_bounds(&reply.payload);
    let desc = reply.payload["embeds"][0]["description"].as_str().unwrap();
    assert!(desc.contains("Image 1 of 1") && desc.contains("No preview delivered"));
    assert!(desc.contains(&format!("{long_origin}/images?character={}", f.venti)));
    assert_eq!(
        reply.payload["components"],
        viewer::controls(f.venti, chosen, false, false)
    );
    assert!(
        reply.payload["embeds"][0]["fields"][0]["value"]
            .as_str()
            .unwrap()
            .contains("Artist:")
    );
    assert!(
        reply.payload["embeds"][0]["fields"][0]["value"]
            .as_str()
            .unwrap()
            .contains("1×1")
    );
    assert!(
        reply.payload["embeds"][0]["fields"][1]["value"]
            .as_str()
            .unwrap()
            .contains(&format!("{long_origin}/images/{chosen}"))
    );
    assert_eq!(counts(&pool).await, before);
}
#[tokio::test]
async fn component_denials_and_maintenance_do_not_touch_database() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:9/unused")
        .unwrap();
    pool.close().await;
    let dir = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(dir.path()).await.unwrap();
    for maintenance in [false, true] {
        let mut fake = fake_api(vec![], None).await;
        let (runtime, router) = runtime(&pool, &storage, fake.client.clone(), maintenance);
        let good = component(1001, 1, 2, Direction::Next);
        for (path, value) in [
            ("/application_id", json!("112")),
            ("/guild_id", json!("223")),
            ("/member/user/id", json!("335")),
            ("/message/application_id", json!("112")),
            ("/message/author/id", json!("112")),
            ("/data/custom_id", json!("bad")),
            ("/data/component_type", json!(3)),
            ("/member", json!(null)),
            ("/guild_id", json!(null)),
        ] {
            let mut bad = good.clone();
            *bad.pointer_mut(path).unwrap() = value;
            let denied = json_response(router.clone().oneshot(request(&bad)).await.unwrap()).await;
            assert_eq!(denied["type"], 4);
            assert_eq!(denied["data"]["flags"], 64);
            assert_eq!(denied["data"]["allowed_mentions"]["parse"], json!([]));
            assert_eq!(
                denied["data"]["content"],
                "This interaction is not available. Check access and command configuration."
            );
        }
        if maintenance {
            let notice = json_response(router.clone().oneshot(request(&good)).await.unwrap()).await;
            assert_eq!(notice["data"]["flags"], 64);
            assert!(
                notice["data"]["content"]
                    .as_str()
                    .unwrap()
                    .contains("maintenance")
            );
        }
        let mut bad_signature = request(&good);
        bad_signature
            .headers_mut()
            .insert("x-signature-ed25519", "00".repeat(64).parse().unwrap());
        assert_eq!(
            router
                .clone()
                .oneshot(bad_signature)
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            discord::routes(None)
                .oneshot(request(&good))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(runtime.active_jobs(), 0);
        assert!(fake.events.try_recv().is_err());
        runtime.shutdown().await;
    }
}
#[sqlx::test]
async fn shared_click_edits_fresh_token_replaces_files_fallback_and_restart(pool: PgPool) {
    let f = fixture(&pool).await;
    let dir = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(dir.path()).await.unwrap();
    let a = artwork(&pool, &storage, &f, 21, vec![f.link, f.venti]).await;
    let b = artwork(&pool, &storage, &f, 22, vec![f.link]).await;
    let c = artwork(&pool, &storage, &f, 23, vec![f.link]).await;
    let before = counts(&pool).await;
    let mut fake = fake_api(vec![], None).await;
    let (first, router) = runtime(&pool, &storage, fake.client.clone(), false);
    let initial = command(
        "images",
        json!([option("character", &format!("#{}", f.link))]),
    );
    assert_eq!(
        json_response(router.clone().oneshot(request(&initial)).await.unwrap()).await["type"],
        5
    );
    let (view, file) = decode(delivery(&mut fake).await).await;
    assert_eq!(file.unwrap().2, png(23));
    assert_eq!(view["components"], viewer::controls(f.link, c, false, true));
    first.shutdown().await;
    drop(router);
    drop(first);
    // Fresh runtime has no viewer session or initial interaction token. Both owners can move it.
    let (runtime, router) = runtime(&pool, &storage, fake.client.clone(), false);
    for (id, cursor, direction, chosen, color, user) in [
        (1002, c, Direction::Next, b, 22, "334"),
        (1003, b, Direction::Next, a, 21, "333"),
        (1004, a, Direction::Previous, b, 22, "334"),
        (1005, c, Direction::Next, b, 22, "333"),
    ] {
        let mut click = component(id, f.link, cursor, direction);
        click["member"]["user"]["id"] = json!(user);
        let ack = json_response(router.clone().oneshot(request(&click)).await.unwrap()).await;
        assert_eq!(ack, json!({"type":6}));
        let (view, file) = decode_at(delivery(&mut fake).await, &format!("click-token-{id}")).await;
        assert_eq!(file.unwrap().2, png(color));
        assert_eq!(view["attachments"].as_array().unwrap().len(), 1);
        assert_eq!(view["attachments"][0]["id"], 0);
        assert!(
            view["embeds"][0]["image"]["url"]
                .as_str()
                .unwrap()
                .contains(&chosen.to_string())
        );
        assert_eq!(
            view["components"],
            viewer::controls(f.link, chosen, chosen != c, chosen != a)
        );
        payload_bounds(&view);
        idle(&runtime).await;
    }
    let mut denied = component(1006, f.link, b, Direction::Next);
    denied["member"]["user"]["id"] = json!("999");
    assert_eq!(
        json_response(router.clone().oneshot(request(&denied)).await.unwrap()).await["data"]["flags"],
        64
    );
    assert!(fake.events.try_recv().is_err());
    let mut click = component(1007, f.link, c, Direction::Next);
    click["attachment_size_limit"] = json!(1);
    assert_eq!(
        json_response(router.clone().oneshot(request(&click)).await.unwrap()).await["type"],
        6
    );
    let (view, file) = decode_at(delivery(&mut fake).await, "click-token-1007").await;
    assert!(file.is_none());
    assert_eq!(view["attachments"], json!([]));
    assert!(view["embeds"][0].get("image").is_none());
    assert_eq!(view["components"], viewer::controls(f.link, b, true, true));
    assert!(view.to_string().contains("File exceeds delivery limit"));
    idle(&runtime).await;
    // Actual replacement-file length, not only byte_size metadata, controls delivery.
    tokio::fs::write(
        dir.path()
            .join(images::get(&pool, b).await.unwrap().storage_key),
        vec![7; 100001],
    )
    .await
    .unwrap();
    let click = component(1009, f.link, c, Direction::Next);
    assert_eq!(
        json_response(router.clone().oneshot(request(&click)).await.unwrap()).await["type"],
        6
    );
    let (view, file) = decode_at(delivery(&mut fake).await, "click-token-1009").await;
    assert!(file.is_none());
    assert_eq!(view["attachments"], json!([]));
    assert!(view["embeds"][0].get("image").is_none());
    assert_eq!(view["components"], viewer::controls(f.link, b, true, true));
    idle(&runtime).await;
    storage
        .remove(&images::get(&pool, a).await.unwrap().storage_key)
        .await
        .unwrap();
    let click = component(1008, f.link, b, Direction::Next);
    assert_eq!(
        json_response(router.clone().oneshot(request(&click)).await.unwrap()).await["type"],
        6
    );
    let (view, file) = decode_at(delivery(&mut fake).await, "click-token-1008").await;
    assert!(file.is_none());
    assert_eq!(view["attachments"], json!([]));
    assert_eq!(view["components"], viewer::controls(f.link, a, true, false));
    idle(&runtime).await;
    assert_eq!(counts(&pool).await, before);
    assert!(fake.events.try_recv().is_err());
    runtime.shutdown().await;
}
#[sqlx::test]
async fn stale_forged_membership_and_failed_delivery_preserve_shared_message(pool: PgPool) {
    let f = fixture(&pool).await;
    let dir = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(dir.path()).await.unwrap();
    let a = artwork(&pool, &storage, &f, 31, vec![f.link]).await;
    let b = artwork(&pool, &storage, &f, 32, vec![f.link]).await;
    let mut fake = fake_api(vec![200, 500, 200, 200], None).await;
    let (runtime, router) = runtime(&pool, &storage, fake.client.clone(), false);
    // Authentic signed controls still cannot address another character's image.
    let forged = component(2001, f.venti, b, Direction::Next);
    assert_eq!(
        json_response(router.clone().oneshot(request(&forged)).await.unwrap()).await["type"],
        6
    );
    assert_private(delivery(&mut fake).await, "click-token-2001", "stale");
    idle(&runtime).await;
    let valid = component(2002, f.link, b, Direction::Next);
    assert_eq!(
        json_response(router.clone().oneshot(request(&valid)).await.unwrap()).await["type"],
        6
    );
    let (view, file) = decode_at(delivery(&mut fake).await, "click-token-2002").await;
    assert!(file.is_some());
    assert_eq!(view["components"], viewer::controls(f.link, a, true, false));
    assert_private(
        delivery(&mut fake).await,
        "click-token-2002",
        "could not be confirmed",
    );
    idle(&runtime).await;
    // The released guard allows a new click after failure, with no public error edit.
    let valid = component(2003, f.link, b, Direction::Next);
    assert_eq!(
        json_response(router.clone().oneshot(request(&valid)).await.unwrap()).await["type"],
        6
    );
    decode_at(delivery(&mut fake).await, "click-token-2003").await;
    idle(&runtime).await;
    sqlx::query("DELETE FROM image_characters WHERE character_id=$1 AND image_id=$2")
        .bind(f.link)
        .bind(b)
        .execute(&pool)
        .await
        .unwrap();
    let stale = component(2004, f.link, b, Direction::Next);
    assert_eq!(
        json_response(router.clone().oneshot(request(&stale)).await.unwrap()).await["type"],
        6
    );
    assert_private(delivery(&mut fake).await, "click-token-2004", "stale");
    idle(&runtime).await;
    assert!(fake.events.try_recv().is_err());
    runtime.shutdown().await;
}
#[sqlx::test]
async fn per_message_busy_replay_fast_ack_and_shutdown_release(pool: PgPool) {
    let f = fixture(&pool).await;
    let dir = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(dir.path()).await.unwrap();
    let a = artwork(&pool, &storage, &f, 41, vec![f.link]).await;
    artwork(&pool, &storage, &f, 42, vec![f.link]).await;
    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE images IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let mut fake = fake_api(vec![], None).await;
    let (runtime, router) = runtime(&pool, &storage, fake.client.clone(), false);
    let click = component(3001, f.link, a, Direction::Previous);
    let response = tokio::time::timeout(
        Duration::from_secs(1),
        router.clone().oneshot(request(&click)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(runtime.active_jobs(), 0);
    assert_eq!(json_response(response).await, json!({"type":6}));
    assert_eq!(runtime.active_jobs(), 1);
    assert!(fake.events.try_recv().is_err());
    let other = component(3002, f.link, a, Direction::Previous);
    let notice = json_response(router.clone().oneshot(request(&other)).await.unwrap()).await;
    assert_eq!(notice["data"]["flags"], 64);
    assert!(
        notice["data"]["content"]
            .as_str()
            .unwrap()
            .contains("already being updated")
    );
    lock.rollback().await.unwrap();
    decode_at(delivery(&mut fake).await, "click-token-3001").await;
    idle(&runtime).await;
    let replay = json_response(router.clone().oneshot(request(&click)).await.unwrap()).await;
    assert!(
        replay["data"]["content"]
            .as_str()
            .unwrap()
            .contains("already accepted")
    );
    // Different messages still obey the global four-job bound. Unread bodies release their guards.
    let mut held = vec![];
    for id in 4001..4005 {
        let mut p = component(id, f.link, a, Direction::Previous);
        p["message"]["id"] = json!(id.to_string());
        held.push(router.clone().oneshot(request(&p)).await.unwrap());
    }
    let mut fifth = component(4005, f.link, a, Direction::Previous);
    fifth["message"]["id"] = json!("4005");
    assert!(json_response(router.clone().oneshot(request(&fifth)).await.unwrap()).await["data"]["content"].as_str().unwrap().contains("busy"));
    drop(held);
    let allowed = router.clone().oneshot(request(&other)).await.unwrap();
    assert_eq!(json_response(allowed).await["type"], 6);
    decode_at(delivery(&mut fake).await, "click-token-3002").await;
    idle(&runtime).await;
    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE images IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let p = component(5001, f.link, a, Direction::Previous);
    assert_eq!(
        json_response(router.clone().oneshot(request(&p)).await.unwrap()).await["type"],
        6
    );
    tokio::time::timeout(Duration::from_secs(20), runtime.shutdown())
        .await
        .unwrap();
    assert_eq!(runtime.active_jobs(), 0);
    assert!(fake.events.try_recv().is_err());
    lock.rollback().await.unwrap();
}

#[sqlx::test]
async fn navigation_timeout_private_notice_releases_message_guard(pool: PgPool) {
    let f = fixture(&pool).await;
    let dir = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(dir.path()).await.unwrap();
    let a = artwork(&pool, &storage, &f, 51, vec![f.link]).await;
    artwork(&pool, &storage, &f, 52, vec![f.link]).await;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let mut fake = fake_api(vec![], Some(gate.clone())).await;
    let (runtime, router) = runtime(&pool, &storage, fake.client.clone(), false);
    let click = component(6001, f.link, a, Direction::Previous);
    assert_eq!(
        json_response(router.clone().oneshot(request(&click)).await.unwrap()).await["type"],
        6
    );
    decode_at(delivery(&mut fake).await, "click-token-6001").await;
    let followup = tokio::time::timeout(Duration::from_secs(15), fake.events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_private(followup, "click-token-6001", "could not be confirmed");
    gate.add_permits(3);
    idle(&runtime).await;
    assert!(fake.events.try_recv().is_err());
    let click = component(6002, f.link, a, Direction::Previous);
    assert_eq!(
        json_response(router.clone().oneshot(request(&click)).await.unwrap()).await["type"],
        6
    );
    decode_at(delivery(&mut fake).await, "click-token-6002").await;
    idle(&runtime).await;
    runtime.shutdown().await;
}
