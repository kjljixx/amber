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
    if alpha.is_nan() {
        return;
    }
    let alpha = alpha.clamp(0.0, 1.0);
    let text = rgb_to_oklab(color);
    recolor_samples(
        buffer,
        area,
        |column, row| covered(column, row).then_some(()),
        |(), below| blend(below, text, color, alpha),
    );
}

/// Darken the samples near whatever `covered` selects, only as far as needed
/// to keep a translucent light text readable over them.
///
/// A sample whose OKLab lightness is above `limit` (0 to 1) is darkened toward
/// black until it is at `limit`; darker samples are left alone. How much of
/// that darkening is applied falls off smoothly with the distance to the
/// nearest covered sample: all of it on the text, none `radius` samples away.
/// Returns how many samples were darkened.
///
/// Call it before the text is blended, so it works on the gradient's own
/// colors. `area` and `covered` are as for [`blend_samples`].
pub fn darken_around(
    buffer: &mut Buffer,
    area: Rect,
    radius: u16,
    limit: f32,
    covered: impl Fn(u16, u16) -> bool,
) -> usize {
    let area = area.intersection(buffer.area);
    let (width, height) = (
        usize::from(area.width),
        usize::from(area.height.min(u16::MAX / 2)) * 2,
    );
    if width == 0 || height == 0 || limit.is_nan() {
        return 0;
    }
    let limit = limit.clamp(0.0, 1.0);
    let reach = usize::from(radius);
    let falloff: Vec<f32> = (0..=reach)
        .map(|distance| {
            0.5 + 0.5 * (std::f32::consts::PI * distance as f32 / (reach + 1) as f32).cos()
        })
        .collect();
    let mut weight: Vec<f32> = (0..height)
        .flat_map(|row| (0..width).map(move |column| (column, row)))
        .map(|(column, row)| f32::from(covered(column as u16, row as u16)))
        .collect();
    // The strongest nearby cover, seen through the falloff, one axis at a time.
    for (length, stride, lines, line_stride) in
        [(width, 1, height, width), (height, width, width, 1)]
    {
        let source = weight.clone();
        for line in 0..lines {
            for position in 0..length {
                let nearest = position.saturating_sub(reach)..(position + reach + 1).min(length);
                weight[line * line_stride + position * stride] = nearest
                    .map(|other| {
                        source[line * line_stride + other * stride]
                            * falloff[position.abs_diff(other)]
                    })
                    .fold(0.0, f32::max);
            }
        }
    }
    let mut darkened = 0;
    recolor_samples(
        buffer,
        area,
        |column, row| {
            let weight = weight[usize::from(row) * width + usize::from(column)];
            (weight > 0.0).then_some(weight)
        },
        |weight, below| {
            let [lightness, a, b] = rgb_to_oklab(below);
            if lightness <= limit {
                return below;
            }
            darkened += 1;
            let scale = 1.0 - weight * (1.0 - limit / lightness);
            oklab_to_rgb([lightness * scale, a * scale, b * scale])
        },
    );
    darkened
}

/// Recolor the samples that `select` gives a value for, with `recolor` told
/// that value and the sample's current color. Samples are as described for
/// [`blend_samples`]: only colored half blocks are touched.
fn recolor_samples<T: Copy>(
    buffer: &mut Buffer,
    area: Rect,
    select: impl Fn(u16, u16) -> Option<T>,
    mut recolor: impl FnMut(T, Rgb) -> Rgb,
) {
    let area = area.intersection(buffer.area);
    let rows = area.height.min(u16::MAX / 2);
    if area.width == 0 || rows == 0 {
        return;
    }
    for row in 0..rows {
        for column in 0..area.width {
            let (top, bottom) = (select(column, 2 * row), select(column, 2 * row + 1));
            if top.is_none() && bottom.is_none() {
                continue;
            }
            let cell = &mut buffer[(area.x + column, area.y + row)];
            let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (cell.fg, cell.bg) else {
                continue;
            };
            if cell.symbol() != "▀" {
                continue;
            }
            let [r, g, b] = top.map_or([r1, g1, b1], |value| recolor(value, [r1, g1, b1]));
            cell.set_fg(Color::Rgb(r, g, b));
            let [r, g, b] = bottom.map_or([r2, g2, b2], |value| recolor(value, [r2, g2, b2]));
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

    fn lightness(color: Color) -> f32 {
        let Color::Rgb(r, g, b) = color else { panic!() };
        rgb_to_oklab([r, g, b])[0]
    }

    fn filled(area: Rect, color: Rgb) -> Buffer {
        let mut buffer = Buffer::empty(area);
        let color = Color::Rgb(color[0], color[1], color[2]);
        for cell in &mut buffer.content {
            cell.set_char('▀').set_fg(color).set_bg(color);
        }
        buffer
    }

    #[test]
    fn pale_samples_under_the_text_are_darkened_to_the_limit() {
        let area = Rect::new(0, 0, 40, 20);
        let mut buffer = filled(area, [250, 245, 235]);
        let darkened = darken_around(&mut buffer, area, 6, 0.6, |column, row| {
            (18..22).contains(&column) && (18..22).contains(&row)
        });
        assert!(darkened > 0);
        assert!((lightness(buffer[(20, 10)].fg) - 0.6).abs() < 0.02);
        assert!((lightness(buffer[(20, 10)].bg) - 0.6).abs() < 0.02);
    }

    #[test]
    fn the_darkening_fades_out_with_distance_and_stops_at_the_radius() {
        let area = Rect::new(0, 0, 60, 30);
        let mut buffer = filled(area, [250, 245, 235]);
        darken_around(&mut buffer, area, 6, 0.6, |column, row| {
            (column, row) == (30, 30)
        });
        let pale = lightness(Color::Rgb(250, 245, 235));
        let at = |distance: u16| lightness(buffer[(30 + distance, 15)].bg);
        assert!(at(0) < at(2) && at(2) < at(4) && at(4) < pale);
        assert!((at(7) - pale).abs() < 0.01);
        assert!((lightness(buffer[(30, 6)].fg) - pale).abs() < 0.01);
    }

    #[test]
    fn dark_samples_are_left_alone() {
        let area = Rect::new(0, 0, 20, 10);
        let mut buffer = filled(area, [60, 70, 120]);
        let before = buffer.clone();
        let darkened = darken_around(&mut buffer, area, 6, 0.6, |_, _| true);
        assert_eq!(darkened, 0);
        assert_eq!(buffer, before);
    }

    #[test]
    fn nothing_is_darkened_without_text() {
        let area = Rect::new(0, 0, 20, 10);
        let mut buffer = filled(area, [255, 255, 255]);
        let before = buffer.clone();
        assert_eq!(darken_around(&mut buffer, area, 6, 0.6, |_, _| false), 0);
        assert_eq!(buffer, before);
    }
}
