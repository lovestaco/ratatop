//! The small gradient bar btop uses for the battery.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;

use crate::theme::{self, Gradient};

/// Segmented glyphs — reads well for a short meter like the battery.
const SEGMENT: &str = "▰";
const EMPTY_SEGMENT: &str = "▱";
/// Solid glyphs — btop's wide CPU bar, which reads as one continuous block.
const SOLID: &str = "█";

/// A fixed-width bar whose filled segments follow a gradient across its length.
pub struct Meter {
    percent: f64,
    gradient: Gradient,
    solid: bool,
}

impl Meter {
    pub fn new(percent: f64, gradient: Gradient) -> Self {
        Self {
            percent,
            gradient,
            solid: false,
        }
    }

    /// Draw as one continuous block instead of discrete segments.
    pub fn solid(mut self) -> Self {
        self.solid = true;
        self
    }

    const fn symbols(&self) -> (&'static str, &'static str) {
        if self.solid {
            (SOLID, SOLID)
        } else {
            (SEGMENT, EMPTY_SEGMENT)
        }
    }
}

impl Widget for Meter {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        let width = area.width as usize;
        let filled = ((self.percent.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize;
        let (fill_symbol, empty_symbol) = self.symbols();

        for x in 0..width {
            // Each segment is colored by its own position along the bar, so the
            // ramp stays put as the value moves.
            let position = if width == 1 {
                1.0
            } else {
                x as f64 / (width - 1) as f64
            };
            let (symbol, style) = if x < filled {
                (
                    fill_symbol,
                    Style::default().fg(self.gradient.sample(position)),
                )
            } else {
                (empty_symbol, Style::default().fg(theme::INACTIVE_FG))
            };
            buf[(area.x + x as u16, area.y)]
                .set_symbol(symbol)
                .set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(percent: f64, width: u16) -> Buffer {
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        Meter::new(percent, theme::BATTERY).render(area, &mut buf);
        buf
    }

    fn symbols(buf: &Buffer) -> String {
        buf.content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn fills_proportionally() {
        assert_eq!(symbols(&render(100.0, 4)), "▰▰▰▰");
        assert_eq!(symbols(&render(50.0, 4)), "▰▰▱▱");
        assert_eq!(symbols(&render(0.0, 4)), "▱▱▱▱");
    }

    #[test]
    fn clamps_out_of_range_values() {
        assert_eq!(symbols(&render(150.0, 3)), "▰▰▰");
        assert_eq!(symbols(&render(-10.0, 3)), "▱▱▱");
    }

    #[test]
    fn gradient_runs_across_the_bar_not_with_the_value() {
        let full = render(100.0, 5);
        assert_eq!(full[(0, 0)].fg, theme::BATTERY.sample(0.0));
        assert_eq!(full[(4, 0)].fg, theme::BATTERY.sample(1.0));
        // The same cell keeps its color when the bar is only partly filled.
        assert_eq!(render(40.0, 5)[(0, 0)].fg, full[(0, 0)].fg);
    }

    #[test]
    fn unfilled_segments_are_inactive() {
        assert_eq!(render(0.0, 2)[(0, 0)].fg, theme::INACTIVE_FG);
    }

    #[test]
    fn solid_style_uses_one_glyph_and_distinguishes_by_color() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        Meter::new(30.0, theme::CPU).solid().render(area, &mut buf);

        assert_eq!(
            symbols(&buf),
            "██████████",
            "solid bar is one continuous glyph"
        );
        // Only the color marks the fill level, so that is what has to be right.
        let filled: Vec<_> = (0..10)
            .filter(|&x| buf[(x, 0)].fg != theme::INACTIVE_FG)
            .collect();
        assert_eq!(filled, vec![0, 1, 2], "30% of ten cells");
    }

    #[test]
    fn solid_style_is_empty_at_zero() {
        let area = Rect::new(0, 0, 6, 1);
        let mut buf = Buffer::empty(area);
        Meter::new(0.0, theme::CPU).solid().render(area, &mut buf);
        for x in 0..6 {
            assert_eq!(
                buf[(x, 0)].fg,
                theme::INACTIVE_FG,
                "an idle bar must read as empty despite the solid glyph"
            );
        }
    }

    #[test]
    fn single_cell_and_empty_areas_are_safe() {
        assert_eq!(symbols(&render(100.0, 1)), "▰");
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        Meter::new(50.0, theme::BATTERY).render(Rect::new(0, 0, 0, 0), &mut buf);
    }
}
