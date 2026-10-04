//! Accounts are access principals, independent of catalog people. Tokens/passwords never log.
use crate::error::Error;
use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use tokio::sync::Semaphore;
pub const SESSION_SECONDS: u64 = 7 * 24 * 60 * 60;
static PASSWORD_SLOTS: Semaphore = Semaphore::const_new(2);
#[derive(Clone, Debug, FromRow)]
pub struct Account {
    pub id: i64,
    pub username: String,
    pub disabled: bool,
}
#[derive(FromRow)]
struct Credentials {
    id: i64,
    password_hash: String,
    disabled: bool,
}
fn validation(field: &str, message: &str) -> Error {
    Error::Validation(vec![(field.into(), message.into())])
}
fn crypto_error() -> Error {
    Error::AuthInternal
}
pub fn username(value: &str) -> Result<String, Error> {
    let v = value.trim().to_ascii_lowercase();
    if !(3..=64).contains(&v.len())
        || !v.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'-')
        })
        || !v.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err(validation(
            "username",
            "Use 3–64 ASCII letters, digits, dots, underscores, or hyphens; start with a letter or digit.",
        ));
    }
    Ok(v)
}
pub fn password(value: &str) -> Result<(), Error> {
    if !(12..=1024).contains(&value.len()) || value.contains('\0') {
        return Err(validation(
            "password",
            "Use 12–1,024 UTF-8 bytes, without NUL. Spaces and long password-manager passwords are welcome.",
        ));
    }
    Ok(())
}
fn argon() -> Result<Argon2<'static>, Error> {
    let params = Params::new(19456, 2, 1, Some(32)).map_err(|_| crypto_error())?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}
pub async fn hash_password(value: &str) -> Result<String, Error> {
    password(value)?;
    let value = value.to_owned();
    let permit = PASSWORD_SLOTS.acquire().await.map_err(|_| crypto_error())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut salt = [0_u8; 16];
        OsRng
            .try_fill_bytes(&mut salt)
            .map_err(|_| crypto_error())?;
        let salt = SaltString::encode_b64(&salt).map_err(|_| crypto_error())?;
        Ok(argon()?
            .hash_password(value.as_bytes(), &salt)
            .map_err(|_| crypto_error())?
            .to_string())
    })
    .await
    .map_err(Error::Task)?
}
pub async fn create(pool: &PgPool, name: &str, pass: &str) -> Result<Account, Error> {
    let name = username(name)?;
    let hash = hash_password(pass).await?;
    Ok(sqlx::query_as(
        "INSERT INTO accounts(username,password_hash) VALUES($1,$2) RETURNING id,username,disabled",
    )
    .bind(name)
    .bind(hash)
    .fetch_one(pool)
    .await?)
}
pub async fn list(pool: &PgPool) -> Result<Vec<Account>, Error> {
    Ok(
        sqlx::query_as("SELECT id,username,disabled FROM accounts ORDER BY username,id")
            .fetch_all(pool)
            .await?,
    )
}
pub async fn reset(pool: &PgPool, name: &str, pass: &str) -> Result<(), Error> {
    let name = username(name)?;
    let hash = hash_password(pass).await?;
    let mut tx = pool.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "UPDATE accounts SET password_hash=$1,updated_at=now() WHERE username=$2 RETURNING id",
    )
    .bind(hash)
    .bind(name)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Error::Missing)?;
    sqlx::query("DELETE FROM sessions WHERE account_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn disable(pool: &PgPool, name: &str) -> Result<(), Error> {
    let name = username(name)?;
    let mut tx = pool.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "UPDATE accounts SET disabled=true,updated_at=now() WHERE username=$1 RETURNING id",
    )
    .bind(name)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Error::Missing)?;
    sqlx::query("DELETE FROM sessions WHERE account_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
pub fn digest(token: &str) -> Option<Vec<u8>> {
    if token.len() != 64
        || !token
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return None;
    }
    Some(Sha256::digest(token.as_bytes()).to_vec())
}
/// A bounded cleanup batch on login, and an explicit CLI. Expiry is always checked on reads.
pub async fn cleanup(pool: &PgPool) -> Result<u64, Error> {
    Ok(sqlx::query("DELETE FROM sessions WHERE token_digest IN (SELECT token_digest FROM sessions WHERE expires_at<=now() ORDER BY expires_at LIMIT 100)").execute(pool).await?.rows_affected())
}
/// None is always a generic credentials failure. Unknown/disabled accounts do equal-cost work.
pub async fn login(pool: &PgPool, name: &str, pass: &str) -> Result<Option<String>, Error> {
    let normalized = username(name).ok();
    let candidate: Option<Credentials> =
        sqlx::query_as("SELECT id,password_hash,disabled FROM accounts WHERE username=$1")
            .bind(normalized.as_deref().unwrap_or(""))
            .fetch_optional(pool)
            .await?;
    let value = if password(pass).is_ok() {
        pass.to_owned()
    } else {
        "invalid login input".into()
    };
    let valid_input = password(pass).is_ok() && normalized.is_some();
    let hash = candidate.as_ref().map(|c| c.password_hash.clone());
    let permit = PASSWORD_SLOTS.acquire().await.map_err(|_| crypto_error())?;
    let matches = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let a = argon()?;
        if let Some(hash) = hash {
            let parsed = PasswordHash::new(&hash).map_err(|_| crypto_error())?;
            Ok::<_, Error>(a.verify_password(value.as_bytes(), &parsed).is_ok())
        } else {
            let salt = SaltString::encode_b64(&[0_u8; 16]).map_err(|_| crypto_error())?;
            let _dummy = a
                .hash_password(value.as_bytes(), &salt)
                .map_err(|_| crypto_error())?;
            Ok(false)
        }
    })
    .await
    .map_err(Error::Task)??;
    let Some(candidate) = candidate.filter(|c| !c.disabled && matches && valid_input) else {
        return Ok(None);
    };
    let mut random = [0_u8; 32];
    OsRng
        .try_fill_bytes(&mut random)
        .map_err(|_| crypto_error())?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let token_digest = digest(&token).ok_or_else(crypto_error)?;
    // Lock/recheck credentials after the expensive operation. Reset/disable cannot race
    // a verified old password into creating a post-revocation session.
    let mut tx = pool.begin().await?;
    let current: Option<Credentials> =
        sqlx::query_as("SELECT id,password_hash,disabled FROM accounts WHERE id=$1 FOR UPDATE")
            .bind(candidate.id)
            .fetch_optional(&mut *tx)
            .await?;
    if !current.is_some_and(|c| !c.disabled && c.password_hash == candidate.password_hash) {
        return Ok(None);
    }
    sqlx::query("INSERT INTO sessions(token_digest,account_id) VALUES($1,$2)")
        .bind(token_digest)
        .bind(candidate.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if let Err(e) = cleanup(pool).await {
        tracing::warn!(error=?e,"expired session batch cleanup failed");
    }
    Ok(Some(token))
}
pub async fn authenticate(pool: &PgPool, token: &str) -> Result<Option<Account>, Error> {
    let Some(hash) = digest(token) else {
        return Ok(None);
    };
    Ok(sqlx::query_as("SELECT a.id,a.username,a.disabled FROM accounts a JOIN sessions s ON s.account_id=a.id WHERE s.token_digest=$1 AND s.expires_at>now() AND NOT a.disabled").bind(hash).fetch_optional(pool).await?)
}
pub async fn revoke(pool: &PgPool, token: &str) -> Result<(), Error> {
    if let Some(hash) = digest(token) {
        sqlx::query("DELETE FROM sessions WHERE token_digest=$1")
            .bind(hash)
            .execute(pool)
            .await?;
    }
    Ok(())
}
