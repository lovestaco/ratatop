//! Render the boxes once and print them as plain text.
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

    let (cpu_width, cpu_height) = ui::cpu::preferred_size(&app);
    let (mem_width, mem_height) = ui::mem::preferred_size();
    let (disk_width, disk_height) = ui::disk::preferred_size(app.disks.all().len());
    // Wide enough for the memory and disks boxes side by side under the CPU box.
    let (net_width, net_height) = ui::net::preferred_size();
    let width = cpu_width.max(mem_width + disk_width + net_width);
    let below = mem_height.max(disk_height).max(net_height);
    let area = Rect::new(0, 0, width, cpu_height + below);
    let mut buf = Buffer::empty(area);
    ui::render(&app, area, &mut buf);

    for row in buf.content().chunks(area.width as usize) {
        let line: String = row.iter().map(ratatui::buffer::Cell::symbol).collect();
        println!("{}", line.trim_end());
    }
}
