use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use the_archive::{
    app::{
        self, associations,
        catalog::{self, Input},
        characters,
    },
    error::Error,
    models::{Filters, Kind},
    web,
};
use tower::ServiceExt;
async fn add(p: &PgPool, k: Kind, name: &str, franchise_id: Option<i64>) -> i64 {
    catalog::save(
        p,
        k,
        None,
        &Input {
            name: name.into(),
            franchise_id,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}
#[test]
fn validation() {
    assert_eq!(app::text("name", "  Éva  ", 200).unwrap(), "Éva");
    assert!(app::text("name", "  ", 200).is_err());
    assert!(app::text("tag", &"🦊".repeat(80), 80).is_ok());
    assert!(app::text("tag", &"🦊".repeat(81), 80).is_err());
    assert!(app::id(0).is_err());
}
#[sqlx::test(migrations = "./migrations")]
async fn database_rules(pool: PgPool) {
    let types = catalog::list(&pool, Kind::Types).await.unwrap();
    assert_eq!(types.len(), 5);
    for (key, label) in [
        ("kin", "Kin"),
        ("associated", "Associated"),
        ("favorite", "Favorite"),
        ("wife", "Wife"),
        ("husband", "Husband"),
    ] {
        assert!(types.iter().any(|t| t.key == key && t.name == label));
    }
    let emoji = add(&pool, Kind::People, &"🦊".repeat(200), None).await;
    assert_eq!(
        catalog::get(&pool, Kind::People, emoji)
            .await
            .unwrap()
            .name
            .chars()
            .count(),
        200
    );
    assert!(
        sqlx::query("INSERT INTO people(name) VALUES($1)")
            .bind("🦊".repeat(201))
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("INSERT INTO tags(name) VALUES($1)")
            .bind("\u{2003}tag")
            .execute(&pool)
            .await
            .is_err()
    );
    let normalized = add(&pool, Kind::Tags, "\u{2003}unicode tag\u{00a0}", None).await;
    assert_eq!(
        catalog::get(&pool, Kind::Tags, normalized)
            .await
            .unwrap()
            .name,
        "unicode tag"
    );
    assert!(types.iter().any(|t| t.key == "kin" && t.name == "Kin"));
    let f = add(&pool, Kind::Franchises, " Zelda ", None).await;
    let f2 = add(&pool, Kind::Franchises, "Moomins", None).await;
    let p = add(&pool, Kind::People, "Alice", None).await;
    let p2 = add(&pool, Kind::People, "Bob", None).await;
    let tag = add(&pool, Kind::Tags, "my favorites!", None).await;
    for (k, name) in [
        (Kind::Franchises, "zelda"),
        (Kind::People, "aLICE"),
        (Kind::Tags, "MY FAVORITES!"),
    ] {
        assert!(matches!(
            catalog::save(
                &pool,
                k,
                None,
                &Input {
                    name: name.into(),
                    ..Default::default()
                }
            )
            .await,
            Err(Error::Conflict(_))
        ));
    }
    let c = add(&pool, Kind::Characters, "Link", Some(f)).await;
    let c2 = add(&pool, Kind::Characters, "Link", Some(f2)).await;
    assert!(matches!(
        catalog::save(
            &pool,
            Kind::Characters,
            None,
            &Input {
                name: "link".into(),
                franchise_id: Some(f),
                ..Default::default()
            }
        )
        .await,
        Err(Error::Conflict(_))
    ));
    let a = types[0].id;
    let b = types[1].id;
    associations::associate(&pool, c, p, a, false)
        .await
        .unwrap();
    associations::associate(&pool, c, p, b, false)
        .await
        .unwrap();
    associations::associate(&pool, c, p, a, false)
        .await
        .unwrap();
    assert_eq!(associations::associations(&pool, c).await.unwrap().len(), 2);
    associations::tag(&pool, c, tag, false).await.unwrap();
    associations::tag(&pool, c, tag, false).await.unwrap();
    assert_eq!(associations::tags(&pool, c).await.unwrap().len(), 1);
    associations::associate(&pool, c2, p, a, false)
        .await
        .unwrap();
    associations::associate(&pool, c2, p2, b, false)
        .await
        .unwrap();
    let rows = characters::browse(
        &pool,
        &Filters {
            person: Some(p),
            association_type: Some(b),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, c);
    assert_eq!(
        characters::browse(
            &pool,
            &Filters {
                franchise: Some(f),
                person: Some(p),
                association_type: Some(b),
                tag: Some(tag)
            }
        )
        .await
        .unwrap()
        .len(),
        1
    );
    assert_eq!(
        characters::browse(
            &pool,
            &Filters {
                person: Some(p),
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .len(),
        2
    );
    assert_eq!(
        characters::browse(
            &pool,
            &Filters {
                association_type: Some(b),
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .len(),
        2
    );
    for (k, id) in [(Kind::People, p), (Kind::Franchises, f), (Kind::Types, a)] {
        assert!(matches!(
            catalog::delete(&pool, k, id).await,
            Err(Error::Conflict(_))
        ));
    }
    assert!(
        sqlx::query("INSERT INTO character_tags(character_id,tag_id) VALUES($1,$2)")
            .bind(c)
            .bind(999999_i64)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("INSERT INTO tags(name) VALUES($1)")
            .bind("🦊".repeat(81))
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("INSERT INTO people(name) VALUES('   ')")
            .execute(&pool)
            .await
            .is_err()
    );
    catalog::delete(&pool, Kind::Tags, tag).await.unwrap();
    assert!(associations::tags(&pool, c).await.unwrap().is_empty());
    associations::associate(&pool, c, p, b, true).await.unwrap();
    assert_eq!(associations::associations(&pool, c).await.unwrap().len(), 1);
    catalog::save(
        &pool,
        Kind::Characters,
        Some(c),
        &Input {
            name: "Link updated".into(),
            franchise_id: Some(f),
            description: "   ".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        catalog::get(&pool, Kind::Characters, c)
            .await
            .unwrap()
            .description
            .is_none()
    );
    catalog::delete(&pool, Kind::Characters, c).await.unwrap();
    assert!(
        associations::associations(&pool, c)
            .await
            .unwrap()
            .is_empty()
    );
    catalog::delete(&pool, Kind::Franchises, f).await.unwrap();
    assert!(matches!(
        catalog::get(&pool, Kind::Characters, c).await,
        Err(Error::Missing)
    ));
    let t = catalog::save(
        &pool,
        Kind::Types,
        None,
        &Input {
            name: "Custom label".into(),
            key: "custom_2".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    catalog::save(
        &pool,
        Kind::Types,
        Some(t),
        &Input {
            name: "Renamed".into(),
            key: "changed".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        catalog::get(&pool, Kind::Types, t).await.unwrap().key,
        "custom_2"
    );
    catalog::delete(&pool, Kind::Types, t).await.unwrap();
    for key in ["Upper", "a__b", "1abc", "é", ""] {
        assert!(matches!(
            catalog::save(
                &pool,
                Kind::Types,
                None,
                &Input {
                    name: "Label".into(),
                    key: key.into(),
                    ..Default::default()
                }
            )
            .await,
            Err(Error::Validation(_))
        ));
    }
}
async fn request(
    r: &axum::Router,
    method: &str,
    path: &str,
    body: &str,
) -> (StatusCode, String, Option<String>) {
    let response = r
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("origin", "http://127.0.0.1:3000")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap().to_string());
    let body = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    (status, body, location)
}
#[sqlx::test(migrations = "./migrations")]
async fn http_workflow(pool: PgPool) {
    let r = web::router(pool.clone(), "http://127.0.0.1:3000".into());
    let (status, _, loc) = request(&r, "POST", "/franchises", "name=Zelda").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let f = loc
        .unwrap()
        .split('/')
        .next_back()
        .unwrap()
        .parse::<i64>()
        .unwrap();
    assert_eq!(
        request(&r, "POST", "/people", "name=Alice").await.0,
        StatusCode::SEE_OTHER
    );
    let p = catalog::list(&pool, Kind::People).await.unwrap()[0].id;
    let(status,_,loc)=request(&r,"POST","/characters",&format!("name=%3Cscript%3Ealert%281%29%3C%2Fscript%3E&franchise_id={f}&description=%3Cb%3Ehello%3C%2Fb%3E")).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let c = loc
        .unwrap()
        .split('/')
        .next_back()
        .unwrap()
        .parse::<i64>()
        .unwrap();
    let t = catalog::list(&pool, Kind::Types).await.unwrap()[0].id;
    assert_eq!(
        request(
            &r,
            "POST",
            &format!("/characters/{c}/associations"),
            &format!("person_id={p}&association_type_id={t}")
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        request(&r, "POST", "/tags", "name=Favorites").await.0,
        StatusCode::SEE_OTHER
    );
    let tag = catalog::list(&pool, Kind::Tags).await.unwrap()[0].id;
    assert_eq!(
        request(
            &r,
            "POST",
            &format!("/characters/{c}/tags"),
            &format!("tag_id={tag}")
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    let (status, body, _) = request(&r, "GET", &format!("/characters/{c}"), "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("<script>"));
    assert!(body.contains("&#60;script&#62;") || body.contains("&lt;script&gt;"));
    assert!(body.contains("Alice"));
    assert!(body.contains("Favorites"));
    assert_eq!(
        request(
            &r,
            "GET",
            "/characters?franchise=&person=&association_type=&tag=",
            ""
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, body, _) = request(
        &r,
        "POST",
        "/characters",
        &format!(
            "name=Kept&franchise_id={f}&description={}",
            "x".repeat(10001)
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("value=\"Kept\""));
    assert!(body.contains("description:"));
    assert_eq!(
        request(&r, "POST", "/people", "name=alice").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&r, "POST", &format!("/people/{p}/delete"), "")
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&r, "GET", "/characters/999999", "").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&r, "GET", &format!("/characters/{c}/delete"), "")
            .await
            .0,
        StatusCode::OK
    );
    let (status, body, _) =
        request(&r, "POST", "/characters", "name=Retained&franchise_id=bad").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("Retained") && body.contains("Invalid selection: bad"));
    let (status, body, _) = request(
        &r,
        "POST",
        &format!("/characters/{c}/associations"),
        &format!("person_id={p}&association_type_id="),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains(&format!("value=\"{p}\" selected"))
    );
    assert_eq!(
        request(
            &r,
            "POST",
            &format!("/people/{p}/edit"),
            "name=Alice+updated"
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    for header in [
        "null",
        "bad",
        "http://127.0.0.1:3000/path",
        "http://127.0.0.1:3000?bad",
    ] {
        let response = r
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/people")
                    .header("origin", header)
                    .header("referer", "http://127.0.0.1:3000/people")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("name=Blocked"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = r
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/people")
                .header("referer", "http://127.0.0.1:3000/people")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("name=Referer+accepted"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let response = r
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/people")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = r
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/people")
                .header("origin", "http://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        request(
            &r,
            "POST",
            &format!("/characters/{c}/tags"),
            &format!("tag_id={tag}&remove=true")
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        request(&r, "POST", &format!("/characters/{c}/delete"), "")
            .await
            .0,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        request(&r, "POST", &format!("/people/{p}/delete"), "")
            .await
            .0,
        StatusCode::SEE_OTHER
    );
}
