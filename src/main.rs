//! Terminal setup and the event loop.

use std::io;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
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
        _ => {}
    }
}

fn draw(app: &App, frame: &mut Frame) {
    let area = cpu_box_area(app, frame.area());
    ui::render(app, area, frame.buffer_mut());
    ui::render_battery_meter(app, area, frame.buffer_mut());
    // Last, over the full frame: see `ui::render_background`.
    ui::render_background(frame.area(), frame.buffer_mut());
}

/// Span the terminal's width, take only the height the content needs, and sit at
/// the top.
///
/// btop's CPU box spans the screen, and a wider box means a longer history in
/// the graphs — so width is worth taking, height is not. The leftover height
/// goes below the box, where the memory, network, and process boxes will land.
fn cpu_box_area(app: &App, area: Rect) -> Rect {
    let (preferred_width, preferred_height) = ui::preferred_size(app);
    let height = preferred_height.min(area.height);
    let width = area.width.max(preferred_width).min(area.width);

    let [horizontal] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [top] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Start)
        .areas(horizontal);
    top
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

    #[test]
    fn box_spans_the_width_but_only_the_height_it_needs() {
        let app = App::new();
        let frame = Rect::new(0, 0, 200, 60);
        let area = cpu_box_area(&app, frame);
        assert_eq!(area.width, frame.width, "width is worth taking");
        assert_eq!(area.height, ui::preferred_size(&app).1, "height is not");
    }

    #[test]
    fn box_sits_at_the_top_of_the_terminal() {
        let app = App::new();
        for height in [10, 30, 60] {
            let frame = Rect::new(0, 0, 120, height);
            assert_eq!(cpu_box_area(&app, frame).y, 0, "no gap above the box");
        }
    }

    #[test]
    fn box_never_exceeds_a_small_terminal() {
        let app = App::new();
        let frame = Rect::new(0, 0, 20, 6);
        let area = cpu_box_area(&app, frame);
        assert!(area.height <= frame.height && area.width <= frame.width);
    }
}
