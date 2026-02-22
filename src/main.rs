//! # Entry point
//!
//! This is where the program starts. It does three things in order:
//!
//! 1. **Handle CLI flags** (`--help`, `--version`) without touching the terminal,
//!    so they work correctly when piped or redirected.
//!
//! 2. **Set up the terminal** for full-screen TUI mode — raw input, alternate
//!    screen buffer, hidden cursor.
//!
//! 3. **Run the event loop** ([`run`]), then unconditionally restore the terminal
//!    to its original state when the loop exits (whether normally or via panic).
//!
//! ## App lifecycle at a glance
//!
//! ```text
//! main()
//!  └─ run(terminal)
//!      └─ loop:
//!          ├─ drain background thread messages  (scan / delete progress)
//!          ├─ draw the current screen
//!          ├─ wait up to 80 ms for a keypress
//!          └─ dispatch key → App method → possibly change App::screen
//! ```
//!
//! The 80 ms poll timeout keeps the spinner animation smooth even when the
//! user isn't pressing anything.

mod app;
mod deleter;
mod scanner;
mod ui;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const HELP: &str = "\
nodenuke — find and delete node_modules directories

USAGE:
    nodenuke [OPTIONS] [DIRECTORY]

ARGS:
    [DIRECTORY]    Directory to scan (defaults to current directory)

OPTIONS:
    -h, --help       Print this help message
    -V, --version    Print version information
";

use std::{
    io::{self, Stdout},
    time::Duration,
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::{App, Screen};

/// The main event loop.
///
/// Each iteration of the loop does four things:
///
/// 1. **Drain messages** from background threads so new scan results and
///    deletion progress appear on the next frame — even without a keypress.
///
/// 2. **Draw** the current screen via [`ui::ui`].
///
/// 3. **Tick** the animation counter so spinners and other time-based
///    visuals advance at a steady pace.
///
/// 4. **Poll for input** for up to 80 ms. If a key arrives, dispatch it
///    to the appropriate handler for the active [`Screen`]. If no key
///    arrives within the timeout, loop again (this keeps the UI alive
///    during scanning/deleting even if the user is idle).
///
/// Returns `Ok(())` when the user quits, or bubbles up any I/O error.
fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    let mut app = App::new();

    loop {
        app.process_scan_messages();
        app.process_delete_messages();

        terminal.draw(|f| ui::ui(f, &mut app))?;

        app.ticker = app.ticker.wrapping_add(1);

        if !event::poll(Duration::from_millis(80))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        // Ignore key-release and key-repeat events — only act on key-down.
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match &app.screen {
            Screen::Welcome => match key.code {
                KeyCode::Enter | KeyCode::Char(' ') => app.begin_scan(),
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                _ => {}
            },

            // Scanning is fully automatic — the only thing the user can do is bail out.
            Screen::Scanning => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                _ => {}
            },

            Screen::List => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Up | KeyCode::Char('k') => app.navigate_up(),
                KeyCode::Down | KeyCode::Char('j') => app.navigate_down(),
                KeyCode::Char(' ') => app.toggle_selected(),
                KeyCode::Char('a') => app.toggle_all(),
                KeyCode::Char('A') => app.toggle_all_force(),
                KeyCode::Enter => {
                    if app.selected_count() > 0 {
                        app.screen = Screen::Confirming;
                    }
                }
                _ => {}
            },

            // Confirming is rendered as an overlay on top of the list screen.
            // Y/Enter proceeds; N/Esc drops back to the list.
            Screen::Confirming => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => app.begin_delete(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    app.screen = Screen::List;
                }
                _ => {}
            },

            // Input is intentionally blocked during deletion — wait for the
            // background thread to finish so nothing can interrupt a delete in progress.
            Screen::Deleting => {}

            Screen::Done => match key.code {
                KeyCode::Char('q') | KeyCode::Enter | KeyCode::Esc => return Ok(()),
                _ => {}
            },
        }
    }
}

/// Sets up the terminal, runs the app, and restores the terminal on exit.
///
/// Terminal setup and teardown are kept here (rather than scattered across the
/// codebase) so it's easy to reason about what state the terminal is in at any
/// point. The sequence is:
///
/// 1. Register a **panic hook** that restores the terminal before printing the
///    panic message — otherwise a crash leaves the user's shell in raw mode.
///
/// 2. Switch the terminal to **raw mode** (keypresses are delivered immediately,
///    without waiting for Enter, and without echo) and push the **alternate
///    screen buffer** (so the TUI doesn't overwrite the user's scrollback).
///
/// 3. Run the event loop.
///
/// 4. **Always** pop the alternate screen and restore cooked mode — even if the
///    loop returned an error.
fn main() -> io::Result<()> {
    // Handle informational flags before touching the terminal.
    // These are intentionally checked before any terminal setup so that
    // `nodenuke --help | cat` works as expected.
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("nodenuke {VERSION}");
                return Ok(());
            }
            _ => {}
        }
    }

    // If the app panics, restore the terminal before letting Rust print the
    // panic message. Without this, a crash would leave the shell in raw mode
    // with no visible cursor, which is very confusing.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    // Enter full-screen TUI mode.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    // Run the app. We capture the result so we can restore the terminal before
    // returning it — otherwise an early `?` would skip cleanup.
    let result = run(&mut terminal);

    // Restore the terminal unconditionally. If any of these fail there is
    // nothing sensible to do, so the errors are intentionally ignored.
    let _ = terminal.show_cursor();
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    result
}
