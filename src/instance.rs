use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

use anyhow::{Context, Result, bail};

/// Advisory locks remain owned until cleanup has closed the device session.
pub struct InstanceGuard {
    _native: File,
    _legacy: Option<File>,
}

impl InstanceGuard {
    pub fn acquire() -> Result<Self> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
        Self::at(Path::new(&runtime))
    }

    fn at(runtime: &Path) -> Result<Self> {
        let directory = runtime.join("iphone-mirror-rs");
        fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(&directory)?;
        if fs::symlink_metadata(&directory)?.file_type().is_symlink() {
            bail!("refusing a symlinked runtime directory");
        }
        let native = lock(&directory.join("instance.lock"))?;
        // Cooperate with the reference app when it is installed. Running two
        // controllers against one display service makes connection errors opaque.
        let legacy_directory = runtime.join("iphone-mirror");
        let legacy = if legacy_directory.is_dir() {
            Some(lock(&legacy_directory.join("instance.lock"))?)
        } else {
            None
        };
        Ok(Self {
            _native: native,
            _legacy: legacy,
        })
    }
}

fn lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.try_lock()
        .context("another iPhone mirror is running, or its lock is unavailable")?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_excludes_another_process_handle_and_releases_on_drop() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("mirror-lock-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("iphone-mirror"))?;
        let guard = InstanceGuard::at(&dir)?;
        assert!(InstanceGuard::at(&dir).is_err());
        assert!(lock(&dir.join("iphone-mirror/instance.lock")).is_err());
        drop(guard);
        let next = InstanceGuard::at(&dir)?;
        drop(next);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
