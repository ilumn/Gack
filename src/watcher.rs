use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

const MAX_WATCH_ENTRIES: usize = 12_000;
const MAX_WATCH_DEPTH: usize = 24;
const NATIVE_WATCH_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable(String),
}

pub struct FsWatcher {
    receiver: Receiver<WatchEvent>,
    backend: WatchBackend,
}

enum WatchBackend {
    Native {
        _watcher: RecommendedWatcher,
    },
    Polling {
        stop: Arc<AtomicBool>,
        handle: Option<JoinHandle<()>>,
    },
}

impl FsWatcher {
    pub fn start(
        root: PathBuf,
        git_dir: PathBuf,
        common_git_dir: PathBuf,
        interval: Duration,
    ) -> Result<Self, String> {
        if !root.is_dir() {
            return Err(format!("{} is not a directory", root.display()));
        }
        if !git_dir.exists() {
            return Err(format!("{} does not exist", git_dir.display()));
        }
        if !common_git_dir.exists() {
            return Err(format!("{} does not exist", common_git_dir.display()));
        }

        Self::start_native(root.clone(), git_dir.clone(), common_git_dir.clone()).or_else(
            |native_err| {
                Self::start_polling(root, git_dir, common_git_dir, interval).map_err(|poll_err| {
                    format!(
                        "native watcher unavailable: {native_err}; polling watcher unavailable: {poll_err}"
                    )
                })
            },
        )
    }

    fn start_native(
        root: PathBuf,
        git_dir: PathBuf,
        common_git_dir: PathBuf,
    ) -> Result<Self, String> {
        verify_native_backend(NATIVE_WATCH_PROBE_TIMEOUT)?;

        let (sender, receiver) = mpsc::channel();
        let watched = WatchScope::new(root.clone(), git_dir.clone(), common_git_dir.clone());
        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<Event>| match result {
                Ok(event) => {
                    if is_relevant_event(&event, &watched) {
                        let _ = sender.send(WatchEvent::Changed);
                    }
                }
                Err(err) => {
                    let _ = sender.send(WatchEvent::Unavailable(err.to_string()));
                }
            },
            Config::default(),
        )
        .map_err(|err| err.to_string())?;

        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|err| format!("could not watch {}: {err}", root.display()))?;
        if !same_path(&root.join(".git"), &git_dir) && !same_path(&root, &git_dir) {
            watcher
                .watch(&git_dir, RecursiveMode::Recursive)
                .map_err(|err| format!("could not watch {}: {err}", git_dir.display()))?;
        }
        if !same_path(&common_git_dir, &git_dir) {
            watcher
                .watch(&common_git_dir, RecursiveMode::Recursive)
                .map_err(|err| format!("could not watch {}: {err}", common_git_dir.display()))?;
        }

        Ok(Self {
            receiver,
            backend: WatchBackend::Native { _watcher: watcher },
        })
    }

    fn start_polling(
        root: PathBuf,
        git_dir: PathBuf,
        common_git_dir: PathBuf,
        interval: Duration,
    ) -> Result<Self, String> {
        let initial = WatchSignature::scan(&root, &git_dir, &common_git_dir)?;
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let mut previous = initial;
            while !thread_stop.load(Ordering::Relaxed) {
                thread::sleep(interval);
                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                match WatchSignature::scan(&root, &git_dir, &common_git_dir) {
                    Ok(next) => {
                        if next != previous {
                            previous = next;
                            if sender.send(WatchEvent::Changed).is_err() {
                                break;
                            }
                        }
                    }
                    Err(err) => {
                        let _ = sender.send(WatchEvent::Unavailable(err));
                        break;
                    }
                }
            }
        });

        Ok(Self {
            receiver,
            backend: WatchBackend::Polling {
                stop,
                handle: Some(handle),
            },
        })
    }

    pub fn try_recv(&self) -> Result<Option<WatchEvent>, TryRecvError> {
        let mut changed = false;
        loop {
            match self.receiver.try_recv() {
                Ok(WatchEvent::Changed) => changed = true,
                Ok(event @ WatchEvent::Unavailable(_)) => return Ok(Some(event)),
                Err(TryRecvError::Empty) => {
                    return Ok(changed.then_some(WatchEvent::Changed));
                }
                Err(err @ TryRecvError::Disconnected) => return Err(err),
            }
        }
    }

    #[cfg(test)]
    fn is_native(&self) -> bool {
        matches!(self.backend, WatchBackend::Native { .. })
    }
}

impl Drop for FsWatcher {
    fn drop(&mut self) {
        if let WatchBackend::Polling { stop, handle } = &mut self.backend {
            stop.store(true, Ordering::Relaxed);
            if let Some(handle) = handle.take() {
                let _ = handle.join();
            }
        }
    }
}

fn verify_native_backend(timeout: Duration) -> Result<(), String> {
    let probe_dir = create_probe_dir()?;
    let result = verify_native_backend_in(&probe_dir, timeout);
    let _ = fs::remove_dir_all(&probe_dir);
    result
}

fn verify_native_backend_in(probe_dir: &Path, timeout: Duration) -> Result<(), String> {
    let (sender, receiver) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |result: notify::Result<Event>| {
            let message = result.map(|_| ()).map_err(|err| err.to_string());
            let _ = sender.send(message);
        },
        Config::default(),
    )
    .map_err(|err| err.to_string())?;
    watcher
        .watch(probe_dir, RecursiveMode::Recursive)
        .map_err(|err| format!("native watcher probe could not watch temp dir: {err}"))?;

    let probe_file = probe_dir.join("gack-watch-probe");
    fs::write(&probe_file, b"probe")
        .map_err(|err| format!("native watcher probe could not write temp file: {err}"))?;

    match receiver.recv_timeout(timeout) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(format!("native watcher probe failed: {err}")),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "native watcher probe did not receive an event within {}ms",
            timeout.as_millis()
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("native watcher probe stopped unexpectedly".to_string())
        }
    }
}

fn create_probe_dir() -> Result<PathBuf, String> {
    let base = env::temp_dir();
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    for attempt in 0..16 {
        let dir = base.join(format!(
            "gack-watch-probe-{}-{timestamp}-{attempt}",
            process::id()
        ));
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(format!(
                    "native watcher probe could not create {}: {err}",
                    dir.display()
                ));
            }
        }
    }
    Err("native watcher probe could not create a unique temp dir".to_string())
}

struct WatchScope {
    root: WatchPath,
    target: WatchPath,
    git_dir: WatchPath,
    common_git_dir: WatchPath,
}

impl WatchScope {
    fn new(root: PathBuf, git_dir: PathBuf, common_git_dir: PathBuf) -> Self {
        let target = root.join("target");
        Self {
            root: WatchPath::new(root),
            target: WatchPath::new(target),
            git_dir: WatchPath::new(git_dir),
            common_git_dir: WatchPath::new(common_git_dir),
        }
    }
}

struct WatchPath {
    raw: PathBuf,
    canonical: Option<PathBuf>,
}

impl WatchPath {
    fn new(path: PathBuf) -> Self {
        let canonical = fs::canonicalize(&path).ok();
        Self {
            raw: path,
            canonical,
        }
    }

    fn contains(&self, path: &Path) -> bool {
        is_under(path, &self.raw)
            || self
                .canonical
                .as_ref()
                .is_some_and(|canonical| is_under(path, canonical))
    }
}

fn is_relevant_event(event: &Event, watched: &WatchScope) -> bool {
    event.paths.is_empty()
        || event
            .paths
            .iter()
            .any(|path| is_relevant_path(path, watched))
}

fn is_relevant_path(path: &Path, watched: &WatchScope) -> bool {
    if watched.git_dir.contains(path) || watched.common_git_dir.contains(path) {
        return true;
    }
    if watched.target.contains(path) {
        return false;
    }
    watched.root.contains(path)
}

fn is_under(path: &Path, parent: &Path) -> bool {
    path == parent || path.starts_with(parent)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WatchSignature {
    hash: u64,
}

impl WatchSignature {
    fn scan(root: &Path, git_dir: &Path, common_git_dir: &Path) -> Result<Self, String> {
        let mut scanner = SignatureScanner::default();
        scanner.scan_worktree(root, git_dir)?;
        scanner.scan_git_metadata(git_dir)?;
        if common_git_dir != git_dir {
            scanner.scan_git_metadata(common_git_dir)?;
        }
        Ok(Self {
            hash: scanner.finish(),
        })
    }
}

#[derive(Default)]
struct SignatureScanner {
    hasher: DefaultHasher,
    entries: usize,
}

impl SignatureScanner {
    fn scan_worktree(&mut self, root: &Path, git_dir: &Path) -> Result<(), String> {
        self.scan_dir(root, root, Some(git_dir), 0)
    }

    fn scan_git_metadata(&mut self, git_dir: &Path) -> Result<(), String> {
        for name in [
            "HEAD",
            "ORIG_HEAD",
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REBASE_HEAD",
            "FETCH_HEAD",
            "index",
            "packed-refs",
        ] {
            let path = git_dir.join(name);
            if path.exists() {
                self.scan_path(git_dir, &path, None, 0)?;
            }
        }
        for name in ["refs", "rebase-merge", "rebase-apply"] {
            let path = git_dir.join(name);
            if path.exists() {
                self.scan_path(git_dir, &path, None, 0)?;
            }
        }
        Ok(())
    }

    fn scan_path(
        &mut self,
        base: &Path,
        path: &Path,
        excluded_dir: Option<&Path>,
        depth: usize,
    ) -> Result<(), String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|err| format!("could not stat {}: {err}", path.display()))?;
        if metadata.file_type().is_dir() {
            self.scan_dir(base, path, excluded_dir, depth)
        } else {
            self.record_entry_metadata(base, path, &metadata)
        }
    }

    fn scan_dir(
        &mut self,
        base: &Path,
        dir: &Path,
        excluded_dir: Option<&Path>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > MAX_WATCH_DEPTH {
            return Err(format!(
                "watch depth limit exceeded under {}",
                dir.display()
            ));
        }
        if excluded_dir.is_some_and(|excluded| same_path(dir, excluded)) {
            return Ok(());
        }
        self.record_entry(base, dir)?;

        let mut entries = fs::read_dir(dir)
            .map_err(|err| format!("could not read {}: {err}", dir.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("could not read {}: {err}", dir.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if excluded_dir.is_some_and(|excluded| same_path(&path, excluded)) {
                continue;
            }
            self.scan_path(base, &path, excluded_dir, depth + 1)?;
        }
        Ok(())
    }

    fn record_entry(&mut self, base: &Path, path: &Path) -> Result<(), String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|err| format!("could not stat {}: {err}", path.display()))?;
        self.record_entry_metadata(base, path, &metadata)
    }

    fn record_entry_metadata(
        &mut self,
        base: &Path,
        path: &Path,
        metadata: &fs::Metadata,
    ) -> Result<(), String> {
        self.entries += 1;
        if self.entries > MAX_WATCH_ENTRIES {
            return Err(format!(
                "watch entry limit exceeded at {}; falling back to periodic refresh",
                path.display()
            ));
        }
        path.strip_prefix(base)
            .unwrap_or(path)
            .to_string_lossy()
            .hash(&mut self.hasher);
        metadata.file_type().is_dir().hash(&mut self.hasher);
        metadata.file_type().is_file().hash(&mut self.hasher);
        metadata.len().hash(&mut self.hasher);
        modified_nanos(metadata).hash(&mut self.hasher);
        Ok(())
    }

    fn finish(self) -> u64 {
        self.hasher.finish()
    }
}

fn modified_nanos(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || fs::canonicalize(left)
            .ok()
            .zip(fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_changes_when_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        fs::create_dir(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(dir.path().join("file.txt"), "one\n").unwrap();

        let first = WatchSignature::scan(dir.path(), &git, &git).unwrap();
        fs::write(dir.path().join("file.txt"), "two\n").unwrap();
        let second = WatchSignature::scan(dir.path(), &git, &git).unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn signature_changes_when_git_metadata_changes() {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        fs::create_dir(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let first = WatchSignature::scan(dir.path(), &git, &git).unwrap();
        fs::write(git.join("index"), "changed").unwrap();
        let second = WatchSignature::scan(dir.path(), &git, &git).unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn watcher_reports_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        fs::create_dir(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let watcher = FsWatcher::start(
            dir.path().to_path_buf(),
            git.clone(),
            git,
            Duration::from_millis(50),
        )
        .unwrap();
        let backend = if watcher.is_native() {
            "native"
        } else {
            "polling fallback"
        };

        fs::write(dir.path().join("file.txt"), "changed\n").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match watcher.try_recv() {
                Ok(Some(WatchEvent::Changed)) => return,
                Ok(Some(WatchEvent::Unavailable(reason))) => {
                    panic!("watcher became unavailable: {reason}");
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                Err(err) => panic!("watcher channel failed: {err}"),
            }
        }
        panic!("{backend} watcher did not report a file change");
    }
}
