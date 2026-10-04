use sqlx::postgres::{PgConnectOptions, PgSslMode};
use std::path::PathBuf;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Environment {
    Development,
    Production,
}
pub struct Config {
    pub database_url: String,
    pub listen: String,
    pub origin: String,
    pub image_storage_dir: PathBuf,
    pub environment: Environment,
    pub maintenance: bool,
}
impl Config {
    pub fn load_database() -> Result<String, Box<dyn std::error::Error>> {
        dotenvy::dotenv().ok();
        let database = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required")?;
        let environment = env("APP_ENV")?
            .as_deref()
            .map(|v| parse_environment(Some(v)))
            .transpose()?;
        validate_database(&database, environment)?;
        Ok(database)
    }
    pub fn load_storage() -> Result<PathBuf, Box<dyn std::error::Error>> {
        dotenvy::dotenv().ok();
        let environment = parse_environment(env("APP_ENV")?.as_deref())?;
        let storage = std::env::var_os("IMAGE_STORAGE_DIR");
        if environment == Environment::Production && storage.is_none() {
            return Err("Production requires an explicit IMAGE_STORAGE_DIR".into());
        }
        let root = storage
            .map(PathBuf::from)
            .unwrap_or_else(|| "./var/images".into());
        if root.as_os_str().is_empty() {
            return Err("IMAGE_STORAGE_DIR cannot be empty".into());
        }
        Ok(root)
    }
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        dotenvy::dotenv().ok();
        let environment = parse_environment(env("APP_ENV")?.as_deref())?;
        let listen = env("LISTEN_ADDR")?.unwrap_or_else(|| "127.0.0.1:3000".into());
        listen.parse::<std::net::SocketAddr>().map_err(
            |_| "LISTEN_ADDR must be an IP address and numeric port (no $PORT expansion)",
        )?;
        let origin = env("APP_ORIGIN")?;
        let origin = validate_origin(environment, origin.as_deref(), &listen)?;
        let database_url = Self::load_database()?;
        validate_database(&database_url, Some(environment))?;
        let image_storage_dir = Self::load_storage()?;
        Ok(Self {
            database_url,
            listen,
            origin,
            image_storage_dir,
            environment,
            maintenance: parse_maintenance(env("MAINTENANCE_MODE")?.as_deref())?,
        })
    }
}
pub fn parse_environment(value: Option<&str>) -> Result<Environment, &'static str> {
    match value {
        Some("development") => Ok(Environment::Development),
        Some("production") => Ok(Environment::Production),
        _ => Err("APP_ENV must explicitly be development or production"),
    }
}
pub fn validate_origin(
    environment: Environment,
    origin: Option<&str>,
    listen: &str,
) -> Result<String, &'static str> {
    let fallback = format!("http://{listen}");
    let origin = match (environment, origin) {
        (Environment::Production, None) => return Err("Production requires APP_ORIGIN"),
        (_, Some(v)) => v,
        (_, None) => &fallback,
    };
    let u = url::Url::parse(origin).map_err(|_| "APP_ORIGIN must be an absolute HTTP(S) origin")?;
    if !matches!(u.scheme(), "http" | "https")
        || u.host_str().is_none()
        || u.path() != "/"
        || u.query().is_some()
        || u.fragment().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
        || origin.chars().any(|c| c.is_whitespace() || c.is_control())
        || origin.contains('\\')
        || origin.split_once("://").is_none_or(|(_, a)| {
            a.split('/')
                .next()
                .is_none_or(|a| a.is_empty() || a.contains('@'))
        })
    {
        return Err(
            "APP_ORIGIN must contain only scheme, host and optional port; no path, query, credentials, or fragment",
        );
    }
    if environment == Environment::Production && u.scheme() != "https" {
        return Err("Production APP_ORIGIN must use HTTPS");
    }
    Ok(u.origin().ascii_serialization())
}
pub fn validate_database(
    value: &str,
    environment: Option<Environment>,
) -> Result<(), &'static str> {
    let u = url::Url::parse(value).map_err(|_| "Invalid DATABASE_URL")?;
    if !matches!(u.scheme(), "postgres" | "postgresql")
        || u.host_str().is_none()
        || u.path().len() < 2
    {
        return Err("DATABASE_URL must specify a PostgreSQL host and database");
    }
    let options: PgConnectOptions = value
        .parse()
        .map_err(|_| "Invalid PostgreSQL connection options")?;
    if u.query_pairs().any(|(key, _)| key == "channel_binding") {
        return Err(
            "SQLx 0.8 does not implement channel_binding; use a Rust-compatible Neon URL with sslmode=verify-full and no channel_binding parameter",
        );
    }
    if environment == Some(Environment::Production)
        && !matches!(
            options.get_ssl_mode(),
            PgSslMode::Require | PgSslMode::VerifyCa | PgSslMode::VerifyFull
        )
    {
        return Err(
            "Production DATABASE_URL must explicitly require TLS (sslmode=require or verify-full)",
        );
    }
    Ok(())
}

pub fn parse_maintenance(value: Option<&str>) -> Result<bool, &'static str> {
    match value {
        None | Some("false") => Ok(false),
        Some("true") => Ok(true),
        _ => Err("MAINTENANCE_MODE must be true or false"),
    }
}

fn env(name: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => Err(format!("{name} must contain valid Unicode").into()),
    }
}
