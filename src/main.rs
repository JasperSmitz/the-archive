use the_archive::{
    MIGRATOR,
    app::auth,
    cli::{self, Command},
    config::Config,
    web,
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "the_archive=info,tower_http=info".into()),
        )
        .init();
    let command = cli::parse(&std::env::args().skip(1).collect::<Vec<_>>())?;
    if command == Command::DiscordPrint {
        println!(
            "{}",
            serde_json::to_string_pretty(&the_archive::discord::commands::manifest())?
        );
        return Ok(());
    }
    if command == Command::DiscordRegister {
        use the_archive::discord::{client::Client, config::snowflake};
        let application = snowflake(
            &std::env::var("DISCORD_APPLICATION_ID")
                .map_err(|_| "DISCORD_APPLICATION_ID is required")?,
        )?;
        let guild = snowflake(
            &std::env::var("DISCORD_GUILD_ID").map_err(|_| "DISCORD_GUILD_ID is required")?,
        )?;
        let token = std::env::var("DISCORD_BOT_TOKEN")
            .map_err(|_| "DISCORD_BOT_TOKEN is required only for explicit registration")?;
        let names = tokio::time::timeout(std::time::Duration::from_secs(60), Client::new()?.register(application, guild, &token)).await.map_err(|_| "Discord registration timed out; some commands may have been upserted. Rerun safely.")??;
        for name in names {
            println!("Upserted guild command /{name}");
        }
        return Ok(());
    }
    // Full production configuration is validated before connecting/serving. Admin commands
    // need only the database; they work even when the volume is unavailable.
    let config = if command == Command::Serve {
        Some(Config::load()?)
    } else {
        None
    };
    let database = Config::load_database()?;
    let discord_config = if command == Command::Serve {
        the_archive::discord::config::Config::load()?
    } else {
        None
    };
    if discord_config.is_some() && config.as_ref().is_some_and(|c| c.origin.len() > 300) {
        return Err("Enabled Discord links require APP_ORIGIN at most 300 bytes".into());
    }
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&database)
        .await
        .map_err(|_| "Database connection failed; check DATABASE_URL, reachability, and TLS")?;
    MIGRATOR
        .run(&pool)
        .await
        .map_err(|e| format!("Database migration failed: {e}"))?;
    let creating_account = matches!(command, Command::AccountCreate(..));
    match command {
        Command::Migrate => println!("Database migrations are up to date."),
        Command::AccountList => {
            for a in auth::list(&pool).await? {
                println!(
                    "{}\t{}",
                    a.username,
                    if a.disabled { "disabled" } else { "active" }
                );
            }
        }
        Command::AccountCreate(name, stdin) | Command::AccountReset(name, stdin) => {
            let create = creating_account;
            // Interactive/stdin reads occur before serving; passwords are never arguments/logs.
            let password = cli::read_password(stdin)?;
            if create {
                auth::create(&pool, &name, &password).await?;
            } else {
                auth::reset(&pool, &name, &password).await?;
            }
            println!(
                "Account {}: {}.",
                auth::username(&name)?,
                if create {
                    "created"
                } else {
                    "password reset; sessions revoked"
                }
            );
        }
        Command::AccountDisable(name) => {
            auth::disable(&pool, &name).await?;
            println!("Account disabled; sessions revoked.");
        }
        Command::SessionsCleanup => println!(
            "Removed {} expired sessions (maximum 100 per invocation).",
            auth::cleanup(&pool).await?
        ),
        Command::Orphans(apply) => {
            let root = Config::load_storage()?;
            let storage = the_archive::storage::LocalStorage::initialize(&root).await?;
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
        }
        Command::Serve => {
            let c = config.ok_or("Server configuration is missing")?;
            let storage = the_archive::storage::LocalStorage::initialize(&c.image_storage_dir)
                .await
                .map_err(
                    |_| "IMAGE_STORAGE_DIR could not be used; check the directory and permissions",
                )?;
            let listener = tokio::net::TcpListener::bind(&c.listen).await?;
            let discord = discord_config
                .map(|config| {
                    Ok::<_, the_archive::discord::client::Failure>(
                        the_archive::discord::Runtime::new(
                            config,
                            pool.clone(),
                            storage.clone(),
                            c.origin.clone(),
                            c.maintenance,
                            the_archive::discord::client::Client::new()?,
                        ),
                    )
                })
                .transpose()?;
            tracing::info!(address=%c.listen, maintenance=c.maintenance,"the-archive ready");
            let shutdown_discord = discord.clone();
            axum::serve(
                listener,
                web::router_with_discord(
                    pool,
                    c.origin,
                    storage,
                    c.environment,
                    c.maintenance,
                    discord.clone(),
                ),
            )
            .with_graceful_shutdown(async move {
                shutdown().await;
                if let Some(runtime) = shutdown_discord {
                    // Begin bounded job drain immediately, independently of slow browser requests.
                    runtime.stop_accepting();
                    tokio::spawn(async move {
                        runtime.shutdown().await;
                    });
                }
            })
            .await?;
            if let Some(discord) = discord {
                discord.shutdown().await;
            }
        }
        Command::DiscordPrint | Command::DiscordRegister => {
            return Err("Discord CLI dispatch failed".into());
        }
    }
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
