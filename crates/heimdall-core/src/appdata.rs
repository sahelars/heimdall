//! Heimdall's per-user application-data directory (SPEC §14).
//!
//! Everything Heimdall keeps about a vault lives here rather than in it: the
//! cross-process write lock, the note-lock rules, and the registry of vaults
//! the shell can find from a working directory. A vault holds the user's
//! Markdown and nothing of ours.
//!
//! ```text
//! <data>/
//! ├── vaults.json            # registry of known vault roots
//! ├── locks/<key>.lock       # one write lock per vault
//! └── vaults/<key>.json      # one set of note-lock rules per vault
//! ```
//!
//! `<key>` is the BLAKE3 hash of the vault's canonical path.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use camino::{Utf8Path, Utf8PathBuf};

use crate::errors::{Error, Result};

/// Distinguishes concurrent temp files written by one process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Where Heimdall's application data lives when the caller has not chosen.
///
/// Application data rather than cache: the system may purge a cache, and a
/// purge that unlinks a lock file some process is holding would leave the next
/// writer locking a different inode — and a purge of the lock rules would
/// silently unlock every note. `HEIMDALL_DATA_DIR` overrides it for a
/// sandboxed deployment with no writable home, and for tests.
pub fn data_dir() -> Utf8PathBuf {
    if let Some(raw) = std::env::var_os("HEIMDALL_DATA_DIR") {
        if let Ok(path) = Utf8PathBuf::from_path_buf(raw.into()) {
            return path;
        }
    }
    // A test run must never write into the developer's real application data.
    // Integration tests pass a directory explicitly; this covers the unit tests
    // in this crate, which open vaults from a dozen different modules.
    if cfg!(test) {
        return temp_base().join("heimdall-test-data");
    }
    let base = dirs::data_local_dir()
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
        .unwrap_or_else(temp_base);
    base.join("heimdall")
}

fn temp_base() -> Utf8PathBuf {
    Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap_or_else(|_| Utf8PathBuf::from("/tmp"))
}

/// Read one of Heimdall's own JSON files, or `None` if it has not been written.
pub(crate) fn read_json<T: serde::de::DeserializeOwned>(path: &Utf8Path) -> Result<Option<T>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Error::from_io("read Heimdall state", &err)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| Error::io_error("Heimdall's saved state is unreadable; it may have been edited by hand"))
}

/// Write one of Heimdall's own JSON files through a temporary sibling, so a
/// reader sees the previous state or the new one and never half of either.
pub(crate) fn write_json<T: serde::Serialize>(path: &Utf8Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::internal("state file has no parent directory"))?;
    std::fs::create_dir_all(parent).map_err(|err| Error::from_io("create Heimdall state", &err))?;

    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| Error::internal("Heimdall state could not be serialized"))?;
    let temp = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name().unwrap_or("state"),
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let written = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if let Err(err) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(Error::from_io("save Heimdall state", &err));
    }
    Ok(())
}
