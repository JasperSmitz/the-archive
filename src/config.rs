pub struct Config {
    pub database_url: String,
    pub listen: String,
    pub origin: String,
    pub image_storage_dir: std::path::PathBuf,
}
impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        dotenvy::dotenv().ok();
        let listen = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".into());
        let origin = std::env::var("APP_ORIGIN").unwrap_or_else(|_| format!("http://{listen}"));
        let u = url::Url::parse(&origin)?;
        if !matches!(u.scheme(), "http" | "https")
            || u.host_str().is_none()
            || u.path() != "/"
            || u.query().is_some()
            || u.fragment().is_some()
            || !u.username().is_empty()
            || u.password().is_some()
        {
            return Err("APP_ORIGIN must be an HTTP origin without a path or credentials".into());
        }
        Ok(Self {
            database_url: std::env::var("DATABASE_URL")?,
            listen,
            origin: u.origin().ascii_serialization(),
            image_storage_dir: std::env::var_os("IMAGE_STORAGE_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "./var/images".into()),
        })
    }
}
