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
pub fn router(pool: PgPool, origin: String, storage: crate::storage::LocalStorage) -> Router {
    Router::new()
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
        .route(
            "/static/app.css",
            get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "text/css")],
                    include_str!("../../static/app.css"),
                )
            }),
        )
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
        .layer(middleware::from_fn_with_state(
            origin.clone(),
            handlers::same_origin,
        ))
        .layer(
            tower_http::trace::TraceLayer::new_for_http()
                .make_span_with(
                    tower_http::trace::DefaultMakeSpan::new().level(tracing::Level::INFO),
                )
                .on_response(
                    tower_http::trace::DefaultOnResponse::new().level(tracing::Level::INFO),
                ),
        )
        .with_state(State {
            pool,
            origin,
            storage,
        })
}
