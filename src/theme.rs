//! Colors and gradients, ported from btop's default theme.
//!
//! btop stores three stops per gradient (`*_start`, `*_mid`, `*_end`) and expands
//! them into a 101-step ramp at startup — see `btop/src/btop_theme.cpp`. We do the
//! interpolation on demand instead, which is cheap enough at our redraw rates.

use ratatui::style::Color;

/// A three-stop color ramp, sampled by a 0.0..=1.0 position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gradient {
    pub start: (u8, u8, u8),
    pub mid: (u8, u8, u8),
    pub end: (u8, u8, u8),
}

impl Gradient {
    pub const fn new(start: (u8, u8, u8), mid: (u8, u8, u8), end: (u8, u8, u8)) -> Self {
        Self { start, mid, end }
    }

    /// Sample the ramp. `t` is clamped to 0.0..=1.0; 0.5 lands exactly on `mid`.
    pub fn sample(&self, t: f64) -> Color {
        let t = t.clamp(0.0, 1.0);
        let (from, to, local) = if t < 0.5 {
            (self.start, self.mid, t * 2.0)
        } else {
            (self.mid, self.end, (t - 0.5) * 2.0)
        };
        Color::Rgb(
            lerp(from.0, to.0, local),
            lerp(from.1, to.1, local),
            lerp(from.2, to.2, local),
        )
    }
}

fn lerp(a: u8, b: u8, t: f64) -> u8 {
    (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8
}

/// btop's `cpu_start` / `cpu_mid` / `cpu_end` — green through amber to red.
pub const CPU: Gradient = Gradient::new((0x77, 0xca, 0x9b), (0xcb, 0xc0, 0x6c), (0xdc, 0x4c, 0x4c));

/// btop's `temp_start` / `temp_mid` / `temp_end` — blue through indigo to pink.
pub const TEMP: Gradient =
    Gradient::new((0x48, 0x97, 0xd4), (0x54, 0x74, 0xe8), (0xff, 0x40, 0xb6));

/// btop's memory ramps. Each field gets its own so the box reads at a glance.
pub const MEM_USED: Gradient =
    Gradient::new((0x59, 0x2b, 0x26), (0xd9, 0x62, 0x6d), (0xff, 0x47, 0x69));
pub const MEM_AVAILABLE: Gradient =
    Gradient::new((0x4e, 0x3f, 0x0e), (0xff, 0xd7, 0x7a), (0xff, 0xb8, 0x14));
pub const MEM_CACHED: Gradient =
    Gradient::new((0x16, 0x33, 0x50), (0x74, 0xe6, 0xfc), (0x26, 0xc5, 0xff));
pub const MEM_FREE: Gradient =
    Gradient::new((0x38, 0x4f, 0x21), (0xb5, 0xe6, 0x85), (0xdc, 0xff, 0x85));

/// Battery reuses the CPU ramp in btop, but inverted: full is green, empty is red.
pub const BATTERY: Gradient =
    Gradient::new((0xdc, 0x4c, 0x4c), (0xcb, 0xc0, 0x6c), (0x77, 0xca, 0x9b));

/// `main_bg` — btop paints the whole terminal rather than inheriting its colors.
pub const MAIN_BG: Color = Color::Rgb(0x00, 0x00, 0x00);
/// `main_fg` — ordinary text.
pub const MAIN_FG: Color = Color::Rgb(0xcc, 0xcc, 0xcc);
/// `title` — box titles and labels.
pub const TITLE: Color = Color::Rgb(0xee, 0xee, 0xee);
/// `hi_fg` — the highlighted key character in a label.
pub const HI_FG: Color = Color::Rgb(0xb5, 0x40, 0x40);
/// `div_line` — box borders.
pub const DIV_LINE: Color = Color::Rgb(0x30, 0x30, 0x30);
/// `selected_bg` — the highlighted row in the process table.
pub const SELECTED_BG: Color = Color::Rgb(0x2a, 0x2a, 0x3a);
/// `inactive_fg` — the unfilled portion of a meter.
pub const INACTIVE_FG: Color = Color::Rgb(0x40, 0x40, 0x40);

/// Temperature as a gradient position, using btop's 0..=`crit` scale.
///
/// btop treats the sensor's own critical trip point as the top of the scale and
/// falls back to 80 °C when the kernel does not expose one.
pub fn temp_position(celsius: f64, critical: f64) -> f64 {
    if critical <= 0.0 {
        return 0.0;
    }
    (celsius / critical).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_hits_its_stops_exactly() {
        assert_eq!(CPU.sample(0.0), Color::Rgb(0x77, 0xca, 0x9b));
        assert_eq!(CPU.sample(0.5), Color::Rgb(0xcb, 0xc0, 0x6c));
        assert_eq!(CPU.sample(1.0), Color::Rgb(0xdc, 0x4c, 0x4c));
    }

    #[test]
    fn gradient_clamps_out_of_range_positions() {
        assert_eq!(CPU.sample(-5.0), CPU.sample(0.0));
        assert_eq!(CPU.sample(12.0), CPU.sample(1.0));
    }

    #[test]
    fn gradient_interpolates_between_stops() {
        // Quarter way is halfway between start and mid.
        let Color::Rgb(r, g, b) = CPU.sample(0.25) else {
            panic!("expected an rgb color");
        };
        assert_eq!((r, g, b), (0xa1, 0xc5, 0x84));
    }

    #[test]
    fn temp_position_scales_against_critical() {
        assert!((temp_position(40.0, 80.0) - 0.5).abs() < f64::EPSILON);
        assert_eq!(temp_position(200.0, 80.0), 1.0);
        assert_eq!(temp_position(50.0, 0.0), 0.0, "no critical point known");
    }
}
