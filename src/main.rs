use the_archive::{MIGRATOR, config::Config, web};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "the_archive=info,tower_http=info".into()),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cleanup = matches!(args.first().map(String::as_str), Some("cleanup-orphans"));
    let apply = args.get(1).is_some_and(|a| a == "--apply");
    if !args.is_empty() && (!cleanup || args.len() > 2 || (args.len() == 2 && !apply)) {
        return Err("Usage: the-archive [cleanup-orphans [--apply]]".into());
    }
    let c = Config::load()?;
    let storage = the_archive::storage::LocalStorage::initialize(&c.image_storage_dir)
        .await
        .map_err(|e| format!("IMAGE_STORAGE_DIR could not be used: {e}"))?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&c.database_url)
        .await?;
    MIGRATOR.run(&pool).await?;
    if cleanup {
        let orphans = the_archive::app::images::cleanup(&pool, &storage, apply).await?;
        println!(
            "{}: {} orphan(s) older than 24 hours. Stop uploads before using --apply.",
            if apply { "Applied cleanup" } else { "Dry run" },
            orphans.len()
        );
        for orphan in orphans {
            println!(
                "{} {}",
                if orphan.removed {
                    "Removed"
                } else {
                    "Would remove"
                },
                orphan.key
            );
        }
        return Ok(());
    }
    let listener = tokio::net::TcpListener::bind(&c.listen).await?;
    tracing::info!(address=%c.listen,"the-archive ready");
    axum::serve(listener, web::router(pool, c.origin, storage))
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
