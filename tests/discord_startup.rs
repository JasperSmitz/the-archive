//! Native executable smoke: no real Discord requests or provider resources.
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use sqlx::PgPool;
use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
#[sqlx::test]
async fn native_startup_signed_ping_maintenance_and_restart(pool: PgPool) {
    let cwd = tempfile::tempdir().unwrap();
    let key = SigningKey::from_bytes(&[91; 32]);
    let mut database = url::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    database.set_path(pool.connect_options().get_database().unwrap());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    for maintenance in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let origin = format!("http://{addr}");
        let child = Command::new(env!("CARGO_BIN_EXE_the-archive"))
            .current_dir(cwd.path())
            .env_clear()
            .env("APP_ENV", "development")
            .env("DATABASE_URL", database.as_str())
            .env("LISTEN_ADDR", addr.to_string())
            .env("APP_ORIGIN", &origin)
            .env("IMAGE_STORAGE_DIR", cwd.path().join("images"))
            .env("MAINTENANCE_MODE", maintenance.to_string())
            .env("DISCORD_ENABLED", "true")
            .env("DISCORD_APPLICATION_ID", "111")
            .env("DISCORD_GUILD_ID", "222")
            .env("DISCORD_PUBLIC_KEY", hex(key.verifying_key().as_bytes()))
            .env("DISCORD_ALLOWED_USER_IDS", "333,334")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut server = Server(child);
        let mut ready = false;
        for _ in 0..50 {
            assert!(
                server.0.try_wait().unwrap().is_none(),
                "Native server exited during startup"
            );
            if let Ok(response) = client.get(format!("{origin}/healthz")).send().await {
                assert_eq!(response.status(), 200);
                assert_eq!(response.text().await.unwrap(), "ok\n");
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(ready, "Native health endpoint did not become ready");
        for payload in [
            json!({"type":1,"application_id":"111"}),
            json!({"id":"1000","type":2,"application_id":"111","guild_id":"222","member":{"user":{"id":if maintenance {"333"} else {"999"}}},"token":"test-only-token","data":{"name":"characters","type":1}}),
        ] {
            let bytes = serde_json::to_vec(&payload).unwrap();
            let timestamp = chrono::Utc::now().timestamp().to_string();
            let signature = key.sign(&[timestamp.as_bytes(), &bytes].concat());
            let response = client
                .post(format!("{origin}/discord/interactions"))
                .header("x-signature-timestamp", timestamp)
                .header("x-signature-ed25519", hex(&signature.to_bytes()))
                .body(bytes)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let value: serde_json::Value = response.json().await.unwrap();
            if payload["type"] == 1 {
                assert_eq!(value, json!({"type":1}));
            } else {
                assert_eq!(value["type"], 4);
                assert_eq!(value["data"]["flags"], 64);
                if maintenance {
                    assert!(
                        value["data"]["content"]
                            .as_str()
                            .unwrap()
                            .contains("maintenance")
                    );
                }
            }
        }
        assert_eq!(
            client
                .get(format!("{origin}/images/1/content"))
                .send()
                .await
                .unwrap()
                .status(),
            if maintenance { 503 } else { 401 }
        );
        #[cfg(unix)]
        {
            // Send SIGTERM only to our own isolated child; verify its graceful shutdown exit.
            unsafe {
                libc::kill(server.0.id() as libc::pid_t, libc::SIGTERM);
            }
            let mut exited = false;
            for _ in 0..100 {
                if let Some(status) = server.0.try_wait().unwrap() {
                    assert!(status.success());
                    exited = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(exited, "Graceful shutdown did not finish");
        }
    }
}
