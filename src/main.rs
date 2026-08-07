//! Terminal setup and the event loop.

use std::io;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{DefaultTerminal, Frame};

use ratatop::app::App;
use ratatop::ui;

/// How often to check for input. Well under the sample interval, so a keypress
/// is never held for the length of a tick.
const POLL: Duration = Duration::from_millis(100);

fn main() -> io::Result<()> {
    let terminal = ratatui::init();
    let result = run(terminal);
    ratatui::restore();
    result
}

fn run(mut terminal: DefaultTerminal) -> io::Result<()> {
    let mut app = App::new();
    app.tick();
    let mut last_tick = Instant::now();

    while !app.should_quit {
        terminal.draw(|frame| draw(&app, frame))?;

        // Sampling happens here rather than inside `draw` so a slow /proc sweep
        // cannot stall input handling mid-frame.
        if last_tick.elapsed() >= app.interval {
            app.tick();
            last_tick = Instant::now();
        }

        if event::poll(POLL)? {
            if let Some(key) = event::read()?.as_key_press_event() {
                handle_key(&mut app, key);
            }
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true;
        }
        // `=` and `_` are the unshifted keys, so the rate can be changed without
        // reaching for shift.
        //
        // These take effect on the next tick rather than resetting the timer, so
        // holding a key down cannot starve sampling.
        KeyCode::Char('+' | '=') => app.adjust_interval(true),
        KeyCode::Char('-' | '_') => app.adjust_interval(false),
        // btop cycles network interfaces with b and n.
        KeyCode::Char('n') => app.net.cycle(true),
        KeyCode::Char('b') => app.net.cycle(false),
        // Process table navigation.
        KeyCode::Up => app.processes.move_selection(-1),
        KeyCode::Down => app.processes.move_selection(1),
        KeyCode::PageUp => app.processes.move_selection(-10),
        KeyCode::PageDown => app.processes.move_selection(10),
        KeyCode::Home => app.processes.select_first(),
        KeyCode::End => app.processes.select_last(),
        KeyCode::Right => app.processes.cycle_sort(true),
        KeyCode::Left => app.processes.cycle_sort(false),
        KeyCode::Char('r') => app.processes.toggle_reverse(),
        _ => {}
    }
}

/// The boxes get the whole terminal; `ui::render` decides how they share it.
fn draw(app: &App, frame: &mut Frame) {
    ui::render(app, frame.area(), frame.buffer_mut());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(app: &mut App, code: KeyCode) {
        handle_key(app, KeyEvent::from(code));
    }

    #[test]
    fn quit_keys_stop_the_loop() {
        for code in [KeyCode::Char('q'), KeyCode::Esc] {
            let mut app = App::new();
            press(&mut app, code);
            assert!(app.should_quit, "{code:?} should quit");
        }

        let mut app = App::new();
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(app.should_quit, "ctrl-c should quit");

        // Bare `c` is not a quit key.
        let mut app = App::new();
        press(&mut app, KeyCode::Char('c'));
        assert!(!app.should_quit);
    }

    #[test]
    fn rate_keys_adjust_the_interval_in_both_directions() {
        let mut app = App::new();
        let default = app.interval;

        press(&mut app, KeyCode::Char('-'));
        assert!(app.interval > default, "- slows sampling down");

        let slowed = app.interval;
        press(&mut app, KeyCode::Char('+'));
        assert!(app.interval < slowed, "+ speeds it up");

        // The unshifted keys do the same thing.
        let mut app = App::new();
        press(&mut app, KeyCode::Char('='));
        assert!(app.interval < default, "= behaves like +");
        let mut app = App::new();
        press(&mut app, KeyCode::Char('_'));
        assert!(app.interval > default, "_ behaves like -");
    }

    #[test]
    fn unmapped_keys_are_ignored() {
        let mut app = App::new();
        let before = app.interval;
        press(&mut app, KeyCode::Char('z'));
        press(&mut app, KeyCode::F(5));
        assert!(!app.should_quit);
        assert_eq!(app.interval, before);
    }
}
