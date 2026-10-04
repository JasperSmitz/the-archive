use image::{ExtendedColorType, ImageEncoder};
use sqlx::PgPool;
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use the_archive::{
    MIGRATOR,
    app::{
        auth,
        catalog::{self, Input},
        images::{self, Metadata, Upload},
    },
    models::Kind,
    storage::LocalStorage,
};
fn pg_env(command: &mut Command, pool: &PgPool) {
    let o = pool.connect_options();
    command
        .env("PGHOST", o.get_host())
        .env("PGPORT", o.get_port().to_string())
        .env("PGUSER", o.get_username())
        .env("PGDATABASE", o.get_database().unwrap());
    // Test credentials stay in the environment, never command arguments or output.
    if let Ok(url) = std::env::var("DATABASE_URL")
        && let Ok(u) = url::Url::parse(&url)
        && let Some(p) = u.password()
    {
        command.env("PGPASSWORD", p);
    }
    command.env_remove("PGSERVICE");
    if let Ok(bin) = std::env::var("PG_BIN") {
        command.env(
            "PATH",
            format!("{}:{}", bin, std::env::var("PATH").unwrap_or_default()),
        );
    }
}
async fn add(pool: &PgPool, kind: Kind, name: &str, franchise: Option<i64>) -> i64 {
    catalog::save(
        pool,
        kind,
        None,
        &Input {
            name: name.into(),
            franchise_id: franchise,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}
#[sqlx::test(migrations = false)]
async fn coordinated_m2_backup_restore_and_bootstrap(pool: PgPool) {
    // Rehearse importing an older M2 archive: only migrations 001–003 in the source.
    let migrations = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir("migrations").unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if !name.starts_with("004_") {
            std::fs::copy(&path, migrations.path().join(name)).unwrap();
        }
    }
    sqlx::migrate::Migrator::new(migrations.path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    let source = tempfile::tempdir().unwrap();
    let s = LocalStorage::initialize(source.path()).await.unwrap();
    let person = add(&pool, Kind::People, "Uploader", None).await;
    let f = add(&pool, Kind::Franchises, "Zelda", None).await;
    let c = add(&pool, Kind::Characters, "Link", Some(f)).await;
    let mut bytes = vec![];
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[1, 2, 3], 1, 1, ExtendedColorType::Rgb8)
        .unwrap();
    let archived = images::upload(
        &pool,
        &s,
        Upload {
            bytes: bytes.clone(),
            original_filename: "shared.png".into(),
            metadata: Metadata {
                uploader_id: person,
                artist: "Artist".into(),
                character_ids: vec![c],
                ..Default::default()
            },
        },
    )
    .await
    .unwrap();
    let image = images::get(&pool, archived.id).await.unwrap();
    let bundle_parent = tempfile::tempdir().unwrap();
    let bundle = bundle_parent.path().join("backup");
    let mut backup = Command::new("sh");
    backup
        .arg("scripts/backup.sh")
        .arg(&bundle)
        .env("ARCHIVE_WRITES_STOPPED", "yes")
        .env("IMAGE_STORAGE_DIR", source.path());
    pg_env(&mut backup, &pool);
    let result = backup
        .output()
        .expect("Install PostgreSQL client tools or set PG_BIN to their directory");
    assert!(
        result.status.success(),
        "Backup failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let name = format!("restore_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&pool)
        .await
        .unwrap();
    let dest = sqlx::postgres::PgPoolOptions::new()
        .connect_with((*pool.connect_options()).clone().database(&name))
        .await
        .unwrap();
    let files = tempfile::tempdir().unwrap();
    let mut restore = Command::new("sh");
    restore
        .arg("scripts/restore.sh")
        .arg(&bundle)
        .env("ARCHIVE_WRITES_STOPPED", "yes")
        .env("IMAGE_STORAGE_DIR", files.path());
    pg_env(&mut restore, &dest);
    let result = restore.output().unwrap();
    assert!(
        result.status.success(),
        "Restore failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        catalog::get(&dest, Kind::Characters, c).await.unwrap().name,
        "Link"
    );
    let restored = images::get(&dest, archived.id).await.unwrap();
    assert_eq!(restored.storage_key, image.storage_key);
    assert_eq!(restored.sha256, image.sha256);
    assert_eq!(restored.artist.as_deref(), Some("Artist"));
    assert_eq!(
        images::memberships(&dest, archived.id).await.unwrap().len(),
        1
    );
    assert_eq!(
        std::fs::read(files.path().join(&image.storage_key)).unwrap(),
        bytes
    );
    // Startup/admin applies only pending M3 migration after restoring M2 history.
    MIGRATOR.run(&dest).await.unwrap();
    auth::create(&dest, "restored-owner", "restore-only long password")
        .await
        .unwrap();
    assert!(
        auth::login(&dest, "restored-owner", "restore-only long password")
            .await
            .unwrap()
            .is_some()
    );
    // Populated database or nonempty storage is never overwritten.
    let result = restore.output().unwrap();
    assert!(!result.status.success());
    dest.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&pool)
        .await
        .unwrap();
}
fn admin(
    pool: &PgPool,
    directory: &Path,
    args: &[&str],
    password: Option<&str>,
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_the-archive"));
    let mut url = url::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    url.set_path(pool.connect_options().get_database().unwrap());
    cmd.args(args)
        .current_dir(directory)
        .env("DATABASE_URL", url.as_str())
        .env_remove("APP_ENV")
        .env_remove("APP_ORIGIN")
        .env(
            "IMAGE_STORAGE_DIR",
            "/unusable/database-admin-must-not-touch-storage",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    if let Some(password) = password {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(password.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}
#[sqlx::test(migrations = "./migrations")]
async fn cli_accounts_without_storage(pool: PgPool) {
    let d = tempfile::tempdir().unwrap();
    assert!(admin(&pool, d.path(), &["migrate"], None).status.success());
    assert!(
        admin(
            &pool,
            d.path(),
            &["account", "create", "Owner", "--password-stdin"],
            Some("test-only CLI long password\n")
        )
        .status
        .success()
    );
    let listed = admin(&pool, d.path(), &["account", "list"], None);
    assert!(listed.status.success());
    let text = String::from_utf8(listed.stdout).unwrap();
    assert!(text.contains("owner\tactive"));
    assert!(!text.contains("password") && !text.contains("argon2"));
    let token = auth::login(&pool, "owner", "test-only CLI long password")
        .await
        .unwrap()
        .unwrap();
    assert!(
        admin(
            &pool,
            d.path(),
            &["account", "reset-password", "owner", "--password-stdin"],
            Some("replacement CLI long password\n")
        )
        .status
        .success()
    );
    assert!(auth::authenticate(&pool, &token).await.unwrap().is_none());
    assert!(
        admin(&pool, d.path(), &["account", "disable", "owner"], None)
            .status
            .success()
    );
    assert!(auth::list(&pool).await.unwrap()[0].disabled);
    assert!(
        !admin(
            &pool,
            d.path(),
            &["account", "create", "other", "--password-stdin"],
            Some("short\n")
        )
        .status
        .success()
    );
}

#[test]
fn server_configuration_fails_before_connecting() {
    let d = tempfile::tempdir().unwrap();
    let cases: Vec<Vec<(&str, &str)>> = vec![
        vec![],
        vec![("APP_ENV", "production")],
        vec![
            ("APP_ENV", "production"),
            ("APP_ORIGIN", "http://archive.example.test"),
        ],
        vec![
            ("APP_ENV", "production"),
            ("APP_ORIGIN", "https://archive.example.test"),
            (
                "DATABASE_URL",
                "postgres://archive@127.0.0.1:55432/postgres?sslmode=disable",
            ),
        ],
        vec![
            ("APP_ENV", "production"),
            ("APP_ORIGIN", "https://archive.example.test"),
        ],
        vec![("APP_ENV", "development"), ("LISTEN_ADDR", "0.0.0.0:$PORT")],
    ];
    for fields in cases {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_the-archive"));
        cmd.current_dir(d.path())
            .env_remove("APP_ENV")
            .env_remove("APP_ORIGIN")
            .env_remove("IMAGE_STORAGE_DIR")
            .env_remove("LISTEN_ADDR")
            .env_remove("MAINTENANCE_MODE")
            .env(
                "DATABASE_URL",
                "postgres://archive@127.0.0.1:55432/postgres?sslmode=verify-full",
            );
        for (key, value) in fields {
            cmd.env(key, value);
        }
        let result = cmd.output().unwrap();
        assert!(!result.status.success());
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(
            !error.contains("Database connection failed"),
            "Configuration unexpectedly reached connection: {error}"
        );
        assert!(!error.contains("postgres://"));
    }
}
