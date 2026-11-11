//! Config hot reload: content-hash polling of the config file.
//!
//! The driver loop asks the [`ConfigWatcher`] whether the file changed;
//! applying the new configuration (what applies live, what waits for a
//! restart) is the composition root's job. Hashing rather than mtime is
//! deliberate: editors save through temp-file renames, which mtime
//! handling misreports, while content is the truth.

use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{
        Hash,
        Hasher,
    },
    path::{
        Path,
        PathBuf,
    },
};

use tracing::warn;

use crate::domain::engine::Millis;

/// How often the config file is hashed for changes.
const CHECK_INTERVAL_MS: Millis = 1_000;

/// Watches the config file for content changes by re-hashing it on a
/// fixed cadence. The hash is not cryptographic — it only detects change.
#[derive(Debug)]
pub struct ConfigWatcher {
    path: PathBuf,
    last_hash: Option<u64>,
    next_check: Millis,
    read_failed: bool,
}

impl ConfigWatcher {
    /// Starts watching `path`. The first [`ConfigWatcher::changed`] call
    /// establishes the baseline and never reports a change.
    #[must_use]
    pub fn new(path: &Path) -> Self {
        Self { path: path.to_path_buf(), last_hash: None, next_check: 0, read_failed: false }
    }

    /// The watched config path, re-read when a change is reported.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the file's content changed since the last reported change.
    /// Read failures are tolerated (an editor mid-rename): each failure
    /// streak warns once and reports nothing.
    pub fn changed(&mut self, now: Millis) -> bool {
        if now < self.next_check {
            return false;
        }
        self.next_check = now.saturating_add(CHECK_INTERVAL_MS);
        let Ok(bytes) = fs::read(&self.path) else {
            if !self.read_failed {
                self.read_failed = true;
                warn!(
                    "config file {} is unreadable; the loaded configuration stands",
                    self.path.display()
                );
            }
            return false;
        };
        self.read_failed = false;
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let hash = hasher.finish();
        match self.last_hash {
            None => {
                self.last_hash = Some(hash);
                false
            }
            Some(last) if last != hash => {
                self.last_hash = Some(hash);
                true
            }
            Some(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        fs,
        path::{
            Path,
            PathBuf,
        },
    };

    use super::ConfigWatcher;

    /// Unique-enough temp paths per test tag; tests write and clean up
    /// themselves. A write that cannot happen silently skips the test,
    /// mirroring the config-module tests.
    fn temp_path(tag: &str) -> PathBuf {
        env::temp_dir().join(format!("simple-clicker-reload-{tag}-{}.toml", std::process::id()))
    }

    fn try_write(path: &Path, content: &str) -> bool {
        fs::write(path, content).is_ok()
    }

    #[test]
    fn the_first_check_establishes_the_baseline_without_reporting() {
        let path = temp_path("baseline");
        if !try_write(&path, "a = 1") {
            return;
        }
        let mut watcher = ConfigWatcher::new(&path);
        assert!(!watcher.changed(0), "the baseline check must not report");
        assert!(!watcher.changed(2_000), "unchanged content must not report");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn content_changes_are_reported_once() {
        let path = temp_path("change");
        if !try_write(&path, "a = 1") {
            return;
        }
        let mut watcher = ConfigWatcher::new(&path);
        assert!(!watcher.changed(0));
        assert!(try_write(&path, "a = 2"));
        assert!(watcher.changed(1_000), "new content must report exactly once");
        assert!(!watcher.changed(2_000), "the same content must not report again");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn checks_are_throttled_to_the_interval() {
        let path = temp_path("throttle");
        if !try_write(&path, "a = 1") {
            return;
        }
        let mut watcher = ConfigWatcher::new(&path);
        assert!(!watcher.changed(0));
        assert!(try_write(&path, "a = 2"));
        assert!(!watcher.changed(500), "inside the check interval nothing is read");
        assert!(watcher.changed(1_000), "the next due check must see the change");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn an_unreadable_file_is_tolerated_and_recovers() {
        let path = temp_path("missing");
        let _ = fs::remove_file(&path);
        let mut watcher = ConfigWatcher::new(&path);
        assert!(!watcher.changed(0), "a missing file must not report or panic");
        if !try_write(&path, "a = 1") {
            return;
        }
        // The first readable check becomes the new baseline.
        assert!(!watcher.changed(1_000));
        assert!(try_write(&path, "a = 2"));
        assert!(watcher.changed(2_000), "changes after recovery must report");
        let _ = fs::remove_file(&path);
    }
}
