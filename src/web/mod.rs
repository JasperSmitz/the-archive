pub mod auth;
mod forms;
mod handlers;
mod images;
mod templates;
use axum::{Router, middleware, routing::get};
use sqlx::PgPool;
#[derive(Clone)]
pub struct State {
    pub pool: PgPool,
    pub origin: String,
    pub storage: crate::storage::LocalStorage,
}
pub fn router(
    pool: PgPool,
    origin: String,
    storage: crate::storage::LocalStorage,
    environment: crate::config::Environment,
) -> Router {
    router_with_mode(pool, origin, storage, environment, false)
}
pub fn router_with_mode(
    pool: PgPool,
    origin: String,
    storage: crate::storage::LocalStorage,
    environment: crate::config::Environment,
    maintenance: bool,
) -> Router {
    let auth = auth::AuthState {
        pool: pool.clone(),
        environment,
        limiter: auth::LoginLimiter::default(),
    };
    let private = Router::new()
        .route(
            "/images",
            get(images::gallery)
                .post(images::upload)
                .layer::<_, std::convert::Infallible>(axum::extract::DefaultBodyLimit::max(
                    images::REQUEST_LIMIT,
                ))
                .layer(tower_http::limit::RequestBodyLimitLayer::new(
                    images::REQUEST_LIMIT,
                )),
        )
        .route("/images/new", get(images::new))
        .route("/images/{id}", get(images::detail))
        .route("/images/{id}/edit", get(images::edit).post(images::update))
        .route(
            "/images/{id}/delete",
            get(images::confirm).post(images::delete),
        )
        .route("/images/{id}/content", get(images::content))
        .route("/", get(handlers::home))
        .route("/{kind}", get(handlers::list).post(handlers::create))
        .route("/{kind}/new", get(handlers::new))
        .route("/{kind}/{id}", get(handlers::detail))
        .route(
            "/{kind}/{id}/edit",
            get(handlers::edit).post(handlers::update),
        )
        .route(
            "/{kind}/{id}/delete",
            get(handlers::confirm).post(handlers::delete),
        )
        .route("/characters/{id}/tags", axum::routing::post(handlers::tag))
        .route(
            "/characters/{id}/associations",
            axum::routing::post(handlers::associate),
        )
        .route(
            "/logout",
            axum::routing::post(auth::logout).with_state(auth.clone()),
        )
        .route_layer(middleware::from_fn_with_state(auth.clone(), auth::protect))
        .with_state(State {
            pool,
            origin: origin.clone(),
            storage,
        });
    Router::new()
        .merge(private)
        .route(
            "/login",
            get(auth::login_form)
                .post(auth::login)
                .layer(axum::extract::DefaultBodyLimit::max(16 * 1024))
                .with_state(auth),
        )
        .route(
            "/healthz",
            get(|| async { (axum::http::StatusCode::OK, "ok\n") }),
        )
        .route(
            "/static/app.css",
            get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "text/css")],
                    include_str!("../../static/app.css"),
                )
            }),
        )
        .layer(middleware::from_fn_with_state(
            origin,
            handlers::same_origin,
        ))
        .layer(middleware::from_fn_with_state(
            maintenance,
            maintenance_mode,
        ))
        .layer(middleware::from_fn(auth::cache_policy))
        .layer(
            tower_http::trace::TraceLayer::new_for_http()
                .make_span_with(|r: &axum::http::Request<axum::body::Body>| {
                    // Matched route patterns exclude query strings/user values. Never log cookies/bodies.
                    let route = r
                        .extensions()
                        .get::<axum::extract::MatchedPath>()
                        .map(|p| p.as_str())
                        .unwrap_or("unmatched");
                    tracing::info_span!("http",method=%r.method(),route)
                })
                .on_response(
                    tower_http::trace::DefaultOnResponse::new().level(tracing::Level::INFO),
                ),
        )
}

async fn maintenance_mode(
    axum::extract::State(enabled): axum::extract::State<bool>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if enabled && !matches!(request.uri().path(), "/healthz" | "/static/app.css") {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "The archive is paused for maintenance. Please try again later.",
        )
            .into_response();
    }
    next.run(request).await
}
