//! Read caches for the hot config sections.
//!
//! `settings.json` and `library.json` are read from hot paths, so they are served from a
//! process cache that is reused while the file keeps the same size and modification time,
//! and invalidated after every successful save.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::SystemTime;

use super::sections::{Library, Settings};
use super::storage::STAMP_SETTLE;

/// Identity of a file on disk. A cached section is reused while the file keeps the same
/// size and modification time; any write (our own or another process's) invalidates it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    modified: Option<std::time::SystemTime>,
    len: u64,
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    fs::metadata(path).ok().map(|metadata| FileStamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

impl FileStamp {
    /// A stamp is trustworthy only once the timestamp behind it is old enough to have been
    /// recorded in full. A fresh file is re-read on every access until then, so a coarse
    /// timestamp cannot hide a rewrite and a cached default cannot pin a transient failure.
    fn is_settled(self, now: SystemTime) -> bool {
        self.modified.is_some_and(|modified| {
            now.duration_since(modified)
                .is_ok_and(|age| age >= STAMP_SETTLE)
        })
    }
}

pub(super) struct CachedSection<T> {
    path: PathBuf,
    stamp: Option<FileStamp>,
    value: Arc<T>,
}

/// `settings.json` and `library.json` are read from hot paths (every action step, every
/// Lua `ph.overlay_*` call, overlay windows). The cache turns those reads into one
/// `stat` per call; a miss re-reads and re-parses the file.
pub(super) static SETTINGS_CACHE: LazyLock<RwLock<Option<CachedSection<Settings>>>> =
    LazyLock::new(|| RwLock::new(None));
pub(super) static LIBRARY_CACHE: LazyLock<RwLock<Option<CachedSection<Library>>>> =
    LazyLock::new(|| RwLock::new(None));

pub(super) fn cached_section<T: Default>(
    cache: &RwLock<Option<CachedSection<T>>>,
    path: PathBuf,
    load: impl FnOnce(&Path) -> Option<T>,
) -> Arc<T> {
    let stamp = file_stamp(&path);
    let now = SystemTime::now();
    if let Ok(guard) = cache.read() {
        if let Some(entry) = guard.as_ref() {
            if entry.path == path
                && entry.stamp == stamp
                && stamp.is_some_and(|stamp| stamp.is_settled(now))
            {
                return Arc::clone(&entry.value);
            }
        }
    }
    let Some(loaded) = load(&path) else {
        // A failed read is never cached: one transient sharing or permission error would
        // otherwise pin defaults for the rest of the process lifetime.
        return Arc::new(T::default());
    };
    let value = Arc::new(loaded);
    if let Ok(mut guard) = cache.write() {
        *guard = Some(CachedSection {
            path,
            stamp,
            value: Arc::clone(&value),
        });
    }
    value
}

pub(super) fn invalidate_section<T>(cache: &RwLock<Option<CachedSection<T>>>) {
    if let Ok(mut guard) = cache.write() {
        *guard = None;
    }
}
