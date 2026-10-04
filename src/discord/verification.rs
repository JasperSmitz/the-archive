//! Verify signed bytes before interpreting any JSON.
use axum::http::HeaderMap;
use ed25519_dalek::{Signature, VerifyingKey};

pub fn hex<const N: usize>(s: &str) -> Result<[u8; N], &'static str> {
    if s.len() != N * 2 || !s.is_ascii() {
        return Err("Invalid hex length");
    }
    let mut bytes = [0; N];
    for (i, out) in bytes.iter_mut().enumerate() {
        *out = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| "Invalid hex")?;
    }
    Ok(bytes)
}
pub fn verify(
    key: &VerifyingKey,
    headers: &HeaderMap,
    body: &[u8],
    now: i64,
) -> Result<(), &'static str> {
    if headers.get_all("x-signature-timestamp").iter().count() != 1
        || headers.get_all("x-signature-ed25519").iter().count() != 1
    {
        return Err("Invalid signature headers");
    }
    let timestamp = headers
        .get("x-signature-timestamp")
        .and_then(|v| v.to_str().ok())
        .ok_or("Missing timestamp")?;
    if timestamp.is_empty()
        || timestamp.len() > 20
        || !timestamp.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("Invalid timestamp");
    }
    let seconds: i64 = timestamp.parse().map_err(|_| "Invalid timestamp")?;
    let age = now.checked_sub(seconds).ok_or("Invalid timestamp")?;
    if !(-30..=300).contains(&age) {
        return Err("Expired timestamp");
    }
    let signature = headers
        .get("x-signature-ed25519")
        .and_then(|v| v.to_str().ok())
        .ok_or("Missing signature")?;
    let signature = Signature::from_bytes(&hex(signature)?);
    let mut signed = Vec::with_capacity(timestamp.len() + body.len());
    signed.extend_from_slice(timestamp.as_bytes());
    signed.extend_from_slice(body);
    key.verify_strict(&signed, &signature)
        .map_err(|_| "Invalid signature")
}
