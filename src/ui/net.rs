//! The network box.
//!
//! btop's arrangement: the interface name and address on the top border, a
//! download graph over a mirrored upload graph on the left, and a stats panel on
//! the right whose own borders are labelled `download` and `upload`.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget};

use crate::app::App;
use crate::collect::net::{Direction, Interface};
use crate::theme;
use crate::widgets::braille::BrailleGraph;

use super::mem::format_bytes;

/// Width of the stats panel on the right.
const STATS_WIDTH: u16 = 30;
/// Rows the stats panel needs: three per direction.
const STATS_ROWS: u16 = 6;
/// Width of the label column inside the stats panel.
const STATS_LABEL_WIDTH: u16 = 9;

pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let interface = app.net.selected();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::DIV_LINE))
        .title(title(interface))
        .title(selector(interface).alignment(Alignment::Right));

    let inner = block.inner(area);
    block.render(area, buf);
    if inner.is_empty() {
        return;
    }

    let Some(interface) = interface else {
        Paragraph::new(Line::from(Span::styled(
            "no interface",
            Style::default().fg(theme::INACTIVE_FG),
        )))
        .render(inner, buf);
        return;
    };

    let [graphs, stats] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(STATS_WIDTH.min(inner.width)),
    ])
    .areas(inner);

    render_graphs(interface, graphs, buf);
    render_stats(interface, stats, buf);
}

/// Download above, upload mirrored below, sharing one scale.
fn render_graphs(interface: &Interface, area: Rect, buf: &mut Buffer) {
    if area.is_empty() {
        return;
    }
    let [download, upload] =
        Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).areas(area);

    // Both graphs share a scale so their heights are directly comparable; two
    // independent scales would make a trickle look like a flood.
    let max = interface.graph_max();

    BrailleGraph::new(&scaled(&interface.download, max), theme::CPU).render(download, buf);
    BrailleGraph::new(&scaled(&interface.upload, max), theme::CPU)
        .inverted()
        .render(upload, buf);

    // The scale label sits at the outer edge of each graph, where btop puts it.
    let label = format_speed(max);
    render_scale(&label, download, true, buf);
    render_scale(&label, upload, false, buf);
}

fn render_scale(label: &str, area: Rect, top: bool, buf: &mut Buffer) {
    if area.is_empty() {
        return;
    }
    let y = if top { area.y } else { area.bottom() - 1 };
    Paragraph::new(Line::from(Span::styled(
        label.to_string(),
        Style::default().fg(theme::DIV_LINE),
    )))
    .render(Rect::new(area.x, y, area.width.min(10), 1), buf);
}

/// The graph takes percentages, so rates have to be scaled against the max.
fn scaled(direction: &Direction, max: f64) -> Vec<f64> {
    if max <= 0.0 {
        return Vec::new();
    }
    direction
        .history
        .iter()
        .map(|rate| (rate / max * 100.0).clamp(0.0, 100.0))
        .collect()
}

/// The bordered panel of current, peak and cumulative figures.
fn render_stats(interface: &Interface, area: Rect, buf: &mut Buffer) {
    if area.is_empty() {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::DIV_LINE))
        .title(Span::styled(
            "download",
            Style::default()
                .fg(theme::TITLE)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            "upload",
            Style::default()
                .fg(theme::TITLE)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    block.render(area, buf);
    if inner.is_empty() {
        return;
    }

    let rows: Vec<(&str, &str, String, String)> = vec![
        (
            "▼",
            "",
            format_speed_per_second(interface.download.rate),
            format_bits(interface.download.rate),
        ),
        (
            "▼",
            "Top:",
            String::new(),
            format_bits(interface.download.top),
        ),
        (
            "▼",
            "Total:",
            format_bytes(interface.download.total),
            String::new(),
        ),
        (
            "▲",
            "",
            format_speed_per_second(interface.upload.rate),
            format_bits(interface.upload.rate),
        ),
        (
            "▲",
            "Top:",
            String::new(),
            format_bits(interface.upload.top),
        ),
        (
            "▲",
            "Total:",
            format_bytes(interface.upload.total),
            String::new(),
        ),
    ];

    for (index, (arrow, label, value, trailing)) in rows.iter().enumerate() {
        let y = inner.y + index as u16;
        if y >= inner.bottom() {
            break;
        }
        render_stat_row(
            Rect::new(inner.x, y, inner.width, 1),
            arrow,
            label,
            value,
            trailing,
            buf,
        );
    }
}

fn render_stat_row(
    area: Rect,
    arrow: &str,
    label: &str,
    value: &str,
    trailing: &str,
    buf: &mut Buffer,
) {
    let [arrow_area, label_area, rest] = Layout::horizontal([
        Constraint::Length(2),
        Constraint::Length(STATS_LABEL_WIDTH),
        Constraint::Min(0),
    ])
    .areas(area);

    // Down is the reassuring direction, up is the one that costs you money.
    let arrow_color = if arrow == "▼" {
        theme::MEM_CACHED
    } else {
        theme::MEM_FREE
    };
    Paragraph::new(Line::from(Span::styled(
        arrow.to_string(),
        Style::default().fg(arrow_color.sample(1.0)),
    )))
    .render(arrow_area, buf);

    // A rate has no label, so its figure starts where a label would.
    if label.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            value.to_string(),
            Style::default().fg(theme::TITLE),
        )))
        .render(label_area.union(rest), buf);
    } else {
        Paragraph::new(Line::from(Span::styled(
            label.to_string(),
            Style::default().fg(theme::MAIN_FG),
        )))
        .render(label_area, buf);

        Paragraph::new(Line::from(Span::styled(
            value.to_string(),
            Style::default().fg(theme::TITLE),
        )))
        .alignment(Alignment::Right)
        .render(rest, buf);
    }

    if !trailing.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            trailing.to_string(),
            Style::default().fg(theme::MAIN_FG),
        )))
        .alignment(Alignment::Right)
        .render(rest, buf);
    }
}

/// `net  192.168.29.14` on the top border.
fn title(interface: Option<&Interface>) -> Line<'static> {
    let mut spans = vec![
        Span::styled("n", Style::default().fg(theme::HI_FG)),
        Span::styled("et", Style::default().fg(theme::TITLE)),
    ];
    if let Some(ip) = interface.and_then(|i| i.ip.clone()) {
        spans.push(Span::styled(
            format!("  {ip}"),
            Style::default().fg(theme::MAIN_FG),
        ));
    }
    Line::from(spans)
}

/// `<b wlo1 n>` on the top right, showing the cycle keys.
fn selector(interface: Option<&Interface>) -> Line<'static> {
    let name = interface.map_or("none".to_string(), |i| i.name.clone());
    Line::from(vec![
        Span::styled("<b ", Style::default().fg(theme::HI_FG)),
        Span::styled(
            name,
            Style::default()
                .fg(theme::TITLE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" n>", Style::default().fg(theme::HI_FG)),
    ])
}

/// Compact scale label, e.g. `10K`.
fn format_speed(bytes_per_sec: f64) -> String {
    const UNITS: [&str; 4] = ["", "K", "M", "G"];
    let (value, unit) = scale(bytes_per_sec, 1024.0, UNITS.len());
    format!("{value:.0}{}", UNITS[unit])
}

/// Current throughput, e.g. `566 Byte/s`, `1.23 KiB/s`.
fn format_speed_per_second(bytes_per_sec: f64) -> String {
    const UNITS: [&str; 4] = ["Byte/s", "KiB/s", "MiB/s", "GiB/s"];
    let (value, unit) = scale(bytes_per_sec, 1024.0, UNITS.len());
    let decimals = decimals(value, unit);
    format!("{value:.decimals$} {}", UNITS[unit])
}

/// The same figure in bits, e.g. `(4.42 Kibps)`.
///
/// Network links are sold in bits per second, so btop shows both and lets you
/// stop doing the factor-of-eight conversion in your head.
fn format_bits(bytes_per_sec: f64) -> String {
    const UNITS: [&str; 4] = ["bps", "Kibps", "Mibps", "Gibps"];
    let (value, unit) = scale(bytes_per_sec * 8.0, 1024.0, UNITS.len());
    let decimals = decimals(value, unit);
    format!("({value:.decimals$} {})", UNITS[unit])
}

/// Three significant figures, the same rule the memory box uses.
///
/// Keeps every figure the same width so the column does not jitter as traffic
/// moves between `4.42`, `43.8` and `438`.
fn decimals(value: f64, unit: usize) -> usize {
    if unit == 0 || value >= 100.0 {
        0
    } else if value >= 10.0 {
        1
    } else {
        2
    }
}

/// Divide down to the largest unit that keeps the figure under `step`.
fn scale(value: f64, step: f64, units: usize) -> (f64, usize) {
    let mut value = value.max(0.0);
    let mut unit = 0;
    while value >= step && unit < units - 1 {
        value /= step;
        unit += 1;
    }
    (value, unit)
}

/// The size the box wants.
pub fn preferred_size() -> (u16, u16) {
    (STATS_WIDTH + 26, STATS_ROWS + 4)
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
        let app = App::new();
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(&app, area, &mut buf);
        buf
    }

    #[test]
    fn formats_throughput_in_bytes() {
        assert_eq!(format_speed_per_second(0.0), "0 Byte/s");
        assert_eq!(format_speed_per_second(566.0), "566 Byte/s");
        assert_eq!(format_speed_per_second(1024.0), "1.00 KiB/s");
        assert_eq!(format_speed_per_second(1024.0 * 1536.0), "1.50 MiB/s");
    }

    #[test]
    fn figures_keep_three_significant_digits() {
        // btop writes 43.8, not 43.80, so the column stays a fixed width.
        assert_eq!(format_bits(5606.0), "(43.8 Kibps)");
        assert_eq!(format_bits(56.0), "(448 bps)");
        assert_eq!(format_speed_per_second(1024.0 * 43.8), "43.8 KiB/s");
    }

    #[test]
    fn formats_the_same_figure_in_bits() {
        // 566 bytes/s is 4528 bits/s, which is 4.42 Kibps.
        assert_eq!(format_bits(566.0), "(4.42 Kibps)");
        assert_eq!(format_bits(0.0), "(0 bps)");
    }

    #[test]
    fn scale_labels_are_terse() {
        assert_eq!(format_speed(10.0 * 1024.0), "10K");
        assert_eq!(format_speed(0.0), "0");
    }

    #[test]
    fn formatting_stops_at_the_largest_unit() {
        assert!(format_speed_per_second(f64::MAX).ends_with("GiB/s"));
        assert!(format_bits(f64::MAX).ends_with("Gibps)"));
        assert!(format_speed(f64::MAX).ends_with('G'));
    }

    #[test]
    fn negative_rates_never_appear() {
        // A counter reset should already be clamped upstream, but the formatter
        // must not print a minus sign if one slips through.
        assert_eq!(format_speed_per_second(-5.0), "0 Byte/s");
    }

    #[test]
    fn renders_both_directions_and_the_interface() {
        let app = App::new();
        let (width, height) = preferred_size();
        let rendered = text(&draw(width, height));

        assert!(rendered.contains("net"), "title missing");
        assert!(rendered.contains("download"), "download label missing");
        assert!(rendered.contains("upload"), "upload label missing");
        assert!(rendered.contains("Top:"), "peak row missing");
        assert!(rendered.contains("Total:"), "total row missing");
        assert!(rendered.contains('▼') && rendered.contains('▲'), "arrows");

        if let Some(iface) = app.net.selected() {
            assert!(rendered.contains(&iface.name), "interface name missing");
        }
    }

    #[test]
    fn scaling_history_maps_rates_onto_percentages() {
        let direction = Direction {
            history: vec![0.0, 500.0, 1000.0, 2000.0],
            ..Direction::default()
        };
        assert_eq!(scaled(&direction, 1000.0), vec![0.0, 50.0, 100.0, 100.0]);
        assert!(
            scaled(&direction, 0.0).is_empty(),
            "no scale means no graph, not a divide by zero"
        );
    }

    #[test]
    fn both_graphs_share_one_scale() {
        // Both above the 10 KiB floor, so the peak decides the scale.
        let interface = Interface {
            download: Direction {
                top: 800_000.0,
                ..Direction::default()
            },
            upload: Direction {
                top: 20_000.0,
                ..Direction::default()
            },
            ..Interface::default()
        };
        // The upload graph must not be rescaled to its own peak, or a trickle
        // would look identical to a flood.
        assert_eq!(interface.graph_max(), 800_000.0);
    }

    #[test]
    fn tiny_and_empty_areas_are_safe() {
        for width in 0..18 {
            for height in 0..8 {
                draw(width, height);
            }
        }
    }
}
