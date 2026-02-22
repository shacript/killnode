//! # Welcome screen
//!
//! The first thing the user sees when killnode starts. It is intentionally
//! minimal — just the scan root path and two key hints. The goal is to give
//! the user a chance to confirm they are about to scan the right directory
//! before any filesystem work begins.
//!
//! ## Layout
//!
//! ```text
//! ┌─ 💥 killnode ──────────────────────────────────────────────┐
//! │ ┌─ Scan root ───────────────────────────────────────────┐  │
//! │ │  /home/alice/projects                                 │  │
//! │ └───────────────────────────────────────────────────────┘  │
//! │                                                             │
//! │                        (spacer)                             │
//! │                                                             │
//! ├─────────────────────────────────────────────────────────────┤
//! │  [Enter] Start scan    [Q] Quit                             │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use super::{help_bar, inner_area};
use crate::app::App;

/// Renders the welcome screen into `f`.
///
/// The layout has three vertical regions:
///
/// 1. **Scan root box** — a small bordered widget showing the directory that
///    will be scanned. Displayed in cyan so it stands out as the one piece of
///    information the user should verify before pressing Enter.
///
/// 2. **Spacer** — fills the remaining vertical space so the help bar is
///    pushed to the bottom rather than floating in the middle of the screen.
///
/// 3. **Help bar** — shows the two available actions: start the scan or quit.
pub fn render_welcome(f: &mut Frame, app: &App) {
    let area = f.area();

    // Outer border with the app title in the top-left corner.
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " 💥 killnode ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(outer, area);

    // Work inside the border so content doesn't overlap the box outline.
    let inner = inner_area(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // scan root box (1 line of text + top/bottom border)
            Constraint::Min(0),    // spacer — expands to fill available height
            Constraint::Length(3), // help bar (1 line of hints + top border + padding)
        ])
        .split(inner);

    // ── Scan root ─────────────────────────────────────────────────────────────
    //
    // Shows the directory that will be scanned when the user presses Enter.
    // Rendered with a border and a "Scan root" title so it reads like a
    // labelled field rather than just a raw string floating on screen.
    let root = Paragraph::new(Line::from(Span::styled(
        format!("  {}", app.scan_root),
        Style::default().fg(Color::Cyan),
    )))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                " Scan root ",
                Style::default().fg(Color::DarkGray),
            )),
    );
    f.render_widget(root, chunks[0]);

    // ── Help bar ──────────────────────────────────────────────────────────────
    f.render_widget(
        help_bar(&[("Enter", "Start scan"), ("Q", "Quit")]),
        chunks[2],
    );
}
