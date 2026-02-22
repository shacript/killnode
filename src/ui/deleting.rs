//! # Deleting screen
//!
//! Shown while the background delete thread is working through the list of
//! selected directories. The user cannot interact with this screen — input is
//! intentionally blocked until deletion finishes so nothing can interrupt a
//! `remove_dir_all` call that is already in progress.
//!
//! ## Layout
//!
//! ```text
//! ┌─ 💥 killnode  ⠹  Deleting… ──────────────────────────────────┐
//! │                                                               │
//! │  ┌─ Progress ────────────────────────────────────────────┐   │
//! │  │ ████████████████████░░░░░░░░░░░░  7 / 12             │   │
//! │  └───────────────────────────────────────────────────────┘   │
//! │                                                               │
//! │  Removing:                                                    │
//! │  …/old-project/node_modules                                   │
//! │                                                               │
//! └───────────────────────────────────────────────────────────────┘
//! ```
//!
//! There is no help bar on this screen because no keys are active. The spinner
//! in the title and the advancing progress gauge are the only live elements.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
};

use super::{SPINNER, inner_area, truncate_left};
use crate::app::App;

/// Renders the deleting screen into `f`.
///
/// The layout has six vertical regions:
///
/// 1. **Top spacer** — a single blank row so the progress gauge doesn't sit
///    flush against the outer border, giving it room to breathe.
///
/// 2. **Progress gauge** — a bordered bar that fills from left to right as
///    directories are removed. The label inside the bar shows the raw count
///    (`done / total`) so the user can see the exact progress even when the
///    bar is nearly full and the visual fill is hard to judge precisely.
///
/// 3. **Middle spacer** — a blank row separating the gauge from the path area.
///
/// 4. **"Removing:" label** — a static dim label so the path below it has
///    context. Kept on its own line so the path has the full terminal width.
///
/// 5. **Current path** — the directory being removed right now, left-truncated
///    via [`truncate_left`] so the meaningful part (the end of the path) stays
///    visible even on narrow terminals.
///
/// 6. **Bottom spacer** — fills remaining vertical space so the content block
///    sits near the top rather than being stretched to fill the whole screen.
pub fn render_deleting(f: &mut Frame, app: &App) {
    let area = f.area();

    // Advance the spinner the same way the scanning screen does: divide the
    // raw ticker by 2 so each frame is held for ~160 ms instead of ~80 ms.
    let spinner = SPINNER[(app.ticker as usize / 2) % SPINNER.len()];

    // ── Outer border ──────────────────────────────────────────────────────────
    //
    // Red title to signal that a destructive operation is in progress.
    // The spinner makes it immediately obvious that the app is busy.
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Reset))
        .title(Span::styled(
            format!(" 💥 killnode  {}  Deleting… ", spinner),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(outer, area);

    // Work inside the border so content doesn't overlap the box outline.
    let inner = inner_area(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top spacer — breathing room above the gauge
            Constraint::Length(3), // progress gauge (1 bar row + top/bottom border)
            Constraint::Length(1), // middle spacer
            Constraint::Length(1), // "Removing:" label
            Constraint::Length(1), // currently-deleting path
            Constraint::Min(0),    // bottom spacer — expands to fill remaining height
        ])
        .split(inner);

    // ── Progress gauge ────────────────────────────────────────────────────────
    //
    // `ratio` is clamped to [0.0, 1.0] by dividing done by total. We use
    // `max(1)` on the total to avoid a divide-by-zero if `delete_total` is
    // somehow 0 (which the UI prevents, but is handled defensively here).
    let total = app.delete_total.max(1);
    let ratio = app.delete_done as f64 / total as f64;
    let label = format!("{} / {}", app.delete_done, app.delete_total);

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Reset))
                .title(Span::styled(
                    " Progress ",
                    Style::default()
                        .fg(Color::Reset)
                        .add_modifier(Modifier::DIM),
                )),
        )
        // Red fill on a dark-gray track so the remaining work is visible
        // even when only a small fraction has been completed.
        .gauge_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .ratio(ratio)
        .label(label);

    f.render_widget(gauge, chunks[1]);

    // ── "Removing:" label ─────────────────────────────────────────────────────
    //
    // A plain dim label so the path below it doesn't look like a random
    // string floating on screen without context.
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Removing:",
            Style::default()
                .fg(Color::Reset)
                .add_modifier(Modifier::DIM),
        ))),
        chunks[3],
    );

    // ── Current path ──────────────────────────────────────────────────────────
    //
    // The path is left-truncated so the directory name at the end is always
    // visible. Subtract 4 from the inner width to leave room for the "  "
    // indent prefix and a small safety margin.
    //
    // Note: this shows the path that was sent with the most recent
    // `DeleteMsg::Progress` message, which is dispatched *before* the
    // deletion begins. So this always reflects what is currently being removed,
    // not what was just finished.
    let max_width = inner.width.saturating_sub(4) as usize;
    let display_path = truncate_left(&app.delete_current, max_width);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {display_path}"),
            Style::default()
                .fg(Color::Reset)
                .add_modifier(Modifier::DIM),
        ))),
        chunks[4],
    );
}
