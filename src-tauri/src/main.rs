#![allow(dead_code, unused_imports)]

//! Terminal UI entry point.
//!
//! Replaces the Tauri webview control UI with a ratatui terminal interface.
//! The glium OpenGL output windows are still created on background threads
//! for media playback on the HDMI display.

use std::io;
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

mod app;
mod audio;
mod cue;
mod display;
mod logging;
mod media;
mod playlist;
mod renderer;
mod scheduler;

use app::App;
use logging::init;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init();
    logging::info("starting dui (terminal UI)");

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = result {
        logging::error(&format!("application error: {err}"));
        eprintln!("{err}");
        std::process::exit(1);
    }

    logging::info("application exited normally");
    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let app = App::new()?;
    let mut app = app;

    loop {
        terminal.draw(|f| app.ui(f))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('c')
                            if key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL) =>
                        {
                            return Ok(());
                        }
                        _ => app.handle_key(key),
                    }
                }
            }
        }

        app.tick()?;
    }
}
