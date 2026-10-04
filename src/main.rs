use the_archive::{MIGRATOR, config::Config, web};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "the_archive=info,tower_http=info".into()),
        )
        .init();
    let c = Config::load()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&c.database_url)
        .await?;
    MIGRATOR.run(&pool).await?;
    let listener = tokio::net::TcpListener::bind(&c.listen).await?;
    tracing::info!(address=%c.listen,"the-archive ready");
    axum::serve(listener, web::router(pool, c.origin))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}
async fn shutdown() {
    #[cfg(unix)]
    {
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {_ = tokio::signal::ctrl_c()=>{},_ = term.recv()=>{}}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
