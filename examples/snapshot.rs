//! Render the CPU box once and print it as plain text.
//!
//! Useful for checking layout without a terminal — `make snapshot`.

use std::thread::sleep;
use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use ratatop::app::App;
use ratatop::ui;

fn main() {
    // Sample faster than the real interval so the graphs have some history to
    // show; pass a count to change how much.
    let ticks: usize = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(60);

    let mut app = App::new();
    for _ in 0..ticks {
        sleep(Duration::from_millis(30));
        app.tick();
    }

    let (width, height) = ui::preferred_size(&app);
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    ui::render(&app, area, &mut buf);
    ui::render_battery_meter(&app, area, &mut buf);

    for row in buf.content().chunks(area.width as usize) {
        let line: String = row.iter().map(ratatui::buffer::Cell::symbol).collect();
        println!("{}", line.trim_end());
    }
}
