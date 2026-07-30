//! The disks box.
//!
//! Three rows per disk, matching btop's I/O mode: the name with combined
//! throughput and total size, an `IO%` utilisation graph, then a `Used:` meter
//! with the used size.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget};

use crate::app::App;
use crate::collect::disk::Disk;
use crate::theme;
use crate::widgets::braille::BrailleGraph;
use crate::widgets::meter::Meter;

use super::mem::format_bytes;

/// Rows each disk occupies: name, I/O graph, used meter.
const ROWS_PER_DISK: u16 = 3;
/// Width of the `Used:` label.
const USED_LABEL_WIDTH: u16 = 6;
/// Width of the percentage beside the used meter.
const PERCENT_WIDTH: u16 = 5;
/// Width of the right-aligned byte figures.
const VALUE_WIDTH: u16 = 10;
/// Blank cells between the percentage and the start of the bar.
const BAR_INDENT: u16 = 2;

pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::DIV_LINE))
        .title(Span::styled(
            "disks",
            Style::default()
                .fg(theme::HI_FG)
                .add_modifier(Modifier::BOLD),
        ))
        .title(
            Line::from(Span::styled("io", Style::default().fg(theme::HI_FG)))
                .alignment(Alignment::Right),
        );

    let inner = block.inner(area);
    block.render(area, buf);
    if inner.is_empty() {
        return;
    }

    // Whole disks only: a partially drawn disk reads as a rendering bug.
    let capacity = (inner.height / ROWS_PER_DISK) as usize;
    for (index, disk) in app.disks.all().iter().take(capacity).enumerate() {
        let top = inner.y + index as u16 * ROWS_PER_DISK;
        render_disk(
            disk,
            Rect::new(inner.x, top, inner.width, ROWS_PER_DISK),
            buf,
        );
    }
}

fn render_disk(disk: &Disk, area: Rect, buf: &mut Buffer) {
    let [name_row, io_row, used_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    render_name(disk, name_row, buf);
    render_io(disk, io_row, buf);
    render_used(disk, used_row, buf);
}

/// `root            ▼60K    91.1 GiB`
fn render_name(disk: &Disk, area: Rect, buf: &mut Buffer) {
    Paragraph::new(Line::from(Span::styled(
        disk.name.clone(),
        Style::default()
            .fg(theme::TITLE)
            .add_modifier(Modifier::BOLD),
    )))
    .render(area, buf);

    // Throughput sits between the name and the size, so neither has to move as
    // it changes width.
    let rate = disk.io_rate();
    if rate >= 1.0 {
        Paragraph::new(Line::from(Span::styled(
            format!("▼{}", format_rate(rate)),
            Style::default().fg(theme::MAIN_FG),
        )))
        .alignment(Alignment::Center)
        .render(area, buf);
    }

    Paragraph::new(Line::from(Span::styled(
        format_bytes(disk.total),
        Style::default()
            .fg(theme::TITLE)
            .add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Right)
    .render(area, buf);
}

/// `IO%` plus a utilisation graph across the rest of the row.
fn render_io(disk: &Disk, area: Rect, buf: &mut Buffer) {
    let [label, graph] =
        Layout::horizontal([Constraint::Length(4), Constraint::Min(0)]).areas(area);

    Paragraph::new(Line::from(Span::styled(
        "IO%",
        Style::default().fg(theme::DIV_LINE),
    )))
    .render(label, buf);

    // Swap has no block device behind it, so there is no utilisation to plot.
    if !disk.is_swap {
        BrailleGraph::new(&disk.io_history, theme::CPU).render(graph, buf);
    }
}

/// `Used:  79%  ██████████░░░  72.1 GiB`
fn render_used(disk: &Disk, area: Rect, buf: &mut Buffer) {
    let percent = disk.used_percent();
    let [label, percent_area, bar, value] = Layout::horizontal([
        Constraint::Length(USED_LABEL_WIDTH),
        Constraint::Length(PERCENT_WIDTH),
        Constraint::Min(0),
        Constraint::Length(VALUE_WIDTH),
    ])
    .areas(area);

    Paragraph::new(Line::from(Span::styled(
        "Used:",
        Style::default().fg(theme::MAIN_FG),
    )))
    .render(label, buf);

    Paragraph::new(Line::from(Span::styled(
        format!("{percent:.0}%"),
        Style::default().fg(theme::MEM_USED.sample(percent / 100.0)),
    )))
    .alignment(Alignment::Right)
    .render(percent_area, buf);

    // Breathing room between the percentage and the bar.
    let bar = Rect {
        x: bar.x.saturating_add(BAR_INDENT),
        width: bar.width.saturating_sub(BAR_INDENT),
        ..bar
    };
    Meter::new(percent, theme::MEM_USED)
        .solid()
        .render(bar, buf);

    Paragraph::new(Line::from(Span::styled(
        format_bytes(disk.used),
        Style::default().fg(theme::TITLE),
    )))
    .alignment(Alignment::Right)
    .render(value, buf);
}

/// Compact throughput, as btop writes it: `60K`, `1.2M`.
///
/// Deliberately terser than `format_bytes`: this sits in a narrow gap between
/// two other figures, so a full `60.0 KiB/s` would not fit.
fn format_rate(bytes_per_sec: f64) -> String {
    const UNITS: [&str; 4] = ["", "K", "M", "G"];

    let mut value = bytes_per_sec;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{value:.0}");
    }
    if value < 10.0 {
        format!("{value:.1}{}", UNITS[unit])
    } else {
        format!("{value:.0}{}", UNITS[unit])
    }
}

/// Cells the used-meter needs to read as a bar rather than a few blocks.
const MIN_BAR_WIDTH: u16 = 18;

/// The size the box wants, given how many disks there are.
pub fn preferred_size(disks: usize) -> (u16, u16) {
    let height = 2 + ROWS_PER_DISK * disks.max(1) as u16;
    let width = USED_LABEL_WIDTH + PERCENT_WIDTH + BAR_INDENT + MIN_BAR_WIDTH + VALUE_WIDTH + 2;
    (width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(name: &str, total: u64, used: u64) -> Disk {
        Disk {
            name: name.to_string(),
            mount: format!("/{name}"),
            total,
            used,
            free: total - used,
            io_history: vec![10.0, 40.0, 80.0],
            ..Disk::default()
        }
    }

    fn draw(width: u16, height: u16) -> Buffer {
        let app = App::new();
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(&app, area, &mut buf);
        buf
    }

    fn draw_disk(d: &Disk, width: u16) -> String {
        let area = Rect::new(0, 0, width, ROWS_PER_DISK);
        let mut buf = Buffer::empty(area);
        render_disk(d, area, &mut buf);
        buf.content()
            .chunks(width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_name_size_used_and_percent() {
        let gib = 1024u64.pow(3);
        let rendered = draw_disk(&disk("home", 356 * gib, 348 * gib), 40);
        assert!(rendered.contains("home"), "name missing");
        assert!(rendered.contains("356 GiB"), "total missing");
        assert!(rendered.contains("348 GiB"), "used missing");
        assert!(rendered.contains("98%"), "percent missing");
        assert!(rendered.contains("IO%"), "io label missing");
    }

    #[test]
    fn each_disk_takes_exactly_three_rows() {
        let gib = 1024u64.pow(3);
        let rendered = draw_disk(&disk("root", 100 * gib, 50 * gib), 40);
        assert_eq!(rendered.lines().count(), 3);
    }

    #[test]
    fn swap_gets_no_io_graph() {
        let mut swap = disk("swap", 1000, 500);
        swap.is_swap = true;
        swap.io_history = vec![100.0, 100.0, 100.0];
        let rendered = draw_disk(&swap, 40);
        // A full graph would draw braille; swap has no block device to measure.
        assert!(
            !rendered.contains('⣿'),
            "swap should not plot io: {rendered:?}"
        );
    }

    #[test]
    fn throughput_is_hidden_when_idle() {
        let gib = 1024u64.pow(3);
        let mut d = disk("root", 10 * gib, 5 * gib);
        assert!(!draw_disk(&d, 40).contains('▼'), "idle disk shows no rate");

        d.read_rate = 60_000.0;
        assert!(draw_disk(&d, 40).contains('▼'), "busy disk shows a rate");
    }

    #[test]
    fn formats_rates_compactly() {
        assert_eq!(format_rate(0.0), "0");
        assert_eq!(format_rate(512.0), "512");
        assert_eq!(format_rate(61_440.0), "60K");
        assert_eq!(format_rate(1024.0 * 1024.0 * 1.5), "1.5M");
        assert_eq!(format_rate(1024.0 * 1024.0 * 1024.0 * 2.0), "2.0G");
    }

    #[test]
    fn rate_formatting_stops_at_the_largest_unit() {
        assert!(format_rate(f64::MAX).ends_with('G'), "no overflow past G");
    }

    #[test]
    fn shows_only_whole_disks_that_fit() {
        // Two disks need six rows plus borders; eight rows fits exactly two.
        let app = App::new();
        let count = app.disks.all().len();
        if count < 2 {
            return;
        }
        let buf = draw(46, 2 + ROWS_PER_DISK * 2);
        let rendered: String = buf
            .content()
            .chunks(46)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        let shown = app
            .disks
            .all()
            .iter()
            .filter(|d| rendered.contains(&d.name))
            .count();
        assert_eq!(shown, 2, "should fit exactly two disks");
    }

    #[test]
    fn preferred_size_grows_with_disk_count() {
        assert_eq!(preferred_size(1).1, 2 + 3);
        assert_eq!(preferred_size(4).1, 2 + 12);
        assert_eq!(preferred_size(0).1, 2 + 3, "never zero height");
    }

    #[test]
    fn tiny_and_empty_areas_are_safe() {
        for width in 0..16 {
            for height in 0..6 {
                draw(width, height);
            }
        }
    }

    #[test]
    fn a_disk_with_no_size_does_not_panic() {
        let rendered = draw_disk(&disk("empty", 0, 0), 40);
        assert!(rendered.contains("0%"), "unstatable mount reads as 0%");
    }
}
