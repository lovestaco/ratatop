//! The memory box.
//!
//! One labelled row per field with the value right-aligned, and under each an
//! indented meter with its share of total. Total gets no meter: it is the
//! denominator, so a bar for it would always be full.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget};

use crate::app::App;
use crate::theme::{self, Gradient};
use crate::widgets::meter::Meter;

/// Width of the meter under each value.
const METER_WIDTH: u16 = 10;
/// Width of the right-aligned percentage beside the meter.
const PERCENT_WIDTH: u16 = 5;
/// Widest label plus its colon, e.g. `Available:`.
const LABEL_WIDTH: u16 = 10;

pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::DIV_LINE))
        .title(Span::styled("mem", Style::default().fg(theme::TITLE)));

    let inner = block.inner(area);
    block.render(area, buf);
    if inner.is_empty() {
        return;
    }

    let mem = app.memory;
    let mut y = inner.y;

    // Total is a plain figure with no meter under it.
    render_value(inner, &mut y, "Total:", mem.total, buf);

    for (label, value, gradient) in [
        ("Used:", mem.used, theme::MEM_USED),
        ("Available:", mem.available, theme::MEM_AVAILABLE),
        ("Cached:", mem.cached, theme::MEM_CACHED),
        ("Free:", mem.free, theme::MEM_FREE),
    ] {
        render_value(inner, &mut y, label, value, buf);
        render_meter(inner, &mut y, mem.percent(value), gradient, buf);
    }
}

/// `Used:                6.37 GiB`
fn render_value(inner: Rect, y: &mut u16, label: &str, bytes: u64, buf: &mut Buffer) {
    if *y >= inner.bottom() {
        return;
    }
    let row = Rect::new(inner.x, *y, inner.width, 1);
    let [label_area, value_area] =
        Layout::horizontal([Constraint::Length(LABEL_WIDTH), Constraint::Min(0)]).areas(row);

    Paragraph::new(Line::from(Span::styled(
        label.to_string(),
        Style::default().fg(theme::MAIN_FG),
    )))
    .render(label_area, buf);

    Paragraph::new(Line::from(Span::styled(
        format_bytes(bytes),
        Style::default().fg(theme::TITLE),
    )))
    .alignment(Alignment::Right)
    .render(value_area, buf);

    *y += 1;
}

/// The indented `▰▰▰▰▱▱▱▱▱▱  47%` line under a value.
fn render_meter(inner: Rect, y: &mut u16, percent: f64, gradient: Gradient, buf: &mut Buffer) {
    if *y >= inner.bottom() {
        return;
    }
    let row = Rect::new(inner.x, *y, inner.width, 1);
    let [_indent, meter, percent_area] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(METER_WIDTH),
        Constraint::Length(PERCENT_WIDTH),
    ])
    .areas(row);

    // Segmented rather than solid: these sit under a number that already gives
    // the exact figure, so the bar only needs to convey a rough proportion.
    Meter::new(percent, gradient).render(meter, buf);

    Paragraph::new(Line::from(Span::styled(
        format!("{percent:.0}%"),
        Style::default().fg(gradient.sample(percent / 100.0)),
    )))
    .alignment(Alignment::Right)
    .render(percent_area, buf);

    *y += 1;
}

/// Bytes as three significant figures plus a binary unit, like btop.
///
/// `13.5 GiB`, `6.37 GiB`, `485 MiB`.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    // Keep the figure three digits wide so the column stays aligned.
    let decimals = if unit == 0 || value >= 100.0 {
        0
    } else if value >= 10.0 {
        1
    } else {
        2
    };
    format!("{value:.decimals$} {}", UNITS[unit])
}

/// The size the box wants: total, then a value and meter for four fields.
pub fn preferred_size() -> (u16, u16) {
    let height = 2 + 1 + 4 * 2;
    let width = LABEL_WIDTH + 2 + METER_WIDTH + PERCENT_WIDTH + 2;
    (width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::mem::Memory;

    fn draw(mem: Memory, width: u16, height: u16) -> Buffer {
        let mut app = App::new();
        app.memory = mem;
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(&app, area, &mut buf);
        buf
    }

    fn text(buf: &Buffer) -> String {
        let width = buf.area.width as usize;
        buf.content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn sample() -> Memory {
        // The reading from the btop screenshot this box is modelled on.
        let total = 14_177_796 * 1024;
        let available = 7_490_900 * 1024;
        Memory {
            total,
            available,
            used: total - available,
            cached: 5_146_132 * 1024,
            free: 497_404 * 1024,
        }
    }

    #[test]
    fn formats_three_significant_figures() {
        assert_eq!(format_bytes(14_177_796 * 1024), "13.5 GiB");
        assert_eq!(format_bytes(6_686_896 * 1024), "6.38 GiB");
        assert_eq!(format_bytes(497_404 * 1024), "486 MiB");
    }

    #[test]
    fn formats_small_and_zero_values() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B", "bytes never get decimals");
        assert_eq!(format_bytes(1024), "1.00 KiB");
    }

    #[test]
    fn formats_stop_at_the_largest_unit() {
        // Well past TiB; the loop must not run off the end of the unit table.
        assert!(format_bytes(u64::MAX).ends_with("TiB"));
    }

    #[test]
    fn renders_every_field() {
        let (width, height) = preferred_size();
        let rendered = text(&draw(sample(), width, height));
        for label in ["Total:", "Used:", "Available:", "Cached:", "Free:"] {
            assert!(rendered.contains(label), "{label} missing");
        }
        assert!(rendered.contains("13.5 GiB"), "total value missing");
        assert!(rendered.contains("mem"), "title missing");
    }

    #[test]
    fn total_has_no_meter_under_it() {
        let (width, height) = preferred_size();
        let rendered = text(&draw(sample(), width, height));
        let rows: Vec<&str> = rendered.lines().collect();
        let total_row = rows.iter().position(|r| r.contains("Total:")).unwrap();
        assert!(
            rows[total_row + 1].contains("Used:"),
            "Used should follow Total directly, got {:?}",
            rows[total_row + 1]
        );
    }

    #[test]
    fn percentages_reflect_the_reading() {
        let (width, height) = preferred_size();
        let rendered = text(&draw(sample(), width, height));
        // used is 47% of total, available 53%.
        assert!(rendered.contains("47%"), "used percent missing");
        assert!(rendered.contains("53%"), "available percent missing");
    }

    #[test]
    fn each_field_gets_its_own_gradient() {
        let (width, height) = preferred_size();
        let buf = draw(sample(), width, height);
        let colors: std::collections::HashSet<_> = buf
            .content()
            .iter()
            .filter(|cell| cell.symbol() == "▰")
            .map(|cell| cell.fg)
            .collect();
        assert!(colors.len() > 1, "meters should not all be one color");
    }

    #[test]
    fn a_short_box_clips_rows_instead_of_panicking() {
        let (width, _) = preferred_size();
        for height in 0..12 {
            draw(sample(), width, height);
        }
    }

    #[test]
    fn tiny_and_empty_areas_are_safe() {
        for width in 0..14 {
            for height in 0..4 {
                draw(sample(), width, height);
            }
        }
        draw(Memory::default(), 30, 12);
    }
}
