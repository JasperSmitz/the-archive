mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use image::{ExtendedColorType, ImageEncoder};
use sqlx::PgPool;
use std::{
    collections::HashSet,
    time::{Duration, SystemTime},
};
use tempfile::TempDir;
use the_archive::{
    app::{
        catalog::{self, Input},
        images::{
            self, Metadata, Upload,
            validation::{self, Limits},
        },
    },
    error::Error,
    models::Kind,
    storage::{LocalStorage, ORPHAN_GRACE},
};
use tower::ServiceExt;
fn fixture(format: &str, color: u8) -> Vec<u8> {
    let pixels = [color, 22, 33].repeat(4);
    let mut bytes = vec![];
    match format {
        "png" => image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&pixels, 2, 2, ExtendedColorType::Rgb8)
            .unwrap(),
        "jpg" => image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .write_image(&pixels, 2, 2, ExtendedColorType::Rgb8)
            .unwrap(),
        "webp" => image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
            .write_image(&pixels, 2, 2, ExtendedColorType::Rgb8)
            .unwrap(),
        _ => unreachable!(),
    };
    bytes
}
fn animated_png() -> Vec<u8> {
    let mut bytes = vec![];
    {
        let mut enc = png::Encoder::new(&mut bytes, 1, 1);
        enc.set_color(png::ColorType::Rgb);
        enc.set_animated(2, 0).unwrap();
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(&[1, 2, 3]).unwrap();
        writer.write_image_data(&[4, 5, 6]).unwrap();
        writer.finish().unwrap();
    }
    bytes
}
fn animated_webp() -> Vec<u8> {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(b"WEBPVP8X");
    bytes.extend(10_u32.to_le_bytes());
    bytes.extend([2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    bytes.extend(b"ANIM");
    bytes.extend(6_u32.to_le_bytes());
    bytes.extend([0; 6]);
    let size = (bytes.len() - 8) as u32;
    bytes[4..8].copy_from_slice(&size.to_le_bytes());
    bytes
}
#[test]
fn formats_corruption_animation_and_limits() {
    let lossy = include_bytes!("fixtures/lossy.webp");
    let inspected = validation::inspect(lossy, Limits::default()).unwrap();
    assert_eq!(
        (inspected.width, inspected.height, inspected.content_type),
        (2, 2, "image/webp")
    );
    for format in ["jpg", "png", "webp"] {
        let b = fixture(format, 30);
        let v = validation::inspect(&b, Limits::default()).unwrap();
        assert_eq!((v.width, v.height), (2, 2));
        assert_eq!(v.extension, format);
        assert_eq!(v.sha256.len(), 32);
        for limits in [
            Limits {
                axis: 1,
                ..Default::default()
            },
            Limits {
                pixels: 3,
                ..Default::default()
            },
            Limits {
                allocation: 1,
                ..Default::default()
            },
        ] {
            assert!(matches!(
                validation::inspect(&b, limits),
                Err(Error::Validation(_))
            ));
        }
        assert!(matches!(
            validation::inspect(
                &b,
                Limits {
                    file_bytes: b.len() - 1,
                    ..Default::default()
                }
            ),
            Err(Error::TooLarge)
        ));
        assert!(validation::inspect(&b[..b.len() - 1], Limits::default()).is_err());
        assert!(validation::inspect(&b[..b.len() / 2], Limits::default()).is_err());
    }
    for bytes in [
        b"<svg><script/></svg>".to_vec(),
        b"GIF89a".to_vec(),
        b"not an image".to_vec(),
        vec![],
    ] {
        assert!(matches!(
            validation::inspect(&bytes, Limits::default()),
            Err(Error::Validation(_))
        ));
    }
    for bytes in [animated_png(), animated_webp()] {
        match validation::inspect(&bytes, Limits::default()) {
            Err(Error::Validation(errors)) => assert!(errors[0].1.contains("Animated")),
            other => panic!("{other:?}"),
        }
    }
    // Header bounds alone are insufficient: preserve a valid header but destroy its payload.
    let mut png = fixture("png", 31);
    let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
    png[idat + 5] ^= 0xff;
    assert!(validation::inspect(&png, Limits::default()).is_err());
    // A dishonest extended WebP canvas must not hide oversized coded dimensions.
    let original = fixture("webp", 30);
    let mut spoof = b"RIFF".to_vec();
    spoof.extend(0_u32.to_le_bytes());
    spoof.extend(b"WEBPVP8X");
    spoof.extend(10_u32.to_le_bytes());
    spoof.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    spoof.extend(&original[12..]);
    let size = (spoof.len() - 8) as u32;
    spoof[4..8].copy_from_slice(&size.to_le_bytes());
    assert!(validation::inspect(&spoof, Limits::default()).is_err());
}
#[test]
fn filename_normalization() {
    assert_eq!(validation::filename("../../art.png"), "art.png");
    assert_eq!(validation::filename("C:\\art\\nice\n\r.png"), "nice.png");
    for s in ["", "../", "..", "/\\", "\0\n"] {
        assert_eq!(validation::filename(s), "upload");
    }
    assert_eq!(validation::filename(&"é".repeat(300)).chars().count(), 255);
}
async fn storage() -> (TempDir, LocalStorage) {
    let d = tempfile::tempdir().unwrap();
    let s = LocalStorage::initialize(d.path()).await.unwrap();
    (d, s)
}
async fn add(pool: &PgPool, k: Kind, name: &str, f: Option<i64>) -> i64 {
    catalog::save(
        pool,
        k,
        None,
        &Input {
            name: name.into(),
            franchise_id: f,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}
async fn setup(pool: &PgPool) -> (i64, i64, i64) {
    let p = add(pool, Kind::People, "Uploader", None).await;
    let f = add(pool, Kind::Franchises, "Shared", None).await;
    let a = add(pool, Kind::Characters, "Link", Some(f)).await;
    let b = add(pool, Kind::Characters, "Venti", Some(f)).await;
    (p, a, b)
}
fn upload(p: i64, cs: Vec<i64>, format: &str, color: u8) -> Upload {
    Upload {
        bytes: fixture(format, color),
        original_filename: "../../misleading.svg".into(),
        metadata: Metadata {
            uploader_id: p,
            character_ids: cs,
            artist: "  Artist  ".into(),
            source_url: " https://example.test/art ".into(),
        },
    }
}
fn entries(d: &TempDir) -> Vec<String> {
    let mut v: Vec<_> = std::fs::read_dir(d.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}
#[tokio::test]
async fn storage_containment_and_orphan_cleanup() {
    let (d, s) = storage().await;
    assert!(entries(&d).is_empty());
    for key in [
        "../../escape.png",
        "/tmp/escape.png",
        "unrelated.png",
        "0.png",
        "00000000000000000000000000000000.svg",
    ] {
        assert!(s.open(key).await.is_err());
        assert!(s.remove(key).await.is_err());
    }
    assert!(s.write(b"data", "../png").await.is_err());
    let keep = s.write(b"retained", "png").await.unwrap();
    let orphan = s.write(b"orphan", "webp").await.unwrap();
    let temp = ".upload-00000000000000000000000000000000.tmp";
    std::fs::write(d.path().join(temp), b"temp").unwrap();
    std::fs::write(d.path().join("notes.txt"), b"unrelated").unwrap();
    let keys = HashSet::from([keep.clone()]);
    assert!(
        s.cleanup(&keys, false, ORPHAN_GRACE)
            .await
            .unwrap()
            .is_empty()
    );
    let old = SystemTime::now() - ORPHAN_GRACE - Duration::from_secs(60);
    for key in [&orphan, temp] {
        std::fs::File::open(d.path().join(key))
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    let dry = s.cleanup(&keys, false, ORPHAN_GRACE).await.unwrap();
    assert_eq!(dry.len(), 2);
    assert!(dry.iter().all(|o| !o.removed));
    assert_eq!(entries(&d).len(), 4);
    let applied = s.cleanup(&keys, true, ORPHAN_GRACE).await.unwrap();
    assert_eq!(applied.len(), 2);
    assert!(applied.iter().all(|o| o.removed));
    assert_eq!(entries(&d), vec![keep, "notes.txt".into()]);
    s.remove(&orphan).await.unwrap();
    let root_file = d.path().join("not-a-directory");
    std::fs::write(&root_file, b"x").unwrap();
    assert!(LocalStorage::initialize(root_file).await.is_err());
    assert!(LocalStorage::initialize("").await.is_err());
    #[cfg(unix)]
    {
        let outside = tempfile::NamedTempFile::new().unwrap();
        let key = "00000000000000000000000000000000.png";
        std::os::unix::fs::symlink(outside.path(), d.path().join(key)).unwrap();
        assert!(s.open(key).await.is_err());
        s.cleanup(&keys, true, Duration::ZERO).await.unwrap();
        assert!(
            std::fs::symlink_metadata(d.path().join(key))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        s.remove(key).await.unwrap();
        assert!(outside.path().exists());
    }
}
#[sqlx::test(migrations = "./migrations")]
async fn persistence_duplicates_memberships_and_deletion(pool: PgPool) {
    let (d, s) = storage().await;
    let (p, a, b) = setup(&pool).await;
    let first = images::upload(&pool, &s, upload(p, vec![a, b, a], "png", 1))
        .await
        .unwrap();
    assert!(!first.duplicate);
    let record = images::get(&pool, first.id).await.unwrap();
    assert_eq!(record.original_filename, "misleading.svg");
    assert_eq!(record.content_type, "image/png");
    assert!(record.storage_key.ends_with(".png"));
    assert_eq!(record.artist.as_deref(), Some("Artist"));
    assert_eq!(images::memberships(&pool, first.id).await.unwrap().len(), 2);
    assert_eq!(
        std::fs::read(d.path().join(&record.storage_key)).unwrap(),
        fixture("png", 1)
    );
    let mut dup = upload(p, vec![], "png", 1);
    dup.metadata.artist = "Different".into();
    let dupe = images::upload(&pool, &s, dup).await.unwrap();
    assert!(dupe.duplicate);
    assert_eq!(dupe.id, first.id);
    assert_eq!(entries(&d).len(), 1);
    assert_eq!(
        images::get(&pool, first.id).await.unwrap().artist,
        record.artist
    );
    assert_eq!(images::memberships(&pool, first.id).await.unwrap().len(), 2);
    for (format, color) in [("jpg", 2), ("webp", 3)] {
        let archived = images::upload(&pool, &s, upload(p, vec![], format, color))
            .await
            .unwrap();
        assert!(
            images::memberships(&pool, archived.id)
                .await
                .unwrap()
                .is_empty()
        );
    }
    sqlx::raw_sql("CREATE FUNCTION delay_image_insert() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(0.1); RETURN NEW; END $$; CREATE TRIGGER delay_image_insert BEFORE INSERT ON images FOR EACH ROW EXECUTE FUNCTION delay_image_insert();").execute(&pool).await.unwrap();
    let (left, right) = tokio::join!(
        images::upload(&pool, &s, upload(p, vec![a], "png", 44)),
        images::upload(&pool, &s, upload(p, vec![b], "png", 44))
    );
    sqlx::query("DROP TRIGGER delay_image_insert ON images")
        .execute(&pool)
        .await
        .unwrap();
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.id, right.id);
    assert_ne!(left.duplicate, right.duplicate);
    assert_eq!(entries(&d).len(), 4);
    assert_eq!(images::memberships(&pool, left.id).await.unwrap().len(), 1);
    assert!(
        matches!(catalog::delete(&pool,Kind::People,p).await,Err(Error::Conflict(message)) if message.contains("uploader"))
    );
    let p2 = add(&pool, Kind::People, "Corrected uploader", None).await;
    images::update(
        &pool,
        first.id,
        &Metadata {
            uploader_id: p2,
            artist: " ".into(),
            source_url: "".into(),
            character_ids: vec![b],
        },
    )
    .await
    .unwrap();
    let updated = images::get(&pool, first.id).await.unwrap();
    assert_eq!(updated.created_at, record.created_at);
    assert!(updated.updated_at >= record.updated_at);
    assert!(updated.artist.is_none());
    assert_eq!(updated.uploaded_by_person_id, p2);
    catalog::delete(&pool, Kind::Characters, b).await.unwrap();
    assert!(
        images::memberships(&pool, first.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(images::get(&pool, first.id).await.is_ok());
    assert!(d.path().join(&record.storage_key).exists());
    images::delete(&pool, &s, first.id).await.unwrap();
    assert!(!d.path().join(&record.storage_key).exists());
    assert!(catalog::get(&pool, Kind::Characters, a).await.is_ok());
    let stale = images::upload(&pool, &s, upload(p, vec![], "png", 99))
        .await
        .unwrap();
    let row = images::get(&pool, stale.id).await.unwrap();
    std::fs::remove_file(d.path().join(row.storage_key)).unwrap();
    assert!(
        !images::delete(&pool, &s, stale.id)
            .await
            .unwrap()
            .cleanup_issue
    );
    // A directory in place of a generated file triggers a removal error, after DB deletion.
    let damaged = images::upload(&pool, &s, upload(p, vec![], "png", 98))
        .await
        .unwrap();
    let row = images::get(&pool, damaged.id).await.unwrap();
    std::fs::remove_file(d.path().join(&row.storage_key)).unwrap();
    std::fs::create_dir(d.path().join(&row.storage_key)).unwrap();
    assert!(
        images::delete(&pool, &s, damaged.id)
            .await
            .unwrap()
            .cleanup_issue
    );
    assert!(matches!(
        images::get(&pool, damaged.id).await,
        Err(Error::Missing)
    ));
}
#[sqlx::test(migrations = "./migrations")]
async fn validation_constraints_and_ordinary_failure_cleanup(pool: PgPool) {
    let (d, s) = storage().await;
    let (p, a, _) = setup(&pool).await;
    for url in [
        "relative",
        "http:example.test",
        "https:///example.test",
        "https://example.test/space here",
        "https://example.test\\path",
        "ftp://example.test/a",
        "https://user:pass@example.test",
        "https://@example.test",
        "http:///",
        "javascript:alert(1)",
    ] {
        let mut input = upload(p, vec![a], "png", 1);
        input.metadata.source_url = url.into();
        assert!(matches!(
            images::upload(&pool, &s, input).await,
            Err(Error::Validation(_))
        ));
    }
    for change in 0..5 {
        let mut input = upload(p, vec![a], "png", 2);
        match change {
            0 => input.metadata.uploader_id = 999999,
            1 => input.metadata.character_ids = vec![999999],
            2 => input.metadata.artist = "é".repeat(201),
            3 => input.metadata.source_url = format!("https://example.test/{}", "a".repeat(2048)),
            _ => input.bytes = b"<svg/>".to_vec(),
        };
        assert!(matches!(
            images::upload(&pool, &s, input).await,
            Err(Error::Validation(_))
        ));
        assert!(entries(&d).is_empty());
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM images")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    // Fail after file publication and even after the image INSERT: memberships and file must roll back.
    sqlx::raw_sql("CREATE FUNCTION fail_membership() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected persistence failure'; END $$; CREATE TRIGGER fail_membership BEFORE INSERT ON image_characters FOR EACH ROW EXECUTE FUNCTION fail_membership();").execute(&pool).await.unwrap();
    assert!(matches!(
        images::upload(&pool, &s, upload(p, vec![a], "png", 3)).await,
        Err(Error::Unexpected(_))
    ));
    assert!(entries(&d).is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM images")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER fail_membership ON image_characters")
        .execute(&pool)
        .await
        .unwrap();
    // An explicit COMMIT rejection is also safe to clean up (unlike a lost acknowledgement).
    sqlx::raw_sql("CREATE FUNCTION fail_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected commit rejection'; END $$; CREATE CONSTRAINT TRIGGER fail_commit AFTER INSERT ON images DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION fail_commit();").execute(&pool).await.unwrap();
    assert!(matches!(
        images::upload(&pool, &s, upload(p, vec![a], "png", 4)).await,
        Err(Error::Unexpected(_))
    ));
    assert!(entries(&d).is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM images")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER fail_commit ON images")
        .execute(&pool)
        .await
        .unwrap();
    // Storage failure before publication must not insert metadata.
    let vanished = tempfile::tempdir().unwrap();
    let broken = LocalStorage::initialize(vanished.path()).await.unwrap();
    vanished.close().unwrap();
    assert!(matches!(
        images::upload(&pool, &broken, upload(p, vec![a], "png", 4)).await,
        Err(Error::Storage(_))
    ));
    let archived = images::upload(&pool, &s, upload(p, vec![], "png", 5))
        .await
        .unwrap();
    for sql in [
        "UPDATE images SET sha256='\\x01' WHERE id=$1",
        "UPDATE images SET byte_size=0 WHERE id=$1",
        "UPDATE images SET byte_size=20971521 WHERE id=$1",
        "UPDATE images SET width=12001 WHERE id=$1",
        "UPDATE images SET width=12000,height=12000 WHERE id=$1",
        "UPDATE images SET storage_key='../escape.png' WHERE id=$1",
        "UPDATE images SET content_type='image/svg+xml' WHERE id=$1",
        "UPDATE images SET original_filename='' WHERE id=$1",
        "UPDATE images SET original_filename='unsafe/path' WHERE id=$1",
        "UPDATE images SET uploaded_by_person_id=999999 WHERE id=$1",
        "UPDATE images SET artist=repeat('a',201) WHERE id=$1",
        "UPDATE images SET source_url='ftp://example.test' WHERE id=$1",
    ] {
        assert!(
            sqlx::query(sql)
                .bind(archived.id)
                .execute(&pool)
                .await
                .is_err(),
            "{sql}"
        );
    }
    assert!(
        sqlx::query("INSERT INTO image_characters(image_id,character_id) VALUES($1,999999)")
            .bind(archived.id)
            .execute(&pool)
            .await
            .is_err()
    );
    let keys = entries(&d);
    let report = images::cleanup(&pool, &s, false).await.unwrap();
    assert!(report.is_empty());
    assert_eq!(keys, entries(&d));
}
#[sqlx::test(migrations = "./migrations")]
async fn gallery_order_filter_and_pagination(pool: PgPool) {
    let (_d, s) = storage().await;
    let (p, a, b) = setup(&pool).await;
    let mut ids = vec![];
    for i in 0..26 {
        ids.push(
            images::upload(
                &pool,
                &s,
                upload(p, if i % 2 == 0 { vec![a, b] } else { vec![] }, "png", i),
            )
            .await
            .unwrap()
            .id,
        );
    }
    sqlx::query("UPDATE images SET created_at='2026-01-01T00:00:00Z'")
        .execute(&pool)
        .await
        .unwrap();
    let first = images::gallery(&pool, 1, None).await.unwrap();
    assert_eq!(first.images.len(), 24);
    assert!(first.has_next);
    assert_eq!(first.images[0].id, ids[25]);
    let second = images::gallery(&pool, 2, None).await.unwrap();
    assert_eq!(second.images.len(), 2);
    assert!(!second.has_next);
    assert_eq!(second.images[0].id, ids[1]);
    let filtered = images::gallery(&pool, 1, Some(a)).await.unwrap();
    assert_eq!(filtered.images.len(), 13);
    assert!(!filtered.has_next);
    assert_eq!(
        images::gallery(&pool, 1, Some(b))
            .await
            .unwrap()
            .images
            .len(),
        13
    );
    for page in [0, -1, i64::MAX] {
        assert!(matches!(
            images::gallery(&pool, page, None).await,
            Err(Error::Validation(_))
        ));
    }
    assert!(images::gallery(&pool, 1, Some(999999)).await.is_err());
}
fn multipart(fields: &[(&str, String)], files: &[(&str, &str, Vec<u8>)]) -> Vec<u8> {
    let mut b = vec![];
    for (name, value) in fields {
        b.extend(
            format!(
                "--boundary\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    for (filename, mime, bytes) in files {
        b.extend(format!("--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n").as_bytes());
        b.extend(bytes);
        b.extend(b"\r\n");
    }
    b.extend(b"--boundary--\r\n");
    b
}
async fn send(
    r: &common::Client,
    method: &str,
    path: &str,
    content_type: &str,
    body: Vec<u8>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = r
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("origin", "http://127.0.0.1:3000")
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}
#[sqlx::test(migrations = "./migrations")]
async fn multipart_http_workflow(pool: PgPool) {
    let (d, s) = storage().await;
    let (p, a, b) = setup(&pool).await;
    let r = common::client(&pool, s.clone()).await;
    let mime = "multipart/form-data; boundary=boundary";
    let fields = vec![
        ("uploader", p.to_string()),
        ("characters", a.to_string()),
        ("characters", b.to_string()),
        ("artist", "<script>alert(1)</script>".into()),
    ];
    let data = fixture("png", 1);
    let body = multipart(
        &fields,
        &[("../../misleading.svg", "image/svg+xml", data.clone())],
    );
    let (status, headers, _) = send(&r, "POST", "/images", mime, body.clone()).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = headers["location"].to_str().unwrap();
    let id = location.rsplit('/').next().unwrap().parse::<i64>().unwrap();
    let (status, _, html) = send(&r, "GET", location, "", vec![]).await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(html).unwrap();
    assert!(!html.contains("<script>"));
    assert!(html.contains("&#60;script&#62;") || html.contains("&lt;script&gt;"));
    assert!(html.contains("Link") && html.contains("Venti"));
    let (status, headers, bytes) =
        send(&r, "GET", &format!("/images/{id}/content"), "", vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(bytes, data);
    let (status, headers, _) = send(&r, "POST", "/images", mime, body).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers["location"], format!("/images/{id}?duplicate=1"));
    assert_eq!(entries(&d).len(), 1);
    assert!(
        String::from_utf8(
            send(&r, "GET", &format!("/images/{id}?duplicate=1"), "", vec![])
                .await
                .2
        )
        .unwrap()
        .contains("Already archived")
    );
    for c in [a, b] {
        assert_eq!(
            send(&r, "GET", &format!("/images?character={c}"), "", vec![])
                .await
                .0,
            StatusCode::OK
        );
        let html = String::from_utf8(
            send(&r, "GET", &format!("/characters/{c}"), "", vec![])
                .await
                .2,
        )
        .unwrap();
        assert!(html.contains(&format!("/images?character={c}")));
    }
    let edit = format!(
        "uploader={p}&artist=Corrected&source_url=https%3A%2F%2Fexample.test&characters={b}&characters={b}"
    );
    assert_eq!(
        send(
            &r,
            "POST",
            &format!("/images/{id}/edit"),
            "application/x-www-form-urlencoded",
            edit.into_bytes()
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    assert_eq!(images::memberships(&pool, id).await.unwrap().len(), 1);
    let invalid = format!(
        "uploader={p}&artist=Preserved&source_url=javascript%3Aalert%281%29&characters={b}"
    );
    let (status, _, html) = send(
        &r,
        "POST",
        &format!("/images/{id}/edit"),
        "application/x-www-form-urlencoded",
        invalid.into_bytes(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let html = String::from_utf8(html).unwrap();
    assert!(html.contains("value=\"Preserved\""));
    assert!(html.contains(&format!("value=\"{b}\" checked")));
    for files in [
        vec![("bad.svg", "image/png", b"<svg/>".to_vec())],
        vec![
            ("a.png", "image/png", data.clone()),
            ("b.png", "image/png", data.clone()),
        ],
    ] {
        let (status, _, html) = send(&r, "POST", "/images", mime, multipart(&fields, &files)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            String::from_utf8(html)
                .unwrap()
                .contains("Select the image file again")
        );
        assert_eq!(entries(&d).len(), 1);
    }
    let response = r
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/images")
                .header("content-type", mime)
                .header("origin", "http://127.0.0.1:3000")
                .header(
                    "content-length",
                    (20 * 1024 * 1024 + 64 * 1024 + 1).to_string(),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    for path in [
        "/images".into(),
        format!("/images/{id}/edit"),
        format!("/images/{id}/delete"),
    ] {
        let response = r
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("origin", "http://evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    for q in ["page=0", "page=-1", "page=oops", "page=9223372036854775807"] {
        assert_eq!(
            send(&r, "GET", &format!("/images?{q}"), "", vec![]).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        send(&r, "GET", "/images/999999", "", vec![]).await.0,
        StatusCode::NOT_FOUND
    );
    for path in [
        "/images/nope",
        "/images/0",
        "/images/-1",
        "/images/nope/content",
    ] {
        assert_eq!(
            send(&r, "GET", path, "", vec![]).await.0,
            StatusCode::NOT_FOUND
        );
    }
    // Stale M1 selection is now an inline field error, with its parent/detail retained.
    let (status, _, html) = send(
        &r,
        "POST",
        &format!("/characters/{a}/tags"),
        "application/x-www-form-urlencoded",
        b"tag_id=999999".to_vec(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        String::from_utf8(html)
            .unwrap()
            .contains("selection no longer exists")
    );
    let row = images::get(&pool, id).await.unwrap();
    s.remove(&row.storage_key).await.unwrap();
    let (status, _, html) = send(&r, "GET", &format!("/images/{id}/content"), "", vec![]).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        !String::from_utf8(html)
            .unwrap()
            .contains(d.path().to_str().unwrap())
    );
    assert_eq!(
        send(&r, "GET", &format!("/images/{id}/delete"), "", vec![])
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        send(
            &r,
            "POST",
            &format!("/images/{id}/delete"),
            "application/x-www-form-urlencoded",
            vec![]
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    assert!(catalog::get(&pool, Kind::Characters, a).await.is_ok());
    assert!(catalog::get(&pool, Kind::Characters, b).await.is_ok());
}

/// A body with an unknown size hint, emitted in chunks. Repeated bytes keep fixtures small.
struct Chunked {
    segments: std::collections::VecDeque<(Vec<u8>, usize)>,
    offset: usize,
}
impl futures_core::Stream for Chunked {
    type Item = Result<axum::body::Bytes, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        loop {
            let Some((pattern, total)) = self.segments.front() else {
                return std::task::Poll::Ready(None);
            };
            if self.offset == *total {
                self.segments.pop_front();
                self.offset = 0;
                continue;
            }
            let n = (*total - self.offset).min(16 * 1024);
            let bytes = if pattern.len() == 1 {
                vec![pattern[0]; n]
            } else {
                pattern[self.offset..self.offset + n].to_vec()
            };
            self.offset += n;
            return std::task::Poll::Ready(Some(Ok(bytes.into())));
        }
    }
}
fn streamed(segments: Vec<(Vec<u8>, usize)>) -> Body {
    Body::from_stream(Chunked {
        segments: segments.into(),
        offset: 0,
    })
}
fn segment(bytes: &[u8]) -> (Vec<u8>, usize) {
    (bytes.to_vec(), bytes.len())
}
#[sqlx::test(migrations = "./migrations")]
async fn streamed_multipart_limits_without_content_length(pool: PgPool) {
    let (d, s) = storage().await;
    let (p, _, _) = setup(&pool).await;
    let r = common::client(&pool, s).await;
    let header = format!(
        "--boundary\r\nContent-Disposition: form-data; name=\"uploader\"\r\n\r\n{p}\r\n--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\n\r\n"
    );
    let another=b"\r\n--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"b.png\"\r\n\r\n";
    let footer = b"\r\n--boundary--\r\n";
    // Each file is below the individual cap, but the multi-chunk request crosses the total cap.
    let cases = vec![
        vec![
            segment(header.as_bytes()),
            (vec![b'x'], 20 * 1024 * 1024 - 64 * 1024),
            segment(another),
            (vec![b'y'], 256 * 1024),
            segment(footer),
        ],
        // A single file exceeds 20 MiB while its complete request remains within the allowance.
        vec![
            segment(header.as_bytes()),
            (vec![b'x'], 20 * 1024 * 1024 + 1),
            segment(footer),
        ],
    ];
    for segments in cases {
        let req = Request::builder()
            .method("POST")
            .uri("/images")
            .header("origin", "http://127.0.0.1:3000")
            .header("content-type", "multipart/form-data; boundary=boundary")
            .body(streamed(segments))
            .unwrap();
        assert!(!req.headers().contains_key("content-length"));
        let response = r.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM images")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert!(entries(&d).is_empty());
    }
    let data = fixture("png", 77);
    let req = Request::builder()
        .method("POST")
        .uri("/images")
        .header("origin", "http://127.0.0.1:3000")
        .header("content-type", "multipart/form-data; boundary=boundary")
        .body(streamed(vec![
            segment(header.as_bytes()),
            segment(&data),
            segment(footer),
        ]))
        .unwrap();
    assert!(!req.headers().contains_key("content-length"));
    assert_eq!(
        r.oneshot(req).await.unwrap().status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM images")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(entries(&d).len(), 1);
}
