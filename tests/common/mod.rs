use axum::{
    Router,
    body::Body,
    http::{Request, header},
    response::Response,
};
use std::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use the_archive::{app::auth, config::Environment, storage::LocalStorage, web};
use tower::{Service, ServiceExt};
pub const PASSWORD: &str = "test-only long password 2026";
/// Real account + HTTP login. This client attaches the resulting cookie, without bypassing auth.
#[derive(Clone)]
pub struct Client {
    pub router: Router,
    pub cookie: String,
}
impl Service<Request<Body>> for Client {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }
    fn call(&mut self, mut request: Request<Body>) -> Self::Future {
        if !request.headers().contains_key(header::COOKIE) {
            request
                .headers_mut()
                .insert(header::COOKIE, self.cookie.parse().unwrap());
        }
        let router = self.router.clone();
        Box::pin(async move { router.oneshot(request).await })
    }
}
pub async fn client(pool: &sqlx::PgPool, storage: LocalStorage) -> Client {
    auth::create(pool, "test-owner", PASSWORD).await.unwrap();
    let router = web::router(
        pool.clone(),
        "http://127.0.0.1:3000".into(),
        storage,
        Environment::Development,
    );
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("username", "test-owner")
        .append_pair("password", PASSWORD)
        .finish();
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("origin", "http://127.0.0.1:3000")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    Client { router, cookie }
}
