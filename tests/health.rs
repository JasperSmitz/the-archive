use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use the_archive::{config::Environment, storage::LocalStorage, web};
use tower::ServiceExt;

/// Railway probes use a different Host and no login. Health must bypass maintenance
/// and must not depend on credentials, database queries, or the configured app origin.
#[tokio::test]
async fn railway_health_probe_bypasses_maintenance_in_production() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:9/unused")
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let storage = LocalStorage::initialize(directory.path()).await.unwrap();
    for maintenance in [true, false] {
        let router = web::router_with_mode(
            pool.clone(),
            "https://archive.example.test".into(),
            storage.clone(),
            Environment::Production,
            maintenance,
        );
        for method in ["GET", "HEAD"] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri("/healthz")
                        .header("host", "healthcheck.railway.app")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(
                bytes.as_ref(),
                if method == "GET" {
                    &b"ok\n"[..]
                } else {
                    &b""[..]
                }
            );
        }
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/images")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if maintenance {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::SEE_OTHER
            }
        );
    }
}
