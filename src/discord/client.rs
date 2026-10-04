//! Fixed Discord destinations; errors deliberately contain no request URLs or response bodies.
use super::{commands, config};
use reqwest::{
    Client as Http,
    multipart::{Form, Part},
};
use serde_json::Value;
use std::{net::SocketAddr, time::Duration};
pub struct Attachment {
    pub bytes: Vec<u8>,
    pub filename: String,
    pub content_type: String,
}
#[derive(Clone)]
pub struct Client {
    http: Http,
    base: String,
}
#[derive(Debug)]
pub enum Failure {
    Network,
    Timeout,
    Status(u16),
    InvalidResponse,
    Configuration,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Discord request failed ({self:?}); no secret details are included"
        )
    }
}
impl std::error::Error for Failure {}
impl Client {
    pub fn new() -> Result<Self, Failure> {
        Self::build("https://discord.com/api/v10".into(), true)
    }
    /// Explicit loopback-only dependency injection for local fake API tests. Never env-configured.
    #[doc(hidden)]
    pub fn local_test_server(address: SocketAddr) -> Result<Self, Failure> {
        if !address.ip().is_loopback() {
            return Err(Failure::Configuration);
        }
        Self::build(format!("http://{address}/api/v10"), false)
    }
    fn build(base: String, https: bool) -> Result<Self, Failure> {
        let http = Http::builder()
            .https_only(https)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| Failure::Configuration)?;
        Ok(Self { http, base })
    }
    pub async fn edit(
        &self,
        application: u64,
        token: &str,
        payload: &Value,
        attachment: Option<&Attachment>,
    ) -> Result<(), Failure> {
        if application == 0 || !config::token(token) {
            return Err(Failure::Configuration);
        }
        let url = format!(
            "{}/webhooks/{application}/{token}/messages/@original",
            self.base
        );
        self.deliver(&url, payload, attachment, false).await
    }
    /// A component failure notifies only its invoker; it never edits the shared viewer.
    pub async fn followup(&self, application: u64, token: &str, text: &str) -> Result<(), Failure> {
        if application == 0 || !config::token(token) {
            return Err(Failure::Configuration);
        }
        let mut payload = super::responses::message(text);
        payload["flags"] = serde_json::json!(64);
        let url = format!("{}/webhooks/{application}/{token}", self.base);
        self.deliver(&url, &payload, None, true).await
    }
    async fn deliver(
        &self,
        url: &str,
        payload: &Value,
        attachment: Option<&Attachment>,
        followup: bool,
    ) -> Result<(), Failure> {
        for attempt in 0..2 {
            let request = if followup {
                self.http.post(url)
            } else {
                self.http.patch(url)
            };
            let request = if let Some(a) = attachment {
                let file = Part::bytes(a.bytes.clone())
                    .file_name(a.filename.clone())
                    .mime_str(&a.content_type)
                    .map_err(|_| Failure::Configuration)?;
                request.multipart(
                    Form::new()
                        .text("payload_json", payload.to_string())
                        .part("files[0]", file),
                )
            } else {
                request.json(payload)
            };
            let response = request.send().await.map_err(network)?;
            if response.status().is_success() {
                return Ok(());
            }
            let status = response.status().as_u16();
            // Only an explicit 429 proves non-delivery. Network/5xx failures are never retried.
            if status == 429
                && attempt == 0
                && let Some(delay) = retry_delay(response).await?
            {
                tokio::time::sleep(delay).await;
                continue;
            }
            return Err(Failure::Status(status));
        }
        Err(Failure::Status(429))
    }
    pub async fn register(
        &self,
        application: u64,
        guild: u64,
        bot: &str,
    ) -> Result<Vec<&'static str>, Failure> {
        if application == 0 || guild == 0 || !config::token(bot) {
            return Err(Failure::Configuration);
        }
        let url = format!(
            "{}/applications/{application}/guilds/{guild}/commands",
            self.base
        );
        let mut done = vec![];
        for spec in commands::manifest() {
            let mut succeeded = false;
            for attempt in 0..2 {
                let response = self
                    .http
                    .post(&url)
                    .header("authorization", format!("Bot {bot}"))
                    .json(&spec)
                    .send()
                    .await
                    .map_err(network)?;
                let status = response.status().as_u16();
                if response.status().is_success() {
                    succeeded = true;
                    break;
                }
                if status == 429
                    && attempt == 0
                    && let Some(delay) = retry_delay(response).await?
                {
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(Failure::Status(status));
            }
            if !succeeded {
                return Err(Failure::Status(429));
            }
            tracing::info!(command = spec.name, "guild command upserted");
            done.push(spec.name);
        }
        Ok(done)
    }
}
fn network(e: reqwest::Error) -> Failure {
    if e.is_timeout() {
        Failure::Timeout
    } else {
        Failure::Network
    }
}
async fn retry_delay(mut response: reqwest::Response) -> Result<Option<Duration>, Failure> {
    let mut bytes = vec![];
    while let Some(chunk) = response.chunk().await.map_err(network)? {
        if bytes.len() + chunk.len() > 16 * 1024 {
            return Err(Failure::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    let seconds = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|v| v.get("retry_after").and_then(Value::as_f64));
    Ok(seconds
        .filter(|v| v.is_finite() && (0.0..=5.0).contains(v))
        .map(Duration::from_secs_f64))
}
