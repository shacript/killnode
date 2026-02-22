//! # Done screen
//!
//! The final screen. Shown when either:
//!
//! - The scan finished and the user deleted their selected directories, or
//! - The scan finished but nothing was found (or the user quit without
//!   deleting anything).
//!
//! The screen summarises what happened and, if any deletions failed, lists
//! each error so the user knows which directories were not cleaned up.
//!
//! ## Layout (normal completion)
//!
//! ```text
//! ┌─ 💥 killnode  ·  Complete ────────────────────────────────────┐
//! │                                                               │
//! │   Removed   12  directories                                   │
//! │   Freed     2.3 GB                                            │
//! │   Failed    1  (see errors below)                             │
//! │                                                               │
//! │  ┌─ Errors ────────────────────────────────────────────────┐  │
//! │  │  ✗  /some/path/node_modules: permission denied          │  │
//! │  └─────────────────────────────────────────────────────────┘  │
//! ├───────────────────────────────────────────────────────────────┤
//! │  [Q / Enter] Quit                                             │
//! └───────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Layout (nothing found / nothing deleted)
//!
//! ```text
//! ┌─ 💥 killnode  ·  Complete ────────────────────────────────────┐
//! │                                                               │
//! │   No node_modules found in the specified path.                │
//! │                                                               │
//! ├───────────────────────────────────────────────────────────────┤
//! │  [Q / Enter] Quit                                             │
//! └───────────────────────────────────────────────────────────────┘
//! ```

use humansize::{DECIMAL, format_size};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use super::{help_bar, inner_area};
use crate::app::App;

/// Renders the done screen into `f`.
///
/// The layout has three vertical regions:
///
/// 1. **Summary** — four lines that describe the outcome. The content varies
///    depending on whether anything was found and whether anything was deleted.
///    See [`build_summary`] for the three cases.
///
/// 2. **Error list** (conditional) — a bordered list of every path that could
///    not be deleted, along with the OS error message for each one. Only
///    rendered when `app.delete_errors` is non-empty; the region collapses to
///    nothing when there are no errors.
///
/// 3. **Help bar** — only one action is available here: quit.
pub fn render_done(f: &mut Frame, app: &App) {
    let area = f.area();

    // ── Outer border ──────────────────────────────────────────────────────────
    //
    // Green title to signal successful completion — a deliberate contrast to
    // the red used while the app is active, so it reads as "all done, relax".
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " 💥 killnode  ·  Complete ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    f.render_widget(outer, area);

    // Work inside the border so content doesn't overlap the box outline.
    let inner = inner_area(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // summary block (up to 4 lines of stats)
            Constraint::Min(0),    // error list — expands to fill height, or collapses if empty
            Constraint::Length(3), // help bar (1 line of hints + top border + padding)
        ])
        .split(inner);

    // ── Summary ───────────────────────────────────────────────────────────────
    //
    // The summary is built separately in `build_summary` to keep this function
    // readable. It returns a `Vec<Line>` so Paragraph can render it directly.
    let summary = build_summary(app);
    f.render_widget(Paragraph::new(summary), chunks[0]);

    // ── Error list ────────────────────────────────────────────────────────────
    //
    // Each error is prefixed with a red ✗ so failures stand out immediately
    // when the user's eye lands on this region. The list is only rendered when
    // there is something to show — otherwise the space is left empty and the
    // help bar shifts up naturally.
    if !app.delete_errors.is_empty() {
        let items: Vec<ListItem> = app
            .delete_errors
            .iter()
            .map(|e| {
                ListItem::new(Line::from(Span::styled(
                    format!("  ✗  {e}"),
                    Style::default().fg(Color::Red),
                )))
            })
            .collect();

        let error_list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(Span::styled(
                    " Errors ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
        );
        f.render_widget(error_list, chunks[1]);
    }

    // ── Help bar ──────────────────────────────────────────────────────────────
    f.render_widget(help_bar(&[("Q / Enter", "Quit")]), chunks[2]);
}

// ─── Summary builder ──────────────────────────────────────────────────────────

/// Builds the summary paragraph for the done screen.
///
/// There are three distinct cases, each producing different output:
///
/// ### 1. Nothing was found
///
/// The scan completed but found no `node_modules` directories at all under
/// the given root. A neutral yellow message tells the user so they know the
/// app ran successfully — it just had nothing to do.
///
/// ### 2. Scan finished but nothing was deleted
///
/// The user either quit from the list screen without selecting anything, or
/// pressed Enter but then cancelled at the confirmation prompt. A yellow
/// message acknowledges that no directories were removed.
///
/// ### 3. Normal completion
///
/// One or more directories were deleted. The summary shows three stats:
///
/// - **Removed** — the number of directories successfully deleted, in green.
/// - **Freed** — the total bytes reclaimed, in cyan (formatted as KB/MB/GB).
/// - **Failed** — the number of errors, in red. Only shown when > 0.
fn build_summary(app: &App) -> Vec<Line<'static>> {
    // Case 1: the scan found nothing at all.
    if app.entries.is_empty() {
        return vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No node_modules found in the specified path.",
                Style::default().fg(Color::Yellow),
            )),
        ];
    }

    // Case 2: scan found things but the user didn't delete any of them.
    if app.delete_total == 0 {
        return vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No directories were deleted.",
                Style::default().fg(Color::Yellow),
            )),
        ];
    }

    // Case 3: at least one deletion was attempted.
    let freed_str = format_size(app.delete_freed, DECIMAL);
    let success = app.delete_total - app.delete_errors.len();

    vec![
        Line::from(""),
        // "Removed  N  directories" — count in green, surrounding text dimmed.
        Line::from(vec![
            Span::styled("  Removed  ", Style::default().fg(Color::Gray)),
            Span::styled(
                success.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                // Proper singular/plural so "1 directory" reads correctly.
                format!("  director{}", if success == 1 { "y" } else { "ies" }),
                Style::default().fg(Color::Gray),
            ),
        ]),
        // "Freed    X.X GB" — size in cyan to match the SIZE column in the list.
        Line::from(vec![
            Span::styled("  Freed    ", Style::default().fg(Color::Gray)),
            Span::styled(
                freed_str,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        // "Failed   N  (see errors below)" — only included when there were errors.
        if app.delete_errors.is_empty() {
            Line::from("")
        } else {
            Line::from(vec![
                Span::styled("  Failed   ", Style::default().fg(Color::Gray)),
                Span::styled(
                    app.delete_errors.len().to_string(),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled("  (see errors below)", Style::default().fg(Color::DarkGray)),
            ])
        },
    ]
}
