//! # List screen
//!
//! The main interactive screen. Shown after the scan completes, it presents
//! all discovered `node_modules` directories in a scrollable table so the
//! user can decide which ones to delete.
//!
//! This module also owns the confirmation popup ([`render_confirm_popup`]),
//! which is an overlay drawn on top of the list when the user presses Enter.
//! Because the popup shares the same underlying layout, both [`Screen::List`]
//! and [`Screen::Confirming`] are routed to [`render_list`] by the UI
//! dispatcher — the function checks `app.screen` to decide whether to draw
//! the popup.
//!
//! ## List layout
//!
//! ```text
//! ┌─ 💥 NodeNuke  ·  14 found  ·  2.3 GB total ─────────────────────────┐
//! │  SEL   PATH                                   MODIFIED         SIZE  │
//! │  [✓]   …/my-app/node_modules                   3d ago        450 MB  │
//! │  [✓]   …/old-project/node_modules              2mo ago       210 MB  │
//! │  [ ]   ⚠ …/.config/app/node_modules            1y ago         80 MB  │
//! │  [ ]   …/work/api/node_modules                just now       120 MB  │
//! │  ...                                                                  │
//! ├───────────────────────────────────────────────────────────────────────┤
//! │  [↑↓/jk] Navigate  [Space] Toggle  [a] All safe  [A] All + ⚠  [Q] Quit │
//! └───────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Columns
//!
//! | Column | Width | Notes |
//! |--------|-------|-------|
//! | SEL | 6 chars | `[✓]` green = selected safe, `[✓]` yellow = selected sensitive, `[ ]` = unselected |
//! | PATH | remaining | `⚠ ` prefix in red for sensitive entries; left-truncated so the tail is always visible |
//! | MODIFIED | 10 chars | human-friendly age: "just now", "3d ago", "2mo ago", etc. |
//! | SIZE | 10 chars | formatted with SI decimal units (KB, MB, GB) |
//!
//! ## Confirmation popup layout
//!
//! ```text
//! ┌─ Confirm Deletion ──────────────────────────┐
//! │  Delete  3 directories  freeing ~660 MB?     │
//! │  ⚠  Warning: sensitive paths are selected!  │  ← only shown when relevant
//! │                                              │
//! ├──────────────────────────────────────────────┤
//! │  [Y / Enter] Confirm      [N / Esc] Cancel   │
//! └──────────────────────────────────────────────┘
//! ```
//!
//! The warning line appears whenever one or more sensitive entries are among
//! the selected directories, giving the user a final chance to reconsider
//! before an irreversible deletion of a system-adjacent path.

use humansize::{DECIMAL, format_size};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, ListItem, Paragraph},
};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{centered_rect, help_bar, inner_area, truncate_left};
use crate::app::{App, Screen};

/// Renders the list screen (and optionally the confirmation popup) into `f`.
///
/// The layout has three vertical regions:
///
/// 1. **Column header** — a single row of right/left-aligned labels that line
///    up with the data columns below them.
///
/// 2. **Scrollable list** — one row per discovered `node_modules` directory.
///    Ratatui's stateful `List` widget handles scrolling; the scroll position
///    is stored in `app.list_state`.
///    Every entry is selectable, including sensitive ones. Sensitive entries
///    are distinguished by a red `⚠ ` prefix in the PATH column rather than
///    a locked checkbox state. Selecting a sensitive entry turns its checkbox
///    yellow as a visual reminder that something unusual is queued.
///
/// 3. **Help bar** — shows context-sensitive hints. When at least one entry
///    is selected the Enter hint changes to show the count and size that would
///    be deleted, so the user knows the consequences before confirming.
///    Two distinct "select all" actions are always shown: `a` for safe entries
///    only, and `A` for everything including sensitive ones.
///
/// If `app.screen` is [`Screen::Confirming`], [`render_confirm_popup`] is
/// called after the list to draw the dialog on top of it.
pub fn render_list(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let is_confirming = matches!(app.screen, Screen::Confirming);

    let count = app.entries.len();
    let total_size_str = format_size(app.total_size(), DECIMAL);

    // ── Outer border ──────────────────────────────────────────────────────────
    //
    // The title summarises the scan results at a glance: how many directories
    // were found and how much space they occupy in total.
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            format!(" 💥 NodeNuke  ·  {count} found  ·  {total_size_str} total "),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(outer, area);

    // Work inside the border so content doesn't overlap the box outline.
    let inner = inner_area(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // column header row
            Constraint::Min(0),    // scrollable list — expands to fill available height
            Constraint::Length(3), // help bar (1 line of hints + top border + padding)
        ])
        .split(inner);

    // ── Column header ─────────────────────────────────────────────────────────
    //
    // Column widths are computed dynamically from the terminal width so the
    // layout stays correct on any screen size. PATH gets whatever is left after
    // the fixed-width columns and their separators are accounted for.
    let header_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);
    let list_width = chunks[1].width.saturating_sub(2) as usize;
    let size_col_w: usize = 10;
    let modified_col_w: usize = 10;
    let checkbox_col_w: usize = 6;
    let path_col_w = list_width.saturating_sub(size_col_w + modified_col_w + checkbox_col_w + 3);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{:<checkbox_col_w$}", " SEL"), header_style),
            Span::styled(format!("{:<path_col_w$}", " PATH"), header_style),
            Span::raw(" "),
            Span::styled(format!("{:>modified_col_w$}", "MODIFIED"), header_style),
            Span::raw(" "),
            Span::styled(format!("{:>size_col_w$}", "SIZE"), header_style),
        ])),
        chunks[0],
    );

    // ── List items ────────────────────────────────────────────────────────────
    //
    // Each entry is rendered as a single-line `ListItem`. The checkbox column
    // shows selection state for every entry — including sensitive ones, which
    // can now be manually selected:
    //
    //   [✓]  green   — selected safe entry, will be deleted
    //   [✓]  yellow  — selected sensitive entry, will be deleted (with warning)
    //   [ ]  gray    — not selected, will be kept
    //
    // Sensitive entries have a ⚠ prefix rendered in red directly before the
    // path text, so the PATH column is self-labelling without needing a
    // separate checkbox state.
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let items: Vec<ListItem> = app
        .entries
        .iter()
        .map(|entry| {
            // Checkbox: reflects selection state for all entries.
            // Sensitive + selected uses yellow instead of green as a visual
            // reminder that something unusual is about to be deleted.
            let (checkbox, checkbox_style) = if entry.selected {
                if entry.sensitive {
                    (
                        "[✓]",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    (
                        "[✓]",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                }
            } else {
                ("[ ]", Style::default().fg(Color::DarkGray))
            };

            // Path text is always white and readable — sensitive entries are
            // no longer dimmed since they are now fully selectable.
            let path_style = Style::default().fg(Color::White);

            // Sensitive entries reserve 2 characters at the start of the path
            // column for the "⚠ " prefix. The actual path is truncated to the
            // remaining width so the MODIFIED and SIZE columns still line up.
            let (warn_prefix, path_available) = if entry.sensitive {
                ("⚠ ", path_col_w.saturating_sub(2))
            } else {
                ("", path_col_w)
            };

            let path_trunc = truncate_left(&entry.path, path_available);
            // Pad to the full column width so alignment is preserved regardless
            // of whether the ⚠ prefix is present.
            let path_padded = format!("{:<path_col_w$}", format!("{warn_prefix}{path_trunc}"));
            let size_str = format_size(entry.size, DECIMAL);
            let modified_str = entry
                .last_modified
                .map(|ts| format_age(now_secs.saturating_sub(ts)))
                .unwrap_or_else(|| "?".to_string());

            // Build the path cell as two spans when sensitive so the ⚠ prefix
            // can be coloured red while the path text stays white.
            let path_spans: Vec<Span> = if entry.sensitive {
                vec![
                    Span::styled("⚠ ", Style::default().fg(Color::Red)),
                    Span::styled(format!("{:<path_available$}", path_trunc), path_style),
                ]
            } else {
                vec![Span::styled(path_padded, path_style)]
            };

            let mut spans = vec![Span::styled(format!(" {checkbox} "), checkbox_style)];
            spans.extend(path_spans);
            spans.extend([
                Span::raw(" "),
                Span::styled(
                    format!("{:>modified_col_w$}", modified_str),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:>size_col_w$}", size_str),
                    Style::default().fg(Color::Cyan),
                ),
            ]);

            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = ratatui::widgets::List::new(items).highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    f.render_stateful_widget(list, chunks[1], &mut app.list_state);

    // ── Help bar ──────────────────────────────────────────────────────────────
    //
    // The Enter hint is context-sensitive: when at least one entry is selected
    // it shows the count and combined size so the user knows exactly what will
    // happen before they commit. When nothing is selected, Enter isn't offered
    // at all because there is nothing to delete.
    let selected_count = app.selected_count();
    let selected_size_str = format_size(app.selected_size(), DECIMAL);

    if selected_count > 0 {
        let delete_label = format!("Delete {selected_count} ({selected_size_str})");
        f.render_widget(
            help_bar(&[
                ("↑↓ / jk", "Navigate"),
                ("Space", "Toggle"),
                ("a", "All safe"),
                ("A", "All + ⚠"),
                ("Enter", &delete_label),
                ("Q", "Quit"),
            ]),
            chunks[2],
        );
    } else {
        f.render_widget(
            help_bar(&[
                ("↑↓ / jk", "Navigate"),
                ("Space", "Toggle"),
                ("a", "All safe"),
                ("A", "All + ⚠"),
                ("Q", "Quit"),
            ]),
            chunks[2],
        );
    }

    // ── Confirmation popup (overlay) ──────────────────────────────────────────
    //
    // Drawn last so it appears on top of the list. Only rendered when the
    // screen is in the Confirming state.
    if is_confirming {
        render_confirm_popup(f, app, area);
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Formats a duration (given as seconds) as a human-friendly "time ago" string.
///
/// The output uses the largest unit that gives a whole number, rounded down:
///
/// | Range | Example output |
/// |-------|----------------|
/// | < 60 s | `"just now"` |
/// | < 1 h | `"42m ago"` |
/// | < 1 d | `"3h ago"` |
/// | < 1 w | `"2d ago"` |
/// | < 30 d | `"1w ago"` |
/// | < 1 y | `"3mo ago"` |
/// | ≥ 1 y | `"2y ago"` |
///
/// Months are approximated as 30 days and years as 365 days.
fn format_age(secs: u64) -> String {
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 7 * 86_400 {
        format!("{}d ago", secs / 86_400)
    } else if secs < 30 * 86_400 {
        format!("{}w ago", secs / (7 * 86_400))
    } else if secs < 365 * 86_400 {
        format!("{}mo ago", secs / (30 * 86_400))
    } else {
        format!("{}y ago", secs / (365 * 86_400))
    }
}

/// Renders the confirmation dialog as an overlay on top of the list.
///
/// The dialog is centred on the screen and sized at 60% of the terminal width
/// with a fixed height of 9 rows — enough to show the summary, an optional
/// warning, and the help bar without feeling cramped.
///
/// A [`Clear`] widget is rendered first to erase the list content behind the
/// popup area, preventing the text underneath from bleeding through.
///
/// The dialog has three regions:
///
/// 1. **Summary line** — states exactly what will happen: how many directories
///    will be deleted and how much space will be freed. The directory count is
///    highlighted in red and the size in green to draw the eye to the key facts.
///
/// 2. **Sensitive warning** (conditional) — if the user somehow has sensitive
///    entries selected (which the UI normally prevents, but is checked here as
///    a safety net), a red warning is shown.
///
/// 3. **Help bar** — Y/Enter to confirm, N/Esc to cancel and go back to the list.
fn render_confirm_popup(f: &mut Frame, app: &App, area: Rect) {
    let popup_area = centered_rect(60, 9, area);

    // Erase whatever the list drew in this region so the popup has a clean
    // background rather than showing through to the rows behind it.
    f.render_widget(Clear, popup_area);

    let selected_count = app.selected_count();
    let selected_size_str = format_size(app.selected_size(), DECIMAL);
    let has_sensitive_selected = app.entries.iter().any(|e| e.sensitive && e.selected);

    // ── Popup border ──────────────────────────────────────────────────────────
    //
    // Yellow border to signal "attention required" without being as alarming
    // as red (which is used for destructive actions that have already been
    // confirmed).
    let popup_block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .title(Span::styled(
            " Confirm Deletion ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    f.render_widget(popup_block, popup_area);

    let inner = inner_area(popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // summary line
            Constraint::Length(2), // optional sensitive-path warning (empty if not needed)
            Constraint::Min(0),    // spacer
            Constraint::Length(1), // help bar
        ])
        .split(inner);

    // ── Summary ───────────────────────────────────────────────────────────────
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  Delete  ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{selected_count} directories"),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  freeing ~", Style::default().fg(Color::Gray)),
            Span::styled(
                selected_size_str,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("?", Style::default().fg(Color::Gray)),
        ])),
        chunks[0],
    );

    // ── Sensitive warning ─────────────────────────────────────────────────────
    //
    // Only shown when a sensitive entry is somehow selected. In normal usage
    // the UI prevents selecting sensitive entries, but this acts as a last-
    // chance safety warning just in case.
    if has_sensitive_selected {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  ⚠  Warning: sensitive paths are selected!",
                Style::default().fg(Color::Red),
            ))),
            chunks[1],
        );
    }

    // ── Help bar ──────────────────────────────────────────────────────────────
    f.render_widget(
        help_bar(&[("Y / Enter", "Confirm"), ("N / Esc", "Cancel")]),
        chunks[3],
    );
}
