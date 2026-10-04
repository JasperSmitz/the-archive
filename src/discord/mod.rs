//! HTTP interactions are independently signed/allowlisted; never browser-session authenticated.
pub mod client;
pub mod commands;
pub mod config;
pub mod responses;
pub mod verification;
use crate::{error::Error, storage::LocalStorage};
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

const BODY_CAP: usize = 1024 * 1024;
const JOB_BUDGET: Duration = Duration::from_secs(55);
// Six minutes also covers the inclusive boundary/whole-second UTC timestamp precision.
const REPLAY_LIFETIME: Duration = Duration::from_secs(360);
const DENIED: &str = "This interaction is not available. Check access and command configuration.";
pub struct Replay {
    entries: HashMap<u64, Instant>,
    capacity: usize,
}
#[derive(Debug, PartialEq, Eq)]
pub enum ReplayResult {
    New,
    Duplicate,
    Full,
}
impl Replay {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity,
        }
    }
    pub fn admit(&mut self, id: u64, now: Instant) -> ReplayResult {
        self.entries.retain(|_, expiry| *expiry > now);
        if self.entries.contains_key(&id) {
            return ReplayResult::Duplicate;
        }
        if self.entries.len() >= self.capacity {
            return ReplayResult::Full;
        }
        let Some(expiry) = now.checked_add(REPLAY_LIFETIME) else {
            return ReplayResult::Full;
        };
        self.entries.insert(id, expiry);
        ReplayResult::New
    }
}
pub struct Runtime {
    config: config::Config,
    pool: PgPool,
    storage: LocalStorage,
    origin: String,
    maintenance: bool,
    client: client::Client,
    admission: Arc<Semaphore>,
    replay: Mutex<Replay>,
    tasks: TaskTracker,
    cancellation: CancellationToken,
}
impl Runtime {
    pub fn new(
        config: config::Config,
        pool: PgPool,
        storage: LocalStorage,
        origin: String,
        maintenance: bool,
        client: client::Client,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            pool,
            storage,
            origin,
            maintenance,
            client,
            admission: Arc::new(Semaphore::new(4)),
            replay: Mutex::new(Replay::new(4096)),
            tasks: TaskTracker::new(),
            cancellation: CancellationToken::new(),
        })
    }
    /// Close admission, drain for ten seconds, then cancel outstanding jobs and their child tasks.
    pub async fn shutdown(&self) {
        self.stop_accepting();
        if tokio::time::timeout(Duration::from_secs(10), self.tasks.wait())
            .await
            .is_err()
        {
            self.cancellation.cancel();
            self.tasks.wait().await;
        }
        self.cancellation.cancel();
    }
    pub fn stop_accepting(&self) {
        self.admission.close();
        self.tasks.close();
    }
    pub fn active_jobs(&self) -> usize {
        self.tasks.len()
    }
    fn dispatch(
        self: &Arc<Self>,
        command: commands::Command,
        id: u64,
        token: String,
        cap: u64,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) {
        if self.admission.is_closed() {
            return;
        }
        let runtime = self.clone();
        self.tasks.spawn(async move {
            let _permit = permit;
            runtime.supervise(command, id, token, cap).await;
        });
    }
    async fn supervise(
        self: Arc<Self>,
        command: commands::Command,
        id: u64,
        token: String,
        cap: u64,
    ) {
        if self.cancellation.is_cancelled() {
            return;
        }
        let name = command.name;
        let error_token = token.clone();
        let worker = self.clone();
        let mut task = tokio::spawn(async move { worker.deliver(command, id, token, cap).await });
        let outcome = tokio::select! {
            result = tokio::time::timeout(JOB_BUDGET, &mut task) => Some(result),
            _ = self.cancellation.cancelled() => None,
        };
        let delivered = matches!(&outcome, Some(Ok(Ok(Ok(())))));
        let cancelled = outcome.is_none();
        match outcome {
            Some(Ok(Ok(Ok(())))) => {
                tracing::info!(interaction_id = id, command = name, "retrieval delivered")
            }
            Some(Ok(Ok(Err(e)))) => {
                tracing::warn!(interaction_id=id, command=name, outcome=?e, "delivery failed")
            }
            Some(Ok(Err(_))) => tracing::error!(
                interaction_id = id,
                command = name,
                "retrieval task panicked or was cancelled"
            ),
            _ => {
                task.abort();
                let _ = task.await;
                tracing::warn!(
                    interaction_id = id,
                    command = name,
                    "retrieval budget exhausted or shutdown cancelled job"
                );
            }
        }
        if !delivered && !cancelled {
            // No file on the error edit: never retry an ambiguous attachment delivery.
            let payload = responses::message(
                "Delivery could not be completed. Please rerun this read-only command.",
            );
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                self.client
                    .edit(self.config.application, &error_token, &payload, None),
            )
            .await;
        }
    }
    async fn deliver(
        &self,
        command: commands::Command,
        id: u64,
        token: String,
        cap: u64,
    ) -> Result<(), client::Failure> {
        // Leave time after domain/file preparation for delivery and a error edit.
        let result = tokio::time::timeout(
            Duration::from_secs(35),
            responses::execute(&self.pool, &self.storage, &self.origin, &command, cap),
        )
        .await;
        let reply = match result {
            Ok(Ok(reply)) => reply,
            Ok(Err(Error::Validation(fields))) => responses::Reply {
                payload: responses::message(
                    &fields
                        .into_iter()
                        .map(|(_, value)| value)
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                attachment: None,
            },
            Ok(Err(Error::Missing)) => responses::Reply {
                payload: responses::message(
                    "A selected record no longer exists. Rerun with an existing exact selector.",
                ),
                attachment: None,
            },
            _ => {
                tracing::warn!(
                    interaction_id = id,
                    command = command.name,
                    "retrieval failed or timed out"
                );
                responses::Reply {
                    payload: responses::message(
                        "The Archive could not complete this retrieval. Please rerun the command later.",
                    ),
                    attachment: None,
                }
            }
        };
        self.client
            .edit(
                self.config.application,
                &token,
                &reply.payload,
                reply.attachment.as_ref(),
            )
            .await
    }
}
pub fn routes(runtime: Option<Arc<Runtime>>) -> Router {
    if runtime.is_none() {
        return Router::new().route(
            "/discord/interactions",
            post(|| async { StatusCode::NOT_FOUND }),
        );
    }
    Router::new()
        .route(
            "/discord/interactions",
            post(interactions)
                .layer::<_, std::convert::Infallible>(axum::extract::DefaultBodyLimit::max(
                    BODY_CAP,
                ))
                .layer(tower_http::limit::RequestBodyLimitLayer::new(BODY_CAP)),
        )
        .with_state(runtime)
}
fn snowflake(value: Option<&Value>) -> Option<u64> {
    value
        .and_then(Value::as_str)
        .and_then(|s| config::snowflake(s).ok())
}
async fn interactions(
    State(runtime): State<Option<Arc<Runtime>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(runtime) = runtime else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if verification::verify(
        &runtime.config.key,
        &headers,
        &body,
        chrono::Utc::now().timestamp(),
    )
    .is_err()
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return Json(responses::immediate(DENIED)).into_response();
    };
    if snowflake(payload.get("application_id")) != Some(runtime.config.application) {
        return Json(responses::immediate(DENIED)).into_response();
    }
    if payload.get("type").and_then(Value::as_u64) == Some(1) {
        return Json(json!({"type":1})).into_response();
    }
    if payload.get("type").and_then(Value::as_u64) != Some(2)
        || snowflake(payload.get("guild_id")) != Some(runtime.config.guild)
        || !snowflake(payload.pointer("/member/user/id"))
            .is_some_and(|id| runtime.config.users.contains(&id))
    {
        return Json(responses::immediate(DENIED)).into_response();
    }
    if runtime.maintenance {
        return Json(responses::immediate(
            "The Archive is paused for maintenance. Please rerun the command later.",
        ))
        .into_response();
    }
    let command = match commands::parse(&payload["data"]) {
        Ok(c) => c,
        Err(e) => return Json(responses::immediate(e)).into_response(),
    };
    let Some(id) = snowflake(payload.get("id")) else {
        return Json(responses::immediate(DENIED)).into_response();
    };
    let Some(token) = payload
        .get("token")
        .and_then(Value::as_str)
        .filter(|s| config::token(s))
    else {
        return Json(responses::immediate(DENIED)).into_response();
    };
    let cap = match payload.get("attachment_size_limit") {
        None => responses::FILE_CAP,
        Some(v) => v
            .as_u64()
            .filter(|v| *v > 0)
            .unwrap_or(0)
            .min(responses::FILE_CAP),
    };
    let permit = match runtime.admission.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            return Json(responses::immediate(
                "The Librarian is busy. Please rerun the command shortly.",
            ))
            .into_response();
        }
    };
    let replay = runtime
        .replay
        .lock()
        .map(|mut cache| cache.admit(id, Instant::now()))
        .unwrap_or(ReplayResult::Full);
    if replay != ReplayResult::New {
        return Json(responses::immediate(if replay==ReplayResult::Duplicate {"This interaction was already accepted. Rerun the command if its delivery was interrupted."} else {"The Librarian is busy. Please try again later."})).into_response();
    }
    // Begin only after the response body has yielded the deferred acknowledgement.
    // This avoids spawning domain work while the handler is still constructing its reply.
    let body = axum::body::Body::from_stream(Acknowledgement {
        bytes: Some(Bytes::from_static(
            br#"{"type":5,"data":{"allowed_mentions":{"parse":[]}}}"#,
        )),
        start: Some(Box::new({
            let token = token.to_owned();
            move || runtime.dispatch(command, id, token, cap, permit)
        })),
    });
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}
struct Acknowledgement {
    bytes: Option<Bytes>,
    start: Option<Box<dyn FnOnce() + Send>>,
}
impl futures_core::Stream for Acknowledgement {
    type Item = Result<Bytes, std::convert::Infallible>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if let Some(bytes) = self.bytes.take() {
            return std::task::Poll::Ready(Some(Ok(bytes)));
        }
        if let Some(start) = self.start.take() {
            start();
        }
        std::task::Poll::Ready(None)
    }
}
