use ed25519_dalek::VerifyingKey;
use std::collections::HashSet;

#[derive(Clone)]
pub struct Config {
    pub application: u64,
    pub guild: u64,
    pub users: HashSet<u64>,
    pub key: VerifyingKey,
}
pub fn snowflake(s: &str) -> Result<u64, &'static str> {
    if s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Expected a positive decimal Discord ID");
    }
    s.parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or("Expected a positive decimal Discord ID")
}
pub fn token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 1024
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}
impl Config {
    pub fn load() -> Result<Option<Self>, &'static str> {
        if matches!(
            std::env::var("DISCORD_ENABLED"),
            Err(std::env::VarError::NotUnicode(_))
        ) {
            return Err("DISCORD_ENABLED must be true or false");
        }
        Self::from_values(|name| std::env::var(name).ok())
    }
    pub fn from_values(get: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, &'static str> {
        match get("DISCORD_ENABLED").as_deref() {
            None | Some("false") => return Ok(None),
            Some("true") => {}
            _ => return Err("DISCORD_ENABLED must be true or false"),
        }
        let application =
            snowflake(&get("DISCORD_APPLICATION_ID").ok_or("DISCORD_APPLICATION_ID is required")?)?;
        let guild = snowflake(&get("DISCORD_GUILD_ID").ok_or("DISCORD_GUILD_ID is required")?)?;
        let key = super::verification::hex(
            &get("DISCORD_PUBLIC_KEY").ok_or("DISCORD_PUBLIC_KEY is required")?,
        )
        .map_err(|_| "DISCORD_PUBLIC_KEY must be 64 hex characters")?;
        let key = VerifyingKey::from_bytes(&key).map_err(|_| "Invalid DISCORD_PUBLIC_KEY")?;
        if key.is_weak() {
            return Err("Invalid DISCORD_PUBLIC_KEY");
        }
        let list = get("DISCORD_ALLOWED_USER_IDS").ok_or("DISCORD_ALLOWED_USER_IDS is required")?;
        if list.len() > 2048 {
            return Err("Discord allowlist is too large (maximum 100 IDs)");
        }
        let users: HashSet<u64> = list
            .split(',')
            .map(|s| snowflake(s.trim()))
            .collect::<Result<_, _>>()?;
        if users.is_empty() || users.len() > 100 {
            return Err("Supply 1–100 allowed Discord user IDs");
        }
        Ok(Some(Self {
            application,
            guild,
            users,
            key,
        }))
    }
}
