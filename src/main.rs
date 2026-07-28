//! Terminal setup and the event loop.

use std::io;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, KeyCode};
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
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                    KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        app.should_quit = true;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn draw(app: &App, frame: &mut Frame) {
    let area = cpu_box_area(app, frame.area());
    ui::render(app, area, frame.buffer_mut());
    ui::render_battery_meter(app, area, frame.buffer_mut());
    // Last, over the full frame: see `ui::render_background`.
    ui::render_background(frame.area(), frame.buffer_mut());
}

/// Size the box to its content and center it.
///
/// Only the CPU box exists so far; once the memory, network, and process boxes
/// land this becomes a real four-way layout.
fn cpu_box_area(app: &App, area: Rect) -> Rect {
    let (preferred_width, preferred_height) = ui::preferred_size(app);
    let height = preferred_height.min(area.height);
    let width = preferred_width.min(area.width);

    let [horizontal] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [centered] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(horizontal);
    centered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_takes_its_preferred_size_when_there_is_room() {
        let app = App::new();
        let area = cpu_box_area(&app, Rect::new(0, 0, 200, 60));
        assert_eq!((area.width, area.height), ui::preferred_size(&app));
    }

    #[test]
    fn box_never_exceeds_a_small_terminal() {
        let app = App::new();
        let frame = Rect::new(0, 0, 20, 6);
        let area = cpu_box_area(&app, frame);
        assert!(area.height <= frame.height && area.width <= frame.width);
    }
}
