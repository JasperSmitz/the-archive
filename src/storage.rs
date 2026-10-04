//! Concrete local storage. Only generated, flat names can reach the filesystem.
use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use tokio::{fs, io::AsyncWriteExt};
use uuid::Uuid;

pub const ORPHAN_GRACE: Duration = Duration::from_secs(24 * 60 * 60);
#[derive(Clone, Debug)]
pub struct LocalStorage {
    root: PathBuf,
}
#[derive(Debug)]
pub struct Orphan {
    pub key: String,
    pub removed: bool,
}
fn hex_id(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
pub fn generated_key(key: &str) -> bool {
    key.rsplit_once('.')
        .is_some_and(|(id, ext)| hex_id(id) && matches!(ext, "jpg" | "png" | "webp"))
}
fn temporary_key(key: &str) -> bool {
    key.strip_prefix(".upload-")
        .and_then(|s| s.strip_suffix(".tmp"))
        .is_some_and(hex_id)
}
fn invalid_key() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid generated storage key")
}
impl LocalStorage {
    pub async fn initialize(root: impl AsRef<Path>) -> io::Result<Self> {
        if root.as_ref().as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "IMAGE_STORAGE_DIR cannot be empty",
            ));
        }
        fs::create_dir_all(&root).await?;
        let root = fs::canonicalize(root).await?;
        if !fs::metadata(&root).await?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "storage root must be a directory",
            ));
        }
        let storage = Self { root };
        // Validate the same create/sync/publish/remove operations uploads use.
        let probe = storage.write(b"storage probe", "png").await?;
        storage.remove(&probe).await?;
        Ok(storage)
    }
    fn path(&self, key: &str) -> io::Result<PathBuf> {
        if !generated_key(key) {
            return Err(invalid_key());
        }
        Ok(self.root.join(key))
    }
    /// Write/sync a temporary file, publish without overwriting, then remove the temp name.
    pub async fn write(&self, bytes: &[u8], extension: &str) -> io::Result<String> {
        let id = Uuid::new_v4().simple().to_string();
        let key = format!("{id}.{extension}");
        let final_path = self.path(&key)?;
        let temp_path = self.root.join(format!(".upload-{id}.tmp"));
        let result = async {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .await?;
            file.write_all(bytes).await?;
            file.sync_all().await?;
            drop(file);
            fs::hard_link(&temp_path, &final_path).await?;
            Ok::<_, io::Error>(())
        }
        .await;
        let cleanup = fs::remove_file(&temp_path).await;
        if let Err(e) = result {
            if let Err(clean) = cleanup
                && clean.kind() != io::ErrorKind::NotFound
            {
                tracing::error!(error=%clean,"temporary image cleanup failed");
            }
            return Err(e);
        }
        if let Err(e) = cleanup {
            if let Err(clean) = self.remove(&key).await {
                tracing::error!(key=%key,error=%clean,"new image cleanup failed");
            }
            return Err(e);
        }
        Ok(key)
    }
    pub async fn open(&self, key: &str) -> io::Result<fs::File> {
        let path = self.path(key)?;
        if !fs::symlink_metadata(&path).await?.file_type().is_file() {
            return Err(invalid_key());
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        let file = options.open(path).await?;
        if !file.metadata().await?.is_file() {
            return Err(invalid_key());
        }
        Ok(file)
    }
    /// Missing files are already removed. Removing a symlink never follows its target.
    pub async fn remove(&self, key: &str) -> io::Result<()> {
        match fs::remove_file(self.path(key)?).await {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
    /// Flat generated regular files only; never follow symlinks or recurse.
    pub async fn cleanup(
        &self,
        referenced: &HashSet<String>,
        apply: bool,
        grace: Duration,
    ) -> io::Result<Vec<Orphan>> {
        let mut entries = fs::read_dir(&self.root).await?;
        let now = SystemTime::now();
        let mut orphans = vec![];
        while let Some(entry) = entries.next_entry().await? {
            let Some(key) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !(generated_key(&key) || temporary_key(&key))
                || referenced.contains(&key)
                || !entry.file_type().await?.is_file()
            {
                continue;
            }
            let metadata = entry.metadata().await?;
            if now.duration_since(metadata.modified()?).unwrap_or_default() < grace {
                continue;
            }
            if apply {
                fs::remove_file(entry.path()).await?;
            }
            orphans.push(Orphan {
                key,
                removed: apply,
            });
        }
        orphans.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(orphans)
    }
}
