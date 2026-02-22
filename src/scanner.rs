//! # Background scanner
//!
//! This module is responsible for finding every `node_modules` directory
//! under a given root path as fast as possible, without freezing the UI.
//!
//! ## How it works
//!
//! [`start_scan`] spawns a single background thread that walks the directory
//! tree using [`jwalk::WalkDir`], which parallelises the filesystem I/O
//! internally. As each `node_modules` directory is found, the thread sends a
//! [`ScanMsg::Found`] message over an [`std::sync::mpsc`] channel. When the
//! walk is complete it sends [`ScanMsg::Done`]. The main thread (via
//! [`crate::app::App::process_scan_messages`]) drains the channel on every
//! event-loop tick without blocking, so the UI stays responsive throughout.
//!
//! ## One level deep, intentionally
//!
//! The walker is configured to **not** descend into `node_modules` once it
//! finds one. This means nested `node_modules` (e.g. inside a package inside
//! another package) are ignored. That is the right behaviour: deleting the
//! top-level directory removes everything inside it anyway, and reporting
//! nested ones would just create confusing duplicates in the list.
//!
//! ## Sensitive path detection
//!
//! Not all `node_modules` directories are safe to delete. A global npm cache
//! at `~/.npm` or an app bundled inside `/Applications/Foo.app` should not be
//! touched. [`is_sensitive_dir`] encodes the rules for what counts as
//! "sensitive" on each platform; entries that match are flagged and shown with
//! a warning in the UI rather than being pre-selected for deletion.

use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, Sender},
    },
};

use jwalk::WalkDir;

// ─── Public types ─────────────────────────────────────────────────────────────

/// A handle to a scan that is currently running in the background.
///
/// The event loop holds one of these while [`crate::app::Screen::Scanning`] is
/// active. Every field is cheap to clone or read from the main thread.
pub struct ActiveScan {
    /// Receiving end of the channel the scanner thread sends messages on.
    /// The main thread calls `try_recv` on this non-blocking on every tick.
    pub rx: mpsc::Receiver<ScanMsg>,

    /// The path the scanner is visiting right now. The background thread
    /// updates this via a mutex; the UI reads it to show a live "currently
    /// scanning:" readout. Wrapped in `Arc` so both threads can share it.
    pub current_path: Arc<Mutex<String>>,

    /// Set to `true` by the main thread once it receives [`ScanMsg::Done`].
    /// Used as a cheap guard so [`crate::app::App::process_scan_messages`]
    /// can return immediately on subsequent ticks without touching the channel.
    pub done: bool,
}

/// A message sent from the scanner thread to the main thread.
pub enum ScanMsg {
    /// A `node_modules` directory was found. Contains everything the UI needs
    /// to display and the app needs to make a deletion decision.
    Found {
        /// Absolute path to the directory.
        path: String,

        /// Total size of all files inside, in bytes.
        size: u64,

        /// Whether the directory lives in a location that should not be
        /// deleted automatically. See [`is_sensitive_dir`] for the rules.
        sensitive: bool,

        /// When the directory was last modified, in seconds since the Unix
        /// epoch. `None` if the OS didn't return valid metadata.
        last_modified: Option<u64>,
    },

    /// The walk has finished. No more `Found` messages will be sent.
    Done,
}

// ─── Size calculation ─────────────────────────────────────────────────────────

/// Calculates the total size of all files inside `path`, in bytes.
///
/// This is a recursive walk — every file in every subdirectory is counted.
/// Symlinks to files are followed (their target's size is used); broken
/// symlinks are silently skipped.
///
/// This function is deliberately synchronous. It is only called from the
/// background scan thread, so blocking there is fine.
pub(crate) fn dir_size(path: impl AsRef<Path>) -> u64 {
    WalkDir::new(path)
        .skip_hidden(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

// ─── Path normalisation ───────────────────────────────────────────────────────

/// Normalises a path string for case-insensitive, cross-platform comparison.
///
/// Does two things:
///
/// 1. Replaces all backslashes with forward slashes so Windows and Unix paths
///    can be compared with the same code.
/// 2. Strips a Windows drive letter prefix (`C:/…` → `/…`) so that an
///    absolute Windows path and the same path without a drive letter compare
///    equal when we're only interested in the directory structure.
/// 3. Lowercases everything, because Windows filesystems are case-insensitive.
fn normalize_path(p: &str) -> String {
    let mut s = p.replace('\\', "/").to_lowercase();
    // Strip Windows drive letter: "c:/..." → "/..."
    if s.len() >= 3 && s.as_bytes()[1] == b':' && s.as_bytes()[2] == b'/' {
        s = s[2..].to_string();
    }
    s
}

// ─── Sensitive path detection ─────────────────────────────────────────────────

/// Returns `true` if `path` is in a location where deleting `node_modules`
/// could break something important.
///
/// The goal is to be conservative: it is better to flag something as sensitive
/// and let the user override it manually than to silently delete something that
/// turns out to be load-bearing.
///
/// ## Rules, in order
///
/// ### Inside the user's home directory
///
/// Most `node_modules` found under `~` are fine to delete (they belong to
/// projects). But some hidden directories inside `~` are sensitive:
///
/// - `~/.config/**` — app config, never touch.
/// - `~/.local/share/**` — XDG app data, never touch.
/// - `~/.cache/**` — caches managed by other tools, never touch.
/// - `~/.npm/**` and `~/.pnpm/**` — package manager caches; these look like
///   system directories but are explicitly **safe** to nuke.
/// - Any other `~/.<name>` top-level hidden directory — treated as sensitive
///   because many tools store important state in dotdirs.
///
/// ### macOS application bundles
///
/// A path that looks like `/Applications/Foo.app/…` is considered sensitive.
/// Some Electron apps ship with their own `node_modules` inside the bundle;
/// deleting those would break the app.
///
/// ### Windows UNC paths with hidden segments
///
/// Network paths like `\\server\share\.hidden\…` are treated as sensitive
/// because hidden directories on a network share are usually managed by the
/// server and should not be modified arbitrarily.
///
/// ### Windows AppData
///
/// - `AppData/Roaming/**` — always sensitive (installed apps live here).
/// - `AppData/Local/**` — sensitive, **except** for known package manager
///   cache directories (`.cache`, `.npm`, `.pnpm`) which are safe to delete.
fn is_sensitive_dir(path: impl AsRef<Path>) -> bool {
    let original = path.as_ref();
    let original_str = original.to_string_lossy();

    let is_unc = original_str.starts_with("\\\\") || original_str.starts_with("//");

    // Make sure we're working with an absolute path so prefix comparisons
    // against the home directory are reliable.
    let absolute = if is_unc || original.is_absolute() {
        original.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(original)
    };

    let norm = normalize_path(&absolute.to_string_lossy());
    let norm_original = normalize_path(&original_str);

    // ── Home directory rules ───────────────────────────────────────────────────

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();

    if !home.is_empty() {
        let home_path = Path::new(&home);
        let home_abs = if home_path.is_absolute() {
            home_path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(home_path)
        };
        let norm_home = normalize_path(&home_abs.to_string_lossy());

        let in_home = norm == norm_home || norm.starts_with(&format!("{norm_home}/"));

        if in_home {
            // Strip the home prefix to get the path relative to ~.
            let rel = norm[norm_home.len()..].trim_start_matches('/').to_string();
            let top = rel.split('/').next().unwrap_or("");

            // Always-sensitive XDG directories.
            if rel.starts_with(".config")
                || rel.starts_with(".local/share")
                || rel.starts_with(".cache")
            {
                return true;
            }

            // Package manager caches — look hidden but are safe.
            if top == ".npm" || top == ".pnpm" {
                return false;
            }

            // Anything else under a hidden top-level directory is sensitive.
            if top.starts_with('.') && top != "." && top != ".." {
                return true;
            }
        }
    }

    // ── macOS application bundles ──────────────────────────────────────────────

    // Match paths that look like "/applications/Foo.app/..." where "Foo.app"
    // is a direct child of an Applications folder (no slashes inside the app
    // name before ".app").
    if let Some(idx) = norm.find("/applications/") {
        let rest = &norm[idx + "/applications/".len()..];
        if let Some(app_end) = rest.find(".app/") {
            if !rest[..app_end].contains('/') {
                return true;
            }
        }
    }

    // ── Windows UNC paths with hidden segments ─────────────────────────────────

    // A UNC path looks like "//server/share/path/…". We skip the first four
    // components (the two empty strings, server name, and share name) and
    // check whether any remaining segment starts with a dot.
    if norm_original.starts_with("//") {
        let hidden = norm_original
            .split('/')
            .skip(4)
            .any(|part| !part.is_empty() && part.starts_with('.'));
        if hidden {
            return true;
        }
    }

    // ── Windows AppData ────────────────────────────────────────────────────────

    if norm.contains("/appdata/roaming") {
        return true;
    }

    if norm.contains("/appdata/local") {
        // Carve out the known-safe package manager cache directories.
        let whitelisted = [".cache", ".npm", ".pnpm"]
            .iter()
            .any(|name| norm.contains(&format!("/{name}/")) || norm.ends_with(&format!("/{name}")));
        return !whitelisted;
    }

    false
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Starts a background scan of `root` and returns a handle to it immediately.
///
/// The caller should hold onto the returned [`ActiveScan`] and call
/// [`crate::app::App::process_scan_messages`] (which reads from
/// `ActiveScan::rx`) on every event-loop tick until `ActiveScan::done` is
/// `true`.
///
/// The background thread is intentionally detached — if the caller drops the
/// `ActiveScan` (e.g. the user quits mid-scan) the thread will finish its
/// current directory, fail to send on the now-closed channel, and exit cleanly.
pub fn start_scan(root: String) -> ActiveScan {
    let (tx, rx) = mpsc::channel::<ScanMsg>();
    let current_path = Arc::new(Mutex::new(String::new()));

    let current_path_clone = Arc::clone(&current_path);
    std::thread::spawn(move || scan_thread(root, tx, current_path_clone));

    ActiveScan {
        rx,
        current_path,
        done: false,
    }
}

// ─── Background thread ────────────────────────────────────────────────────────

/// The function that runs on the background scanner thread.
///
/// Walks the directory tree rooted at `root` using `jwalk`, which reads
/// directory contents in parallel using a thread pool internally.
///
/// Two important behaviours are configured via `process_read_dir`:
///
/// 1. **Live path tracking** — every time the walker opens a new directory,
///    its path is written into `current_path` so the UI can show it.
///
/// 2. **Shallow walk** — when a `node_modules` directory is encountered as a
///    *child* entry, its `read_children_path` is set to `None`, which tells
///    `jwalk` not to recurse into it. This means we find the top-level
///    `node_modules` but not any nested ones inside packages.
///
/// For each `node_modules` directory found, a [`ScanMsg::Found`] is sent with
/// its path, size, sensitivity flag, and last-modified time.
///
/// When the walk is complete, a final [`ScanMsg::Done`] is sent.
fn scan_thread(root: String, tx: Sender<ScanMsg>, current_path: Arc<Mutex<String>>) {
    let walker = WalkDir::new(&root).skip_hidden(false).process_read_dir({
        let cp = Arc::clone(&current_path);
        move |_depth, path, _state, children| {
            // Update the live "currently scanning" display.
            if let Ok(mut g) = cp.lock() {
                *g = path.to_string_lossy().to_string();
            }
            // Tell jwalk not to recurse into any node_modules it sees as
            // children of the current directory. We will report the directory
            // itself (in the loop below) but we don't want to walk inside it —
            // that would both be slow and produce spurious nested results.
            for res in children.iter_mut() {
                if let Ok(de) = res {
                    if de.file_name().to_string_lossy() == "node_modules" && de.file_type().is_dir()
                    {
                        de.read_children_path = None;
                    }
                }
            }
        }
    });

    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        if entry.file_name().to_string_lossy() == "node_modules" && entry.file_type().is_dir() {
            let path = entry.path();
            let size = dir_size(&path);
            let sensitive = is_sensitive_dir(&path);
            let last_modified = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|d| d.as_secs())
                });
            tx.send(ScanMsg::Found {
                path: path.to_string_lossy().to_string(),
                size,
                sensitive,
                last_modified,
            })
            .ok();
        }
    }

    tx.send(ScanMsg::Done).ok();
}
