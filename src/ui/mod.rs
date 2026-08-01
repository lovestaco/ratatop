//! The boxes, and how they share the terminal.

pub mod cpu;
pub mod disk;
pub mod mem;
pub mod net;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;

use crate::app::App;
use crate::theme;

/// Paint the whole terminal, not just the boxes.
///
/// btop owns the full screen rather than inheriting the terminal's colors.
///
/// Call this *after* the widgets, not before: it sets `bg` only, so the colors
/// they chose survive, and anything they blanked gets picked up. Running it
/// first leaves holes: laying down a double-width glyph like `⚡` resets the
/// continuation cell behind it, which would otherwise keep the terminal's own
/// background.
pub fn render_background(area: Rect, buf: &mut Buffer) {
    buf.set_style(area, Style::default().bg(theme::MAIN_BG));
}

/// Draw every enabled box.
///
/// The CPU box spans the top; memory sits below it on the left, leaving the
/// right free for the network and process boxes when they land.
pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let (_, cpu_height) = cpu::preferred_size(app);
    let (mem_width, mem_height) = mem::preferred_size();

    let [cpu_area, rest] = Layout::vertical([
        Constraint::Length(cpu_height.min(area.height)),
        Constraint::Min(0),
    ])
    .areas(area);

    cpu::render(app, cpu_area, buf);
    cpu::render_battery_meter(app, cpu_area, buf);

    let (disk_width, disk_height) = disk::preferred_size(app.disks.all().len());

    let (net_width, net_height) = net::preferred_size();

    // Memory, disks and network share the row under the CPU box, side by side.
    let [mem_column, disk_column, net_column] = Layout::horizontal([
        Constraint::Length(mem_width.min(rest.width)),
        Constraint::Length(disk_width),
        Constraint::Min(0),
    ])
    .areas(rest);

    let [mem_area] =
        Layout::vertical([Constraint::Length(mem_height.min(mem_column.height))]).areas(mem_column);
    mem::render(app, mem_area, buf);

    let [disk_area] = Layout::vertical([Constraint::Length(disk_height.min(disk_column.height))])
        .areas(disk_column);
    disk::render(app, disk_area, buf);

    // The network box takes the leftover width, so it grows with the terminal.
    let [net_area] =
        Layout::vertical([Constraint::Length(net_height.min(net_column.height))]).areas(net_column);
    if net_area.width >= net_width / 2 {
        net::render(app, net_area, buf);
    }

    render_background(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(buf: &Buffer) -> String {
        let width = buf.area.width as usize;
        buf.content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn draw(width: u16, height: u16) -> Buffer {
        let mut app = App::new();
        app.tick();
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(&app, area, &mut buf);
        buf
    }

    #[test]
    fn both_boxes_are_drawn() {
        let rendered = text(&draw(120, 30));
        assert!(rendered.contains("CPU"), "cpu box missing");
        assert!(rendered.contains("mem"), "mem box missing");
        assert!(rendered.contains("Total:"), "mem contents missing");
    }

    #[test]
    fn memory_sits_below_the_cpu_box() {
        let rendered = text(&draw(120, 30));
        let rows: Vec<&str> = rendered.lines().collect();
        let cpu_row = rows.iter().position(|r| r.contains("CPU")).expect("cpu");
        let mem_row = rows.iter().position(|r| r.contains("Total:")).expect("mem");
        assert!(mem_row > cpu_row, "mem should be under cpu");
    }

    #[test]
    fn a_short_terminal_drops_the_mem_box_rather_than_panicking() {
        let rendered = text(&draw(120, 8));
        assert!(rendered.contains("CPU"), "cpu box should survive");
    }

    #[test]
    fn tiny_areas_do_not_panic() {
        for width in 0..16 {
            for height in 0..8 {
                draw(width, height);
            }
        }
    }

    #[test]
    fn background_covers_everything() {
        let buf = draw(100, 30);
        for cell in buf.content() {
            assert_eq!(cell.bg, theme::MAIN_BG);
        }
    }
}
