//! The CPU box.
//!
//! Follows btop's compact arrangement: the CPU model and frequency sit on the top
//! border, the aggregate load is a solid gradient bar rather than a graph, cores
//! are dealt into as many columns as the box height needs, and the load averages
//! sit bottom-right.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget};

use crate::app::App;
use crate::theme;
use crate::widgets::braille::BrailleGraph;
use crate::widgets::meter::Meter;

/// Width of the `CPU` / `C0` label column.
const LABEL_WIDTH: u16 = 4;
/// Width of the right-aligned percentage column, including the sign.
const PERCENT_WIDTH: u16 = 5;
/// Width of the temperature readout, e.g. ` 52°C`.
const TEMP_WIDTH: u16 = 5;
/// Width of the small temperature history graph beside the readout.
const TEMP_GRAPH_WIDTH: u16 = 6;
/// Cells the battery meter occupies on the top border.
const BATTERY_METER_WIDTH: u16 = 10;
/// Narrowest a core column can get before we use fewer columns.
const MIN_CORE_COLUMN_WIDTH: u16 = 22;
/// Blank cells between core columns.
const CORE_COLUMN_GAP: u16 = 2;

pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        // btop's boxes have rounded corners; ratatui has the glyph set built in.
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::DIV_LINE))
        .title(top_left_title(app))
        .title(frequency_title(app).alignment(Alignment::Right))
        .title_bottom(hint_title())
        .title_bottom(interval_title(app).alignment(Alignment::Right));

    let inner = block.inner(area);
    block.render(area, buf);
    if inner.is_empty() {
        return;
    }

    // The aggregate row and the load row are fixed; cores take what is left, so
    // a short box hides cores instead of clipping the load averages.
    let [total, cores, load] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(inner);

    render_total(app, total, buf);
    render_cores(app, cores, buf);
    render_load(app, load, buf);
}

/// `CPU  ████░░░░░░░░  14%  ⣀⣠⣤  52°C`
fn render_total(app: &App, area: Rect, buf: &mut Buffer) {
    if area.is_empty() {
        return;
    }
    let [label, bar, percent, _gap, temp_graph, temp] = Layout::horizontal([
        Constraint::Length(LABEL_WIDTH),
        Constraint::Min(0),
        Constraint::Length(PERCENT_WIDTH),
        Constraint::Length(1),
        Constraint::Length(TEMP_GRAPH_WIDTH),
        Constraint::Length(TEMP_WIDTH),
    ])
    .areas(area);

    // btop highlights the first letter of a label in `hi_fg`.
    Paragraph::new(Line::from(vec![
        Span::styled("C", Style::default().fg(theme::HI_FG)),
        Span::styled("PU", Style::default().fg(theme::TITLE)),
    ]))
    .render(label, buf);

    // A bar, not a graph: the aggregate is never idle, so a history graph puts
    // one dot in every column and reads as a flat line.
    let total = app.total_percent();
    Meter::new(total, theme::CPU).solid().render(bar, buf);
    render_percent(total, percent, buf);

    render_temp_graph(app, temp_graph, buf);
    render_temp(app, temp, buf);
}

fn render_temp_graph(app: &App, area: Rect, buf: &mut Buffer) {
    if app.temp_history.is_empty() {
        return;
    }
    // Scale against the sensor's critical point so the graph shares the readout's
    // sense of "hot" rather than treating 100 °C as full scale.
    let scaled: Vec<f64> = app
        .temp_history
        .iter()
        .map(|&celsius| theme::temp_position(celsius, app.info.temp_critical) * 100.0)
        .collect();
    BrailleGraph::new(&scaled, theme::TEMP).render(area, buf);
}

fn render_temp(app: &App, area: Rect, buf: &mut Buffer) {
    let (text, style) = match app.temperature {
        Some(celsius) => (
            format!("{celsius:.0}°C"),
            Style::default()
                .fg(theme::TEMP.sample(theme::temp_position(celsius, app.info.temp_critical))),
        ),
        None => ("--".to_string(), Style::default().fg(theme::INACTIVE_FG)),
    };
    Paragraph::new(Line::from(Span::styled(text, style)))
        .alignment(Alignment::Right)
        .render(area, buf);
}

/// Core rows, dealt down each column then across — `C0`..`C3` beside `C4`..`C7`.
fn render_cores(app: &App, area: Rect, buf: &mut Buffer) {
    if area.is_empty() || app.core_count() == 0 {
        return;
    }
    let columns = core_columns(app.core_count(), area);
    let rows = app.core_count().div_ceil(columns);

    let column_areas = Layout::horizontal(vec![
        Constraint::Ratio(
            1,
            u32::try_from(columns).unwrap_or(1)
        );
        columns
    ])
    // Without a gap a column's percentage runs straight into the next column's
    // label — `25%C5`.
    .spacing(CORE_COLUMN_GAP)
    .split(area);

    for (column, column_area) in column_areas.iter().enumerate() {
        for row in 0..rows.min(area.height as usize) {
            // Fill each column top to bottom before starting the next, matching
            // btop; going across instead would interleave the numbering.
            let index = column * rows + row;
            if index >= app.core_count() {
                break;
            }
            let line = Rect::new(
                column_area.x,
                column_area.y + row as u16,
                column_area.width,
                1,
            );
            render_core(app, index, line, buf);
        }
    }
}

fn render_core(app: &App, index: usize, area: Rect, buf: &mut Buffer) {
    let [label, graph, percent] = Layout::horizontal([
        Constraint::Length(LABEL_WIDTH),
        Constraint::Min(0),
        Constraint::Length(PERCENT_WIDTH),
    ])
    .areas(area);

    Paragraph::new(Line::from(Span::styled(
        format!("C{index}"),
        Style::default().fg(theme::MAIN_FG),
    )))
    .render(label, buf);

    let history = app.core_history.get(index).map_or(&[][..], Vec::as_slice);
    BrailleGraph::new(history, theme::CPU).render(graph, buf);
    render_percent(app.core_percent(index), percent, buf);
}

/// How many columns to deal cores into.
///
/// Enough that every core fits the available height, but never so many that a
/// column has no room for a graph.
fn core_columns(cores: usize, area: Rect) -> usize {
    if area.height == 0 {
        return 1;
    }
    let needed = cores.div_ceil(area.height as usize).max(1);
    // Each extra column costs its own width plus a gap.
    let affordable = ((area.width + CORE_COLUMN_GAP) / (MIN_CORE_COLUMN_WIDTH + CORE_COLUMN_GAP))
        .max(1) as usize;
    needed.min(affordable)
}

/// `Load AVG:  1.16  1.13  0.89`, right-aligned like btop.
fn render_load(app: &App, area: Rect, buf: &mut Buffer) {
    let [one, five, fifteen] = app.load_average;
    Paragraph::new(Line::from(vec![
        Span::styled("Load ", Style::default().fg(theme::TITLE)),
        Span::styled("AVG: ", Style::default().fg(theme::MAIN_FG)),
        Span::styled(
            format!("{one:.2}  {five:.2}  {fifteen:.2}"),
            Style::default().fg(theme::MAIN_FG),
        ),
    ]))
    .alignment(Alignment::Right)
    .render(area, buf);
}

fn render_percent(value: f64, area: Rect, buf: &mut Buffer) {
    Paragraph::new(Line::from(Span::styled(
        format!("{value:.0}%"),
        Style::default().fg(theme::CPU.sample(value / 100.0)),
    )))
    .alignment(Alignment::Right)
    .render(area, buf);
}

/// `BAT⚡100%` — the text part only; the meter is drawn separately.
fn battery_text(app: &App) -> Option<Line<'static>> {
    let battery = app.battery?;
    Some(Line::from(vec![
        Span::styled("BAT", Style::default().fg(theme::TITLE)),
        Span::styled(battery.status.symbol(), Style::default().fg(theme::HI_FG)),
        Span::styled(
            format!("{}%", battery.percent),
            Style::default().fg(theme::MAIN_FG),
        ),
    ]))
}

/// Battery, then the CPU model — both on the top border, left-aligned.
fn top_left_title(app: &App) -> Line<'static> {
    let mut spans = Vec::new();
    if let Some(battery) = battery_text(app) {
        spans.extend(battery.spans);
        // Reserve the meter's cells plus a space either side of it.
        spans.push(Span::raw(" ".repeat(BATTERY_METER_WIDTH as usize + 2)));
    }
    spans.push(Span::styled(
        app.info.model.clone(),
        Style::default()
            .fg(theme::TITLE)
            .add_modifier(Modifier::BOLD),
    ));
    Line::from(spans)
}

fn frequency_title(app: &App) -> Line<'static> {
    match app.frequency_ghz {
        Some(ghz) => Line::from(Span::styled(
            format!("{ghz:.1} GHz"),
            Style::default()
                .fg(theme::TITLE)
                .add_modifier(Modifier::BOLD),
        )),
        None => Line::default(),
    }
}

/// Key hints, dim on the bottom border so they stay out of the way.
fn hint_title() -> Line<'static> {
    Line::from(vec![
        Span::styled(" q ", Style::default().fg(theme::HI_FG)),
        Span::styled("quit ", Style::default().fg(theme::DIV_LINE)),
        Span::styled(" +/- ", Style::default().fg(theme::HI_FG)),
        Span::styled("rate ", Style::default().fg(theme::DIV_LINE)),
    ])
}

fn interval_title(app: &App) -> Line<'static> {
    Line::from(Span::styled(
        format!("{}ms", app.interval.as_millis()),
        Style::default().fg(theme::DIV_LINE),
    ))
}

/// Draw the battery meter over the top border, in the gap `top_left_title` left.
///
/// This cannot be part of the title: `Block` renders titles as plain spans, and
/// the meter needs per-cell colors.
pub fn render_battery_meter(app: &App, area: Rect, buf: &mut Buffer) {
    let Some(battery) = app.battery else { return };
    let Some(text) = battery_text(app) else {
        return;
    };
    // Use the rendered width, not the character count: the status glyph is
    // double-width, and the percentage is two or three digits.
    let x = area.x + 1 + text.width() as u16 + 1;
    if x + BATTERY_METER_WIDTH >= area.x + area.width {
        return;
    }
    let meter = Rect::new(x, area.y, BATTERY_METER_WIDTH, 1);
    Meter::new(f64::from(battery.percent), theme::BATTERY).render(meter, buf);
}

/// The size the box wants: two columns of cores, btop's compact look.
pub fn preferred_size(app: &App) -> (u16, u16) {
    let columns = if app.core_count() > 4 { 2 } else { 1 };
    let rows = app.core_count().div_ceil(columns) as u16;
    // borders + aggregate row + core rows + load row
    let height = 2 + 1 + rows + 1;
    let columns = columns as u16;
    let width = (MIN_CORE_COLUMN_WIDTH + 6) * columns + CORE_COLUMN_GAP * (columns - 1) + 2;
    (width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(app: &App, width: u16, height: u16) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(app, area, &mut buf);
        render_battery_meter(app, area, &mut buf);
        super::super::render_background(area, &mut buf);
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

    fn ticked() -> App {
        let mut app = App::new();
        app.tick();
        app
    }

    #[test]
    fn renders_every_labelled_row() {
        let app = ticked();
        let (width, height) = preferred_size(&app);
        let rendered = text(&draw(&app, width, height));

        assert!(rendered.contains("CPU"), "aggregate row missing");
        assert!(rendered.contains("Load"), "load averages missing");
        assert!(rendered.contains(&app.info.model), "cpu model missing");
        for index in 0..app.core_count() {
            assert!(
                rendered.contains(&format!("C{index}")),
                "core {index} missing"
            );
        }
    }

    #[test]
    fn aggregate_load_is_a_solid_bar_not_a_graph() {
        let app = ticked();
        let rendered = text(&draw(&app, 60, 12));
        let cpu_row = rendered.lines().nth(1).expect("aggregate row");
        assert!(
            cpu_row.contains('█'),
            "expected a solid bar, got {cpu_row:?}"
        );
        assert!(
            !cpu_row.contains('⣀'),
            "the flat-line braille graph should be gone: {cpu_row:?}"
        );
    }

    #[test]
    fn cores_are_dealt_into_columns_top_to_bottom() {
        let app = ticked();
        if app.core_count() < 8 {
            return; // single-column machine; nothing to assert
        }
        let rendered = text(&draw(&app, 70, 9));
        let rows: Vec<&str> = rendered.lines().collect();
        // With four rows of cores, C0 and C4 head the two columns on one line.
        let first_core_row = rows.iter().find(|row| row.contains("C0")).expect("C0 row");
        assert!(
            first_core_row.contains("C4"),
            "expected C0 and C4 side by side, got {first_core_row:?}"
        );
        assert!(
            !first_core_row.contains("C1"),
            "columns must fill downward, not across: {first_core_row:?}"
        );
    }

    #[test]
    fn falls_back_to_one_column_when_too_narrow() {
        let area = Rect::new(0, 0, 30, 4);
        assert_eq!(core_columns(8, area), 1, "no room for two graphs");
        let wide = Rect::new(0, 0, 70, 4);
        assert_eq!(core_columns(8, wide), 2);
        assert_eq!(core_columns(8, Rect::new(0, 0, 70, 8)), 1, "tall enough");
        assert_eq!(core_columns(8, Rect::new(0, 0, 70, 0)), 1, "zero height");
    }

    #[test]
    fn load_averages_are_right_aligned() {
        let app = ticked();
        let rendered = text(&draw(&app, 60, 12));
        let load_row = rendered
            .lines()
            .find(|row| row.contains("Load"))
            .expect("load row");
        assert!(
            load_row.starts_with("│ "),
            "load row should be pushed right: {load_row:?}"
        );
    }

    #[test]
    fn shows_frequency_and_interval_on_the_border() {
        let app = ticked();
        let rendered = text(&draw(&app, 60, 12));
        let lines: Vec<&str> = rendered.lines().collect();
        if app.frequency_ghz.is_some() {
            assert!(lines[0].contains("GHz"), "frequency not on top border");
        }
        assert!(
            lines.last().is_some_and(|row| row.contains("2000ms")),
            "interval not on bottom border"
        );
    }

    #[test]
    fn hides_cores_that_do_not_fit_rather_than_clipping_the_load_row() {
        let app = ticked();
        // Borders + aggregate + load leaves one row of cores.
        let rendered = text(&draw(&app, 40, 5));
        assert!(rendered.contains("CPU"));
        assert!(rendered.contains("Load"), "load row must survive");
    }

    #[test]
    fn background_covers_every_cell_and_survives_the_widgets() {
        let app = ticked();
        let buf = draw(&app, 60, 12);
        let bad: Vec<(u16, u16, String)> = buf
            .content()
            .iter()
            .enumerate()
            .filter(|(_, cell)| cell.bg != theme::MAIN_BG)
            .map(|(i, cell)| {
                let w = buf.area.width as usize;
                ((i % w) as u16, (i / w) as u16, cell.symbol().to_string())
            })
            .collect();
        assert!(bad.is_empty(), "cells lost their background: {bad:?}");
    }

    #[test]
    fn background_reaches_past_the_box() {
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        super::super::render_background(area, &mut buf);
        assert_eq!(buf[(39, 9)].bg, theme::MAIN_BG, "far corner unpainted");
    }

    #[test]
    fn background_does_not_flatten_foreground_colors() {
        let app = ticked();
        let buf = draw(&app, 60, 12);
        let colors: std::collections::HashSet<_> =
            buf.content().iter().map(|cell| cell.fg).collect();
        assert!(colors.len() > 3, "bg pass must set bg only");
    }

    #[test]
    fn tiny_areas_do_not_panic() {
        let app = ticked();
        for width in 0..14 {
            for height in 0..6 {
                draw(&app, width, height);
            }
        }
    }

    #[test]
    fn missing_temperature_renders_a_placeholder() {
        let mut app = ticked();
        app.temperature = None;
        assert!(text(&draw(&app, 60, 12)).contains("--"));
    }

    #[test]
    fn battery_section_is_omitted_without_a_battery() {
        let mut app = ticked();
        app.battery = None;
        assert!(!text(&draw(&app, 60, 12)).contains("BAT"));
    }

    #[test]
    fn preferred_size_is_actually_renderable() {
        let app = ticked();
        let (width, height) = preferred_size(&app);
        let rendered = text(&draw(&app, width, height));
        assert!(rendered.contains("CPU"));
        assert!(rendered.contains("Load"));
        assert!(rendered.contains(&format!("C{}", app.core_count() - 1)));
    }
}
