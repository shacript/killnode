//! # Background deleter
//!
//! This module takes a list of paths and removes them, one by one, on a
//! background thread — keeping the UI responsive throughout.
//!
//! ## How it works
//!
//! [`start_delete`] spawns a thread and returns the receiving end of a channel
//! immediately. The thread sends a [`DeleteMsg::Progress`] message just before
//! it starts removing each directory, so the UI can show which path is
//! currently being deleted. When all paths have been processed it sends a
//! single [`DeleteMsg::Done`] with the total bytes freed and a list of any
//! errors that occurred.
//!
//! The main thread drains the channel on every event-loop tick via
//! [`crate::app::App::process_delete_messages`] without blocking, so the
//! progress bar advances smoothly even on a slow filesystem.
//!
//! ## Error handling
//!
//! Deletion errors are non-fatal. If `remove_dir_all` fails for one path, the
//! error is recorded and the thread moves on to the next path. This means a
//! permission error on one directory won't prevent the others from being
//! cleaned up. All errors are collected and shown to the user on the
//! [`crate::app::Screen::Done`] screen at the end.

use std::{
    path::Path,
    sync::mpsc::{self, Receiver, Sender},
};

use crate::scanner::dir_size;

// ─── Types ────────────────────────────────────────────────────────────────────

/// A message sent from the background delete thread to the main thread.
pub enum DeleteMsg {
    /// Sent immediately *before* a directory is removed.
    ///
    /// The enclosed string is the path about to be deleted. Sending this
    /// before (rather than after) the deletion means the UI always shows
    /// what the thread is currently working on, even if a particular
    /// `remove_dir_all` call takes a long time.
    Progress(String),

    /// Sent once, after all directories have been processed.
    ///
    /// `freed` is the total number of bytes successfully reclaimed.
    /// `errors` contains a human-readable message for each path that
    /// could not be deleted (empty if everything succeeded).
    Done { freed: u64, errors: Vec<String> },
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Spawns a background thread to delete `paths` and returns the receiving end
/// of the progress channel.
///
/// The caller should hold onto the returned [`Receiver`] and drain it on every
/// event-loop tick (via [`crate::app::App::process_delete_messages`]) until a
/// [`DeleteMsg::Done`] is received.
///
/// Like the scanner, the thread is intentionally detached. If the receiver is
/// dropped before `Done` arrives the thread will notice the channel is broken
/// on its next send and exit early.
pub fn start_delete(paths: Vec<String>) -> Receiver<DeleteMsg> {
    let (tx, rx) = mpsc::channel::<DeleteMsg>();
    std::thread::spawn(move || delete_thread(paths, tx));
    rx
}

// ─── Background thread ────────────────────────────────────────────────────────

/// The function that runs on the background delete thread.
///
/// For each path in `paths`:
///
/// 1. Sends [`DeleteMsg::Progress`] so the UI can update the "currently
///    removing" label before any blocking I/O begins.
///
/// 2. Measures the directory size *before* deleting it, because once it's
///    gone there is nothing left to measure. The size is only added to
///    `freed` if the deletion actually succeeds.
///
/// 3. Calls [`std::fs::remove_dir_all`]. On success, adds the size to the
///    running `freed` total. On failure, appends a human-readable error
///    string and continues to the next path.
///
/// After all paths are processed, sends [`DeleteMsg::Done`] with the final
/// totals.
fn delete_thread(paths: Vec<String>, tx: Sender<DeleteMsg>) {
    let mut freed: u64 = 0;
    let mut errors: Vec<String> = Vec::new();

    for path in &paths {
        // Notify the UI first so it shows this path while the deletion runs.
        tx.send(DeleteMsg::Progress(path.clone())).ok();

        let p = Path::new(path);

        // Measure before deleting — there will be nothing to measure after.
        let size = dir_size(p);

        match std::fs::remove_dir_all(p) {
            Ok(_) => freed += size,
            Err(e) => errors.push(format!("{path}: {e}")),
        }
    }

    tx.send(DeleteMsg::Done { freed, errors }).ok();
}
