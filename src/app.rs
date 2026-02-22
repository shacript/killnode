//! # Application state
//!
//! This module is the heart of the app. [`App`] owns every piece of state that
//! the UI reads and the event loop mutates — from the list of discovered
//! `node_modules` folders, to which ones the user has selected, to what the
//! background threads are currently doing.
//!
//! ## How screens flow
//!
//! The app moves through a linear sequence of [`Screen`] variants. Think of
//! it as a tiny state machine:
//!
//! ```text
//! Welcome ──(Enter)──► Scanning ──(done)──► List ──(Enter)──► Confirming
//!                                               ▲                  │
//!                                               └──(Esc)───────────┘
//!                                                        │
//!                                                    (Y / Enter)
//!                                                        │
//!                                                        ▼
//!                                                    Deleting ──(done)──► Done
//! ```
//!
//! Each variant corresponds to a different screen rendered by the `ui` module.
//! The only valid transitions are the ones shown above — there is no way to
//! go backwards past the `List` screen, for example.
//!
//! ## Background threads
//!
//! Scanning and deletion both run on their own threads and communicate back
//! to the main thread via [`std::sync::mpsc`] channels. The event loop calls
//! [`App::process_scan_messages`] and [`App::process_delete_messages`] on
//! every tick to drain whatever messages have arrived since the last frame.
//! This keeps the UI responsive — it never blocks waiting for a thread.

use std::{path::PathBuf, sync::mpsc::Receiver};

use ratatui::widgets::ListState;

use crate::{
    deleter::{DeleteMsg, start_delete},
    scanner::{ActiveScan, ScanMsg, start_scan},
};

// ─── Data types ───────────────────────────────────────────────────────────────

/// A single `node_modules` directory discovered during a scan.
///
/// Every entry in [`App::entries`] corresponds to one directory on disk.
/// Entries are sorted by size (largest first) once the scan finishes, so the
/// most impactful candidates appear at the top of the list.
pub struct NodeModuleEntry {
    /// Absolute path to the `node_modules` directory.
    pub path: String,

    /// Total size of all files inside the directory, in bytes.
    /// Calculated once during scanning via a recursive walk.
    pub size: u64,

    /// Whether this directory lives inside a location that is considered
    /// unsafe to delete automatically (e.g. `~/.config`, AppData/Roaming).
    /// Sensitive entries are shown with a warning indicator and cannot be
    /// selected. See [`crate::scanner::is_sensitive_dir`] for the full rules.
    pub sensitive: bool,

    /// Whether the user has marked this entry for deletion.
    ///
    /// Defaults to `true` for normal entries and `false` for sensitive ones
    /// so that safe entries are opt-out rather than opt-in.
    pub selected: bool,

    /// When the directory was last modified, expressed as seconds since the
    /// Unix epoch. Used to display a human-friendly "X days ago" label.
    /// `None` if the filesystem didn't return valid metadata.
    pub last_modified: Option<u64>,
}

/// Which screen is currently visible and accepting input.
///
/// The variant also implicitly defines what keys are active — the event loop
/// in `main.rs` matches on this to route keypresses to the right handler.
pub enum Screen {
    /// The opening screen. Shows the scan root and waits for the user to
    /// press Enter before doing anything.
    Welcome,

    /// A scan is running in the background. Shows a spinner and a live
    /// "currently scanning: <path>" readout. No navigation is possible here.
    Scanning,

    /// The scan has finished. Shows the full list of discovered directories
    /// so the user can browse and select which ones to delete.
    List,

    /// An overlay popup rendered on top of [`Screen::List`], asking the user
    /// to confirm before deletion begins. Pressing Esc returns to the list.
    Confirming,

    /// Deletion is running in the background. Shows a progress bar and the
    /// path currently being removed. Input is blocked during this phase.
    Deleting,

    /// Everything is finished (or nothing was found). Shows a summary of
    /// how much space was freed and any errors that occurred.
    Done,
}

/// All runtime state for the application.
///
/// A single `App` instance is created in [`crate::run`] and lives for the
/// entire duration of the process. The UI reads from it on every frame, and
/// the event loop writes to it in response to keypresses and thread messages.
pub struct App {
    /// Which screen is currently being rendered.
    pub screen: Screen,

    /// The root directory the user asked to scan. Taken from the first CLI
    /// argument, or the current working directory if none was given.
    pub scan_root: String,

    /// All `node_modules` directories found so far, sorted largest-first once
    /// scanning completes. Populated incrementally by [`Self::process_scan_messages`].
    pub entries: Vec<NodeModuleEntry>,

    /// Ratatui's scroll/selection state for the list widget. Kept here so the
    /// UI can pass it by `&mut` reference on every frame without storing it
    /// separately.
    pub list_state: ListState,

    // ── Scanning ──────────────────────────────────────────────────────────────
    /// Handle to the active background scan, if one is in progress.
    /// `None` before the first scan starts or after it finishes.
    pub scan: Option<ActiveScan>,

    // ── Deletion ──────────────────────────────────────────────────────────────
    /// Receiving end of the channel the background delete thread sends
    /// progress messages on. `None` when no deletion is in progress.
    pub delete_rx: Option<Receiver<DeleteMsg>>,

    /// How many directories have been deleted so far in the current run.
    pub delete_done: usize,

    /// Total number of directories queued for deletion in the current run.
    pub delete_total: usize,

    /// Path of the directory currently being removed. Shown on the
    /// [`Screen::Deleting`] screen so the user knows what's happening.
    pub delete_current: String,

    /// Total bytes freed across all successfully deleted directories.
    pub delete_freed: u64,

    /// Paths that could not be deleted, along with the error message for each.
    /// Displayed on the [`Screen::Done`] screen if non-empty.
    pub delete_errors: Vec<String>,

    // ── Animation ─────────────────────────────────────────────────────────────
    /// Incremented by one on every event-loop iteration. Used to drive
    /// spinner animations: `SPINNER[ticker / 2 % SPINNER.len()]` advances
    /// the frame roughly every 160 ms.
    pub ticker: u64,
}

// ─── Construction ─────────────────────────────────────────────────────────────

impl App {
    /// Creates a fresh `App` ready to show the [`Screen::Welcome`] screen.
    ///
    /// The scan root is determined once here and never changes:
    /// - If the user passed a path as the first CLI argument, that is used.
    /// - Otherwise the current working directory is used.
    /// - If even `cwd` fails (rare, but possible on some systems), `.` is
    ///   used as a last resort so the app always starts successfully.
    pub fn new() -> Self {
        let scan_root = std::env::args().nth(1).unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .to_string_lossy()
                .to_string()
        });

        Self {
            screen: Screen::Welcome,
            scan_root,
            entries: Vec::new(),
            list_state: ListState::default(),
            scan: None,
            delete_rx: None,
            delete_done: 0,
            delete_total: 0,
            delete_current: String::new(),
            delete_freed: 0,
            delete_errors: Vec::new(),
            ticker: 0,
        }
    }
}

// ─── Scanning ─────────────────────────────────────────────────────────────────

impl App {
    /// Clears any previous results and kicks off a new background scan.
    ///
    /// Resets the entry list and list selection so stale data from a previous
    /// run (if any) doesn't flash on screen before new results arrive.
    /// Transitions immediately to [`Screen::Scanning`].
    pub fn begin_scan(&mut self) {
        self.entries.clear();
        self.list_state = ListState::default();
        self.scan = Some(start_scan(self.scan_root.clone()));
        self.screen = Screen::Scanning;
    }

    /// Drains all messages that have arrived from the background scan thread
    /// since the last call, without blocking.
    ///
    /// This is called on every event-loop tick so results stream in as fast
    /// as the scanner produces them. When a [`ScanMsg::Done`] is received:
    ///
    /// - Entries are sorted largest-first (so the biggest space-wasters are
    ///   at the top of the list).
    /// - If at least one entry was found, transitions to [`Screen::List`] with
    ///   the first item selected.
    /// - If nothing was found, transitions straight to [`Screen::Done`] so the
    ///   user gets immediate feedback rather than an empty list.
    ///
    /// Does nothing if no scan is active or if the scan already finished.
    pub fn process_scan_messages(&mut self) {
        let is_done = self.scan.as_ref().map_or(true, |s| s.done);
        if is_done {
            return;
        }

        loop {
            let msg = self.scan.as_ref().unwrap().rx.try_recv();
            match msg {
                Ok(ScanMsg::Found {
                    path,
                    size,
                    sensitive,
                    last_modified,
                }) => {
                    self.entries.push(NodeModuleEntry {
                        // Pre-select safe entries so the user can just hit Enter
                        // to delete everything without manually selecting each one.
                        selected: !sensitive,
                        path,
                        size,
                        sensitive,
                        last_modified,
                    });
                }
                Ok(ScanMsg::Done) => {
                    if let Some(s) = &mut self.scan {
                        s.done = true;
                    }
                    self.entries.sort_by(|a, b| b.size.cmp(&a.size));
                    if self.entries.is_empty() {
                        self.screen = Screen::Done;
                    } else {
                        self.list_state.select(Some(0));
                        self.screen = Screen::List;
                    }
                    break;
                }
                // Channel is empty (or disconnected) — nothing more to do this tick.
                Err(_) => break,
            }
        }
    }

    /// Returns the path the scanner is currently visiting, for display on the
    /// [`Screen::Scanning`] screen.
    ///
    /// Returns an empty string when no scan is active or if the mutex can't
    /// be acquired (which should never happen in practice, but is handled
    /// gracefully rather than panicking).
    pub fn current_scanning_path(&self) -> String {
        self.scan
            .as_ref()
            .and_then(|s| s.current_path.lock().ok())
            .map(|g| g.clone())
            .unwrap_or_default()
    }
}

// ─── List navigation & selection ──────────────────────────────────────────────

impl App {
    /// Moves the highlighted row up by one, wrapping around to the bottom of
    /// the list if the cursor is already at the top.
    pub fn navigate_up(&mut self) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => (i + len - 1) % len,
            None => len - 1,
        };
        self.list_state.select(Some(i));
    }

    /// Moves the highlighted row down by one, wrapping around to the top of
    /// the list if the cursor is already at the bottom.
    pub fn navigate_down(&mut self) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => (i + 1) % len,
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    /// Flips the selection state of the currently highlighted entry.
    ///
    /// Sensitive entries can be selected, but they are never pre-selected
    /// automatically — the user must explicitly choose to include them.
    pub fn toggle_selected(&mut self) {
        if let Some(i) = self.list_state.selected() {
            if let Some(entry) = self.entries.get_mut(i) {
                entry.selected = !entry.selected;
            }
        }
    }

    /// Smartly toggles all non-sensitive entries at once.
    ///
    /// The rule is: *if even one safe entry is currently unselected, select
    /// all of them; otherwise deselect all of them.* This means the first
    /// press of `a` always results in every safe entry being selected, and
    /// the second press deselects them all — which feels natural.
    ///
    /// Sensitive entries are never touched by this method. Use
    /// [`Self::toggle_all_force`] to include them.
    pub fn toggle_all(&mut self) {
        let any_unselected = self.entries.iter().any(|e| !e.sensitive && !e.selected);
        for entry in &mut self.entries {
            if !entry.sensitive {
                entry.selected = any_unselected;
            }
        }
    }

    /// Smartly toggles **all** entries at once, including sensitive ones.
    ///
    /// Follows the same smart rule as [`Self::toggle_all`]: if even one entry
    /// of any kind is unselected, everything gets selected; otherwise
    /// everything gets deselected.
    ///
    /// Because this selects sensitive directories, the confirmation popup
    /// will show a warning before deletion proceeds.
    pub fn toggle_all_force(&mut self) {
        let any_unselected = self.entries.iter().any(|e| !e.selected);
        for entry in &mut self.entries {
            entry.selected = any_unselected;
        }
    }

    /// Returns how many entries are currently selected for deletion.
    pub fn selected_count(&self) -> usize {
        self.entries.iter().filter(|e| e.selected).count()
    }

    /// Returns the combined size (in bytes) of all selected entries.
    /// Shown in the help bar and the confirmation popup as the amount of
    /// space that will be freed.
    pub fn selected_size(&self) -> u64 {
        self.entries
            .iter()
            .filter(|e| e.selected)
            .map(|e| e.size)
            .sum()
    }

    /// Returns the combined size (in bytes) of *all* discovered entries,
    /// regardless of selection state. Shown in the list screen title.
    pub fn total_size(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }

    /// Collects the paths of all selected entries into a `Vec`.
    /// Used by [`Self::begin_delete`] to hand the list off to the background
    /// thread — after which the selection state on the entries themselves no
    /// longer matters.
    fn selected_paths(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.selected)
            .map(|e| e.path.clone())
            .collect()
    }
}

// ─── Deletion ─────────────────────────────────────────────────────────────────

impl App {
    /// Collects the selected paths and hands them to a background delete
    /// thread, then transitions to [`Screen::Deleting`].
    ///
    /// All deletion counters are reset here so a fresh run always starts from
    /// zero, even if the user somehow triggered deletion twice (which the UI
    /// prevents, but defensive resets are cheap).
    pub fn begin_delete(&mut self) {
        let paths = self.selected_paths();
        self.delete_total = paths.len();
        self.delete_done = 0;
        self.delete_current = String::new();
        self.delete_freed = 0;
        self.delete_errors.clear();
        self.delete_rx = Some(start_delete(paths));
        self.screen = Screen::Deleting;
    }

    /// Drains all messages that have arrived from the background delete thread
    /// since the last call, without blocking.
    ///
    /// Each [`DeleteMsg::Progress`] message advances the progress counter and
    /// updates the "currently removing" path shown on screen.
    ///
    /// When [`DeleteMsg::Done`] arrives the final freed-bytes total and any
    /// error messages are recorded, and the screen transitions to
    /// [`Screen::Done`].
    ///
    /// Does nothing if no deletion is currently in progress.
    pub fn process_delete_messages(&mut self) {
        if self.delete_rx.is_none() {
            return;
        }

        loop {
            let msg = self.delete_rx.as_ref().unwrap().try_recv();
            match msg {
                Ok(DeleteMsg::Progress(path)) => {
                    self.delete_current = path;
                    self.delete_done += 1;
                }
                Ok(DeleteMsg::Done { freed, errors }) => {
                    self.delete_freed = freed;
                    self.delete_errors = errors;
                    self.delete_rx = None;
                    self.screen = Screen::Done;
                    break;
                }
                // Channel is empty (or disconnected) — nothing more to do this tick.
                Err(_) => break,
            }
        }
    }
}
