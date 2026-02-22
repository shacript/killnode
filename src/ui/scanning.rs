//! # Scanning screen
//!
//! Shown while the background scanner thread is running. The screen serves two
//! purposes: it gives the user something to look at while they wait, and it
//! provides enough live feedback to confirm that the scan is actually making
//! progress (not frozen).
//!
//! ## Layout
//!
//! ```text
//! ┌─ 💥 killnode  ⠹  Scanning… ──────────────────────────────────┐
//! │                                                               │
//! │   Found  12  node_modules so far…                            │
//! │   Currently scanning:                                         │
//! │   …/alice/projects/some-deep/nested/path                      │
//! │                                                               │
//! │                        (spacer)                               │
//! │                                                               │
//! ├───────────────────────────────────────────────────────────────┤
//! │  [Q] Quit                                                     │
//! └───────────────────────────────────────────────────────────────┘
//! ```
//!
//! The spinner in the title bar and the "currently scanning" path are the two
//! live elements — everything else is static until the scan completes.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::{SPINNER, help_bar, inner_area, truncate_left};
use crate::app::App;

/// Renders the scanning screen into `f`.
///
/// The layout has five vertical regions:
///
/// 1. **Found count** — shows how many `node_modules` directories have been
///    discovered so far. Updates on every frame as new results stream in from
///    the background thread.
///
/// 2. **"Currently scanning:" label** — a static dim label above the live path
///    so the path below it doesn't look like an unlabelled mystery string.
///
/// 3. **Live path** — the directory the scanner is visiting right now. Because
///    deep paths can easily exceed the terminal width, this is left-truncated
///    via [`truncate_left`] so the most meaningful part (the end of the path)
///    always stays visible.
///
/// 4. **Spacer** — fills remaining vertical space so the help bar stays at the
///    bottom of the screen.
///
/// 5. **Help bar** — only one action is available during a scan: quit.
pub fn render_scanning(f: &mut Frame, app: &App) {
    let area = f.area();

    // Advance the spinner by dividing the ticker by 2 so each frame is held
    // for ~160 ms rather than the raw ~80 ms tick rate. Fast enough to look
    // animated, slow enough not to be distracting.
    let spinner = SPINNER[(app.ticker as usize / 2) % SPINNER.len()];
    let count = app.entries.len();

    // Outer border. The title turns yellow and shows a spinner while scanning,
    // making it visually distinct from the red title on the Welcome screen.
    let outer = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::default().fg(Color::Reset))
        .title(Span::styled(
            format!(" 💥 killnode  {}  Scanning… ", spinner),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    f.render_widget(outer, area);

    // Work inside the border so content doesn't overlap the box outline.
    let inner = inner_area(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // found count (1 line + breathing room)
            Constraint::Length(1), // "Currently scanning:" label
            Constraint::Length(1), // live path
            Constraint::Min(0),    // spacer — expands to fill available height
            Constraint::Length(3), // help bar (1 line of hints + top border + padding)
        ])
        .split(inner);

    // ── Found count ───────────────────────────────────────────────────────────
    //
    // The count is highlighted in green so it pops out immediately. The
    // surrounding text is dimmed so the number is the clear focal point.
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  Found  ",
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::DIM),
            ),
            Span::styled(
                count.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  node_modules so far…",
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::DIM),
            ),
        ])),
        chunks[0],
    );

    // ── "Currently scanning:" label ───────────────────────────────────────────
    //
    // A plain dim label so the live path below it has context. Kept on its own
    // line rather than inline with the path so the path has the full width of
    // the terminal available.
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Currently scanning:",
            Style::default()
                .fg(Color::Reset)
                .add_modifier(Modifier::DIM),
        ))),
        chunks[1],
    );

    // ── Live path ─────────────────────────────────────────────────────────────
    //
    // The path is left-truncated so the tail (the directory the scanner is
    // actually inside right now) is always visible even on narrow terminals.
    // Subtract 4 from the inner width to leave room for the "  " indent prefix
    // and a small safety margin so the ellipsis never gets clipped.
    let max_width = inner.width.saturating_sub(4) as usize;
    let scanning_path = app.current_scanning_path();
    let display_path = truncate_left(&scanning_path, max_width);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {display_path}"),
            Style::default()
                .fg(Color::Reset)
                .add_modifier(Modifier::DIM),
        ))),
        chunks[2],
    );

    // ── Help bar ──────────────────────────────────────────────────────────────
    f.render_widget(help_bar(&[("Q", "Quit")]), chunks[4]);
}
