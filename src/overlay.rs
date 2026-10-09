//! Blending a flat color over half-block cells.
use ratatui::{buffer::Buffer, layout::Rect, style::Color};

use crate::color::{Oklab, Rgb, oklab_to_rgb, rgb_to_oklab};

/// Blend `color` at `alpha` (0 to 1) over the samples that `covered` selects.
///
/// `area` is a grid of two samples per terminal row: `covered(column, row)` is
/// asked about every sample with `column` in `0..area.width` and `row` in
/// `0..2 * area.height`, both relative to the area's top-left corner. A cell's
/// foreground is its top sample and its background its bottom sample, so only
/// cells that are colored half blocks (`▀`, as `GradientView` paints them) are
/// touched; anything else is left alone. Blending is done in OKLab, so the hue
/// underneath is kept.
///
/// This works on the colors already in `buffer`, so call it once per sample
/// after the content it covers has been drawn. Blending two sets of samples
/// that overlap would blend those samples twice.
pub fn blend_samples(
    buffer: &mut Buffer,
    area: Rect,
    color: Rgb,
    alpha: f32,
    covered: impl Fn(u16, u16) -> bool,
) {
    let area = area.intersection(buffer.area);
    let rows = area.height.min(u16::MAX / 2);
    if area.width == 0 || rows == 0 || alpha.is_nan() {
        return;
    }
    let alpha = alpha.clamp(0.0, 1.0);
    let text = rgb_to_oklab(color);
    for row in 0..rows {
        for column in 0..area.width {
            let (top, bottom) = (covered(column, 2 * row), covered(column, 2 * row + 1));
            if !top && !bottom {
                continue;
            }
            let cell = &mut buffer[(area.x + column, area.y + row)];
            let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (cell.fg, cell.bg) else {
                continue;
            };
            if cell.symbol() != "▀" {
                continue;
            }
            let over = |selected: bool, below: Rgb| {
                if selected {
                    blend(below, text, color, alpha)
                } else {
                    below
                }
            };
            let [r, g, b] = over(top, [r1, g1, b1]);
            cell.set_fg(Color::Rgb(r, g, b));
            let [r, g, b] = over(bottom, [r2, g2, b2]);
            cell.set_bg(Color::Rgb(r, g, b));
        }
    }
}

/// `color` over `below` at `alpha`, mixed in OKLab. `text` is `color` in OKLab.
fn blend(below: Rgb, text: Oklab, color: Rgb, alpha: f32) -> Rgb {
    if alpha >= 1.0 {
        color
    } else if alpha <= 0.0 {
        below
    } else {
        let below = rgb_to_oklab(below);
        oklab_to_rgb([0, 1, 2].map(|i| below[i] + (text[i] - below[i]) * alpha))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREY: Rgb = [100, 100, 100];

    fn half_blocks(area: Rect) -> Buffer {
        let mut buffer = Buffer::empty(area);
        for cell in &mut buffer.content {
            cell.set_char('▀')
                .set_fg(Color::Rgb(100, 100, 100))
                .set_bg(Color::Rgb(100, 100, 100));
        }
        buffer
    }

    #[test]
    fn selects_top_and_bottom_samples_independently() {
        let area = Rect::new(0, 0, 3, 2);
        let mut buffer = half_blocks(area);
        // Sample (column 1, row 2) is the top half of terminal row 1; sample
        // (column 2, row 1) is the bottom half of terminal row 0.
        blend_samples(&mut buffer, area, [255, 0, 0], 1.0, |column, row| {
            (column, row) == (1, 2) || (column, row) == (2, 1)
        });
        let red = Color::Rgb(255, 0, 0);
        let grey = Color::Rgb(100, 100, 100);
        assert_eq!((buffer[(1, 1)].fg, buffer[(1, 1)].bg), (red, grey));
        assert_eq!((buffer[(2, 0)].fg, buffer[(2, 0)].bg), (grey, red));
        let changed = buffer
            .content
            .iter()
            .filter(|c| c.fg != grey || c.bg != grey);
        assert_eq!(changed.count(), 2);
    }

    #[test]
    fn alpha_mixes_toward_the_color_and_zero_changes_nothing() {
        let area = Rect::new(0, 0, 1, 1);
        let blended = |alpha| {
            let mut buffer = half_blocks(area);
            blend_samples(&mut buffer, area, [255, 255, 255], alpha, |_, _| true);
            buffer[(0, 0)].fg
        };
        assert_eq!(blended(0.0), Color::Rgb(100, 100, 100));
        assert_eq!(blended(1.0), Color::Rgb(255, 255, 255));
        let Color::Rgb(r, g, b) = blended(0.5) else {
            panic!()
        };
        assert!(r > 100 && r < 255 && g > 100 && g < 255 && b > 100 && b < 255);
        // Out-of-range alpha is clamped, and NaN draws nothing.
        assert_eq!(blended(7.0), Color::Rgb(255, 255, 255));
        assert_eq!(blended(f32::NAN), Color::Rgb(100, 100, 100));
    }

    #[test]
    fn coordinates_are_relative_to_an_offset_area_and_clipped_to_the_buffer() {
        let mut buffer = half_blocks(Rect::new(0, 0, 10, 6));
        let seen = std::cell::RefCell::new(Vec::new());
        blend_samples(
            &mut buffer,
            Rect::new(4, 2, 3, 2),
            GREY,
            1.0,
            |column, row| {
                seen.borrow_mut().push((column, row));
                false
            },
        );
        let seen = seen.into_inner();
        assert_eq!(seen.iter().map(|s| s.0).max(), Some(2));
        assert_eq!(seen.iter().map(|s| s.1).max(), Some(3));
        assert_eq!(seen.len(), 3 * 4);

        // An area hanging off the buffer only asks about what fits.
        let asked = std::cell::Cell::new(0);
        blend_samples(&mut buffer, Rect::new(8, 4, 10, 10), GREY, 1.0, |_, _| {
            asked.set(asked.get() + 1);
            false
        });
        assert_eq!(asked.get(), 2 * 2 * 2);
    }

    #[test]
    fn cells_that_are_not_colored_half_blocks_are_left_alone() {
        let area = Rect::new(0, 0, 2, 1);
        let mut buffer = half_blocks(area);
        buffer[(0, 0)].set_char('x');
        buffer[(1, 0)].set_fg(Color::Reset);
        let before = buffer.clone();
        blend_samples(&mut buffer, area, [255, 255, 255], 1.0, |_, _| true);
        assert_eq!(buffer, before);
    }
}
