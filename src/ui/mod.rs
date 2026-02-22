//! # UI layer
//!
//! This module is the top-level entry point for all rendering. On every
//! event-loop tick, [`ui`] is called with the current [`App`] state and a
//! Ratatui [`Frame`] to draw into. It simply looks at which [`Screen`] is
//! active and delegates to the appropriate sub-module.
//!
//! ## Module layout
//!
//! | Module | Screen it renders |
//! |--------|-------------------|
//! | [`welcome`] | [`Screen::Welcome`] — opening screen with scan root |
//! | [`scanning`] | [`Screen::Scanning`] — spinner + live path readout |
//! | [`list`] | [`Screen::List`] and [`Screen::Confirming`] — entry list + confirmation popup |
//! | [`deleting`] | [`Screen::Deleting`] — progress gauge |
//! | [`done`] | [`Screen::Done`] — summary and error list |
//!
//! [`Screen::Confirming`] is handled inside `list` rather than its own module
//! because the confirmation dialog is an overlay rendered *on top of* the list —
//! it shares the same underlying layout.
//!
//! ## Shared utilities
//!
//! The bottom of this file contains a small set of helpers used across multiple
//! screens:
//!
//! - [`help_bar`] — renders the row of `[Key] Action` hints at the bottom of
//!   every screen.
//! - [`inner_area`] — shrinks a [`Rect`] by one cell on each side to account
//!   for a border, so content doesn't overlap the box outline.
//! - [`centered_rect`] — computes a centred rectangle for popup dialogs.
//! - [`truncate_left`] — shortens a string from the left, keeping the *end*
//!   visible. Used for paths, where the filename/tail is more useful than the
//!   root prefix.

pub mod deleting;
pub mod done;
pub mod list;
pub mod scanning;
pub mod welcome;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, Screen};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Braille spinner frames, cycled by the animation ticker.
///
/// There are 10 frames. The event loop advances the ticker by 1 each iteration
/// (roughly every 80 ms), and the UI divides by 2 before indexing, so each
/// frame is held for ~160 ms — fast enough to look smooth without being
/// distracting.
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// ─── Dispatch ─────────────────────────────────────────────────────────────────

/// Draws the current screen into `f`.
///
/// This is called by the event loop on every tick. It reads `app.screen` to
/// decide which renderer to invoke and then delegates entirely — no drawing
/// happens here directly.
///
/// `app` is passed as `&mut` because Ratatui's list widget requires a mutable
/// reference to [`ratatui::widgets::ListState`] when rendering a stateful list.
pub fn ui(f: &mut Frame, app: &mut App) {
    match &app.screen {
        Screen::Welcome => welcome::render_welcome(f, app),
        Screen::Scanning => scanning::render_scanning(f, app),
        // Confirming is an overlay on top of the list, so both variants go to
        // the same renderer and it checks `app.screen` internally to decide
        // whether to draw the popup.
        Screen::List | Screen::Confirming => list::render_list(f, app),
        Screen::Deleting => deleting::render_deleting(f, app),
        Screen::Done => done::render_done(f, app),
    }
}

// ─── Shared widgets ───────────────────────────────────────────────────────────

/// Builds the help bar shown at the bottom of every screen.
///
/// `pairs` is a slice of `(key, description)` tuples, for example:
/// `&[("↑↓", "Navigate"), ("Space", "Toggle"), ("Q", "Quit")]`.
///
/// Each key is rendered with a dark background to make it look like a
/// keyboard key, and the description is rendered in a dimmer colour next to
/// it. Pairs are separated by a few spaces so they don't run together.
///
/// The widget includes a top border line to visually separate the hint row
/// from the content above it.
pub fn help_bar<'a>(pairs: &[(&'a str, &'a str)]) -> Paragraph<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();

    for (i, (key, desc)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(
            format!(" {key} "),
            Style::default()
                .add_modifier(Modifier::REVERSED)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {desc}"),
            Style::default().fg(Color::Reset),
        ));
    }

    Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::Reset)),
    )
}

// ─── Layout helpers ───────────────────────────────────────────────────────────

/// Returns the area inside a one-cell border.
///
/// When a [`Block`] with [`Borders::ALL`] is drawn over a [`Rect`], the
/// content area is inset by one cell on every side. Ratatui doesn't compute
/// this automatically for manual layouts, so this helper does the maths:
/// add 1 to `x` and `y`, subtract 2 from `width` and `height`.
///
/// Using this consistently means content never visually overlaps the box
/// outline, regardless of terminal size.
pub fn inner_area(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Returns a horizontally centred rectangle of proportional width and fixed height.
///
/// `percent_x` is the percentage of `area.width` the rectangle should occupy
/// (e.g. `60` for 60%). `height` is a fixed row count. The rectangle is
/// centred both horizontally and vertically within `area`.
///
/// Used by the confirmation popup in [`list`] to place the dialog in the
/// middle of the screen regardless of terminal size.
pub fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let w = area.width * percent_x / 100;
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: w,
        height: height.min(area.height),
    }
}

/// Truncates `s` to at most `max_width` characters, eliding from the **left**.
///
/// If the string fits within `max_width` it is returned unchanged. If it is
/// too long, the leftmost characters are replaced with a single `…` so that
/// the string is exactly `max_width` characters wide.
///
/// Left-truncation is the right choice for file paths: the end of a path
/// (the directory name) is almost always more meaningful than the root prefix.
/// For example, a path like `/home/alice/projects/my-app/node_modules` becomes
/// `…/projects/my-app/node_modules` rather than `/home/alice/projects/my-…`.
pub fn truncate_left(s: &str, max_width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_width {
        return s.to_string();
    }
    let start = chars.len() - max_width + 1;
    format!("…{}", chars[start..].iter().collect::<String>())
}
