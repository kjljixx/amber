//! Big pixel-style digits blended over half-block content.
//!
//! [`Digits`] is an ordinary Ratatui widget that knows nothing about gradients.
//! Render it after whatever painted the half-block cells (a `GradientView`, for
//! instance): it reads the two colors already in each `▀` cell and blends the
//! digits over them with [`blend_samples`](crate::blend_samples). Cells that are
//! not colored half blocks are left alone.
use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

use crate::{color::Rgb, overlay::blend_samples};

/// Text shrinks to leave this fraction of the width and height empty.
const MARGIN: f32 = 0.1;

// Everything below is in font pixels. A digit is 5 wide and 9 tall with
// 1-pixel strokes, followed by a 1-pixel gap; colons and dots are 1 wide.
const HEIGHT: i32 = 9;
const DIGIT_ADVANCE: f32 = 6.0;
const DOT_ADVANCE: f32 = 2.0;
const SPACE_ADVANCE: f32 = 3.0;
/// Empty space after the last glyph of a line, so centering ignores it.
const TRAILING_GAP: f32 = 1.0;
/// Top of one line to top of the next.
const LINE_PITCH: f32 = 12.0;

/// Rectangles `[left, top, right, bottom]` in font pixels, relative to the
/// glyph's top-left corner. Every digit touches all four sides of its 5 by 9
/// box, so none looks smaller than another. The shapes come from
/// `tools/digit-editor.html`, whose "Rust" export has this form.
fn glyph(character: char) -> Option<&'static [[i32; 4]]> {
    let rectangles: &'static [[i32; 4]] = match character {
        '0' => &[[1, 0, 4, 1], [0, 1, 1, 8], [4, 1, 5, 8], [1, 8, 4, 9]],
        '1' => &[[0, 0, 3, 1], [2, 1, 3, 8], [0, 8, 5, 9]],
        '2' => &[
            [1, 0, 4, 1],
            [0, 1, 1, 2],
            [4, 1, 5, 3],
            [3, 3, 5, 4],
            [2, 4, 4, 5],
            [1, 5, 3, 6],
            [0, 6, 2, 7],
            [0, 7, 1, 8],
            [0, 8, 5, 9],
        ],
        '3' => &[
            [0, 0, 5, 1],
            [4, 1, 5, 4],
            [1, 4, 5, 5],
            [4, 5, 5, 8],
            [0, 8, 5, 9],
        ],
        '4' => &[[0, 0, 1, 4], [4, 0, 5, 4], [0, 4, 5, 5], [4, 5, 5, 9]],
        '5' => &[
            [0, 0, 5, 1],
            [0, 1, 1, 3],
            [0, 3, 4, 4],
            [4, 4, 5, 8],
            [0, 7, 1, 8],
            [1, 8, 4, 9],
        ],
        '6' => &[
            [0, 0, 5, 1],
            [0, 1, 1, 4],
            [0, 4, 5, 5],
            [0, 5, 1, 8],
            [4, 5, 5, 8],
            [0, 8, 5, 9],
        ],
        '7' => &[
            [0, 0, 5, 1],
            [4, 1, 5, 3],
            [3, 3, 5, 4],
            [3, 4, 4, 5],
            [2, 5, 4, 6],
            [2, 6, 3, 9],
        ],
        '8' => &[
            [1, 0, 4, 1],
            [0, 1, 1, 4],
            [4, 1, 5, 4],
            [1, 4, 4, 5],
            [0, 5, 1, 8],
            [4, 5, 5, 8],
            [1, 8, 4, 9],
        ],
        '9' => &[
            [0, 0, 5, 1],
            [0, 1, 1, 4],
            [4, 1, 5, 4],
            [0, 4, 5, 5],
            [4, 5, 5, 8],
            [0, 8, 5, 9],
        ],
        ':' => &[[0, 2, 1, 3], [0, 6, 1, 7]],
        '.' => &[[0, 8, 1, 9]],
        '-' => &[[0, 4, 5, 5]],
        ' ' => &[],
        _ => return None,
    };
    Some(rectangles)
}

/// Distance from one glyph to the next, in font pixels.
fn advance(character: char) -> f32 {
    match character {
        ':' | '.' => DOT_ADVANCE,
        ' ' => SPACE_ADVANCE,
        _ => DIGIT_ADVANCE,
    }
}

/// Big digits over half-block cells: `0` to `9`, `:`, `.`, `-` and spaces,
/// with newlines for several lines.
///
/// The layer works on the half-block sample grid (two samples per terminal
/// row). Each pixel of the font becomes a square of `n` by `n` samples, where
/// `n` is the largest whole number that fits the area with a 10% margin, so
/// strokes are even and edges hard, and the size changes in steps. If even
/// `n = 1` does not fit, nothing is drawn. The text is centered. `alpha` 1 is
/// opaque; lower values let what is underneath show through, blended in OKLab
/// so its hue is kept. The coverage is computed once per area size.
///
/// Render it once per frame, after the content it covers: it blends over the
/// colors already in the buffer, so drawing it twice would blend twice.
#[derive(Clone, Debug)]
pub struct Digits {
    lines: Vec<String>,
    color: Rgb,
    alpha: f32,
    mask: Option<Mask>,
}

impl Digits {
    pub fn new(text: &str, color: Rgb, alpha: f32) -> Result<Self, &'static str> {
        let lines: Vec<String> = text
            .lines()
            .map(|line| line.trim_end().to_owned())
            .collect();
        if lines.iter().all(|line| line.is_empty()) {
            return Err("text must contain at least one visible character");
        }
        if lines
            .iter()
            .flat_map(|line| line.chars())
            .any(|character| glyph(character).is_none())
        {
            return Err("digits may only use 0-9, ':', '.', '-' and spaces");
        }
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err("alpha must be finite and between 0 and 1");
        }
        Ok(Self {
            lines,
            color,
            alpha,
            mask: None,
        })
    }

    /// The smallest rectangle of terminal cells that holds the digits when they
    /// are rendered into `area`, or `None` if nothing fits and nothing is drawn.
    /// A host can use it to place something next to the digits.
    pub fn bounds(&mut self, area: Rect) -> Option<Rect> {
        let rows = area.height.min(u16::MAX / 2);
        if area.width == 0 || rows == 0 {
            return None;
        }
        let mask = self.mask(area.width, 2 * rows);
        let (mut left, mut top, mut right, mut bottom) = (u16::MAX, u16::MAX, 0, 0);
        for row in 0..2 * rows {
            for column in 0..area.width {
                if mask.at(column, row) {
                    left = left.min(column);
                    right = right.max(column);
                    top = top.min(row);
                    bottom = bottom.max(row);
                }
            }
        }
        (left <= right).then(|| {
            Rect::new(
                area.x + left,
                area.y + top / 2,
                right - left + 1,
                bottom / 2 - top / 2 + 1,
            )
        })
    }

    /// The mask for a sample grid, rebuilt only when its size changes.
    fn mask(&mut self, width: u16, height: u16) -> &Mask {
        if self.mask.as_ref().map(Mask::size) != Some((width, height)) {
            self.mask = Some(self.layout(width, height));
        }
        self.mask.as_ref().unwrap()
    }

    fn layout(&self, width: u16, height: u16) -> Mask {
        let mut mask = Mask::new(width, height);
        if width == 0 || height == 0 {
            return mask;
        }
        // Sizes are in font pixels until multiplied by `n`.
        let line_pixels = |line: &str| line.chars().map(advance).sum::<f32>() - TRAILING_GAP;
        let widest = self
            .lines
            .iter()
            .map(|line| line_pixels(line))
            .fold(0.0, f32::max);
        let block = (self.lines.len() - 1) as f32 * LINE_PITCH + HEIGHT as f32;
        let fit = (f32::from(width) * (1.0 - 2.0 * MARGIN) / widest)
            .min(f32::from(height) * (1.0 - 2.0 * MARGIN) / block);
        if fit < 1.0 {
            return mask;
        }
        let n = fit.floor();
        let top = ((f32::from(height) - block * n) / 2.0).round();
        for (index, line) in self.lines.iter().enumerate() {
            let left = (f32::from(width) - line_pixels(line) * n) / 2.0;
            let line_top = (top + index as f32 * LINE_PITCH * n) as i32;
            let mut pen = 0.0;
            for character in line.chars() {
                // Snap each glyph to a whole sample; its pixels follow at
                // whole multiples of `n`.
                let origin = (left + pen * n).round() as i32;
                pen += advance(character);
                for &[x0, y0, x1, y1] in glyph(character).unwrap_or_default() {
                    let n = n as i32;
                    for y in line_top + y0 * n..line_top + y1 * n {
                        for x in origin + x0 * n..origin + x1 * n {
                            mask.set(x, y);
                        }
                    }
                }
            }
        }
        mask
    }
}

impl Widget for &mut Digits {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let area = area.intersection(buffer.area);
        let area = Rect {
            height: area.height.min(u16::MAX / 2),
            ..area
        };
        if area.is_empty() {
            return;
        }
        let (color, alpha) = (self.color, self.alpha);
        let mask = self.mask(area.width, 2 * area.height);
        blend_samples(buffer, area, color, alpha, |column, row| {
            mask.at(column, row)
        });
    }
}

/// Which samples of a `width` by `height` grid the digits cover.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Mask {
    width: u16,
    height: u16,
    covered: Vec<bool>,
}

impl Mask {
    fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            covered: vec![false; usize::from(width) * usize::from(height)],
        }
    }

    fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    fn at(&self, column: u16, row: u16) -> bool {
        self.covered[usize::from(row) * usize::from(self.width) + usize::from(column)]
    }

    /// Cover one sample; positions off the grid are ignored.
    fn set(&mut self, x: i32, y: i32) {
        if (0..i32::from(self.width)).contains(&x) && (0..i32::from(self.height)).contains(&y) {
            self.covered[y as usize * usize::from(self.width) + x as usize] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, style::Color};

    use super::*;
    use crate::{GradientView, presets};

    const WHITE: Rgb = [255, 255, 255];

    fn digits(text: &str) -> Digits {
        Digits::new(text, WHITE, 1.0).unwrap()
    }

    fn count(mask: &Mask) -> usize {
        mask.covered.iter().filter(|&&covered| covered).count()
    }

    /// Left, top, right and bottom of the covered samples.
    fn bounds(mask: &Mask) -> (u16, u16, u16, u16) {
        let mut found = (u16::MAX, u16::MAX, 0, 0);
        for row in 0..mask.height {
            for column in 0..mask.width {
                if mask.at(column, row) {
                    found = (
                        found.0.min(column),
                        found.1.min(row),
                        found.2.max(column),
                        found.3.max(row),
                    );
                }
            }
        }
        found
    }

    /// Lengths of every unbroken run of covered samples, along rows and columns.
    fn runs(mask: &Mask) -> Vec<usize> {
        let mut found = vec![];
        let mut scan = |line: &mut dyn Iterator<Item = bool>| {
            let mut length = 0;
            for covered in line.chain([false]) {
                if covered {
                    length += 1;
                } else if length > 0 {
                    found.push(length);
                    length = 0;
                }
            }
        };
        for row in 0..mask.height {
            scan(&mut (0..mask.width).map(|column| mask.at(column, row)));
        }
        for column in 0..mask.width {
            scan(&mut (0..mask.height).map(|row| mask.at(column, row)));
        }
        found
    }

    #[test]
    fn only_digits_and_clock_punctuation_are_accepted() {
        assert!(Digits::new("12:34.5 -6\n7", WHITE, 1.0).is_ok());
        for text in ["", " \n ", "A", "12h", "1/2", "é"] {
            assert!(Digits::new(text, WHITE, 1.0).is_err(), "{text:?}");
        }
        for alpha in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            assert!(Digits::new("1", WHITE, alpha).is_err(), "{alpha}");
        }
    }

    #[test]
    fn text_is_fitted_and_centered_inside_the_margins() {
        let mut clock = digits("25:49");
        let mask = clock.mask(120, 80);
        let (left, top, right, bottom) = bounds(mask);
        assert!(left >= 12 && right <= 108 && top >= 8 && bottom <= 72);
        assert!((i32::from(left) + i32::from(right) - 119).abs() <= 2);
        assert!((i32::from(top) + i32::from(bottom) - 79).abs() <= 2);
    }

    #[test]
    fn pixels_are_whole_squares_so_strokes_are_even() {
        for (text, (width, height)) in [("8", (120, 80)), ("25:49", (120, 40)), ("0123", (90, 60))]
        {
            let mut layer = digits(text);
            let mask = layer.mask(width, height);
            // Digits are 9 font pixels tall, so the height gives the scale.
            let (_, top, _, bottom) = bounds(mask);
            let rows = usize::from(bottom - top + 1);
            assert_eq!(rows % 9, 0, "{text}");
            let n = rows / 9;
            for length in runs(mask) {
                assert_eq!(length % n, 0, "{text}: run of {length} at scale {n}");
            }
        }
    }

    #[test]
    fn the_scale_steps_in_whole_samples_as_the_area_grows() {
        let rows = |height| {
            let mut layer = digits("12");
            let (_, top, _, bottom) = bounds(layer.mask(300, height));
            usize::from(bottom - top + 1)
        };
        let all: Vec<usize> = [40, 50, 60, 80, 100, 160].map(rows).to_vec();
        assert!(all.iter().all(|rows| rows % 9 == 0), "{all:?}");
        assert!(all.windows(2).all(|pair| pair[0] <= pair[1]), "{all:?}");
        assert!(all[0] < all[5], "{all:?}");
    }

    #[test]
    fn every_digit_looks_different_and_fills_the_same_box() {
        let masks: Vec<(char, Mask)> = "0123456789"
            .chars()
            .map(|digit| (digit, digits(&digit.to_string()).mask(60, 40).clone()))
            .collect();
        for (i, (a, first)) in masks.iter().enumerate() {
            for (b, second) in &masks[i + 1..] {
                assert_ne!(first, second, "{a} and {b} are identical");
            }
        }
        // No digit looks smaller than another: all touch the same four sides.
        let first = bounds(&masks[0].1);
        for (digit, mask) in &masks {
            assert_eq!(bounds(mask), first, "{digit}");
        }
    }

    #[test]
    fn the_eight_is_notched_at_every_outer_corner() {
        // At scale 1 a digit is 5 by 9 samples.
        let mut eight = digits("8");
        let mask = eight.mask(15, 12).clone();
        let (left, top, right, bottom) = bounds(&mask);
        assert_eq!((right - left, bottom - top), (4, 8));
        for (x, y) in [(left, top), (right, top), (left, bottom), (right, bottom)] {
            assert!(!mask.at(x, y), "corner ({x}, {y})");
        }
        // The upper and lower bowls are closed.
        for (y0, y1) in [(top, top + 4), (top + 4, bottom)] {
            assert!((y0 + 1..y1).all(|row| mask.at(left, row) && mask.at(right, row)));
        }
    }

    #[test]
    fn digits_share_one_advance_so_clocks_do_not_shift() {
        let width = |text: &str| {
            let mut layer = digits(text);
            let (left, _, right, _) = bounds(layer.mask(300, 40));
            right - left + 1
        };
        // Each extra digit adds 6 font pixels; the scale is `n` samples each.
        let one = width("8");
        let n = one / 5;
        assert_eq!(width("88"), one + 6 * n);
        assert_eq!(width("8888"), one + 18 * n);
        // Every digit fills its cell, so "11" and "88" cover the same span.
        let (mut eights, mut ones) = (digits("88"), digits("11"));
        assert_eq!(bounds(eights.mask(300, 40)), bounds(ones.mask(300, 40)));
    }

    #[test]
    fn colon_is_two_square_dots_and_dot_is_one() {
        let mut colon = digits(":");
        let mask = colon.mask(120, 80);
        // The dots are at font-pixel rows 2 and 6: 5 pixels apart, 1 tall.
        let (_, top, _, bottom) = bounds(mask);
        let n = usize::from(bottom - top + 1) / 5;
        assert_eq!(count(mask), 2 * n * n);
        assert_eq!(count(digits(".").mask(120, 80)), n * n);
    }

    #[test]
    fn spaces_advance_the_pen_and_lines_stack() {
        let width = |text: &str| {
            let mut layer = digits(text);
            let (left, _, right, _) = bounds(layer.mask(300, 40));
            right - left + 1
        };
        // A space is 3 font pixels, so "1 1" is 3n wider than "11".
        let n = width("8") / 5;
        assert_eq!(width("1 1"), width("11") + 3 * n);

        let mut two = digits("12\n34");
        let mask = two.mask(120, 80);
        let (_, top, _, bottom) = bounds(mask);
        // Both lines show, with blank rows between them.
        assert!(bottom - top > 40);
        let middle = (top + bottom) / 2;
        assert!((0..120).all(|column| !mask.at(column, middle)));
    }

    #[test]
    fn too_small_areas_draw_nothing_and_never_panic() {
        let mut clock = digits("25:49");
        for (width, height) in [(0, 0), (0, 10), (10, 0), (1, 1), (20, 8), (30, 10)] {
            assert_eq!(count(clock.mask(width, height)), 0, "{width}x{height}");
        }
        assert!(count(clock.mask(120, 80)) > 0);
        // Just enough for one sample per font pixel with the margin: 32 columns
        // by 6 terminal rows (12 sample rows) for "25:49".
        assert!(count(clock.mask(32, 12)) > 0);
        assert_eq!(count(clock.mask(31, 12)), 0);
        assert_eq!(count(clock.mask(32, 11)), 0);
        // The mask is rebuilt only when the size changes.
        let first = clock.mask(80, 40).clone();
        assert_eq!(clock.mask(80, 40), &first);
        assert_ne!(clock.mask(100, 50).size(), first.size());
    }

    // ---- as a widget over a gradient ----

    fn gradient() -> GradientView {
        GradientView::new(&presets::presets()[1].recipe, std::time::Instant::now()).unwrap()
    }

    fn draw(gradient: &mut GradientView, digits: Option<&mut Digits>, size: (u16, u16)) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(size.0, size.1)).unwrap();
        terminal
            .draw(|frame| {
                gradient.render(frame, frame.area());
                if let Some(digits) = digits {
                    frame.render_widget(digits, frame.area());
                }
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn opaque_digits_cover_the_gradient_and_leave_the_rest_alone() {
        let mut view = gradient();
        let plain = draw(&mut view, None, (80, 24));
        let magenta = [255, 0, 255];
        let mut text = Digits::new("25:49", magenta, 1.0).unwrap();
        let covered = draw(&mut view, Some(&mut text), (80, 24));

        let painted = covered
            .content
            .iter()
            .filter(|cell| cell.fg == Color::Rgb(255, 0, 255) || cell.bg == Color::Rgb(255, 0, 255))
            .count();
        assert!(painted > 50, "{painted}");
        // Rows above and below the digits are untouched, and so are cells the
        // digits do not cover: the symbol never changes.
        for column in 0..80 {
            assert_eq!(plain[(column, 0)], covered[(column, 0)]);
            assert_eq!(plain[(column, 23)], covered[(column, 23)]);
        }
        assert!(covered.content.iter().all(|cell| cell.symbol() == "▀"));
    }

    #[test]
    fn translucent_digits_blend_and_zero_alpha_changes_nothing() {
        let mut view = gradient();
        let plain = draw(&mut view, None, (80, 24));
        let mut invisible = Digits::new("25:49", WHITE, 0.0).unwrap();
        assert_eq!(draw(&mut view, Some(&mut invisible), (80, 24)), plain);

        let mut half = Digits::new("25:49", WHITE, 0.5).unwrap();
        let mut opaque = digits("25:49");
        let (half, opaque) = (
            draw(&mut view, Some(&mut half), (80, 24)),
            draw(&mut view, Some(&mut opaque), (80, 24)),
        );
        // Some cell is lighter than the gradient and darker than opaque white.
        let blended = (0..24)
            .flat_map(|y| (0..80).map(move |x| (x, y)))
            .any(|(x, y)| {
                let (Color::Rgb(a, ..), Color::Rgb(b, ..), Color::Rgb(c, ..)) =
                    (plain[(x, y)].fg, half[(x, y)].fg, opaque[(x, y)].fg)
                else {
                    return false;
                };
                c == 255 && a < b && b < c
            });
        assert!(blended);
    }

    #[test]
    fn only_colored_half_blocks_are_blended() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 24));
        let blank = buffer.clone();
        (&mut digits("25:49")).render(buffer.area, &mut buffer);
        assert_eq!(buffer, blank);
    }

    #[test]
    fn it_follows_resizes_offsets_and_empty_areas() {
        let mut view = gradient();
        let mut text = digits("25:49");
        for size in [(80, 24), (120, 40), (10, 4), (1, 1)] {
            draw(&mut view, Some(&mut text), size);
        }
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 10));
        for area in [
            Rect::new(0, 0, 0, 0),
            Rect::new(3, 2, 0, 5),
            Rect::new(15, 5, 40, 40),
        ] {
            (&mut text).render(area, &mut buffer);
        }
    }

    #[test]
    fn bounds_are_the_cells_the_digits_touch() {
        // Cells that changed when the digits were drawn over flat half blocks.
        let changed_cells = |digits: &mut Digits, area: Rect| {
            let flat = |buffer: &mut Buffer| {
                for cell in &mut buffer.content {
                    cell.set_char('▀')
                        .set_fg(Color::Rgb(40, 60, 90))
                        .set_bg(Color::Rgb(40, 60, 90));
                }
            };
            let mut plain = Buffer::empty(area);
            flat(&mut plain);
            let mut drawn = plain.clone();
            Widget::render(digits, area, &mut drawn);
            let mut cells = Vec::new();
            for y in area.top()..area.bottom() {
                for x in area.left()..area.right() {
                    if plain[(x, y)] != drawn[(x, y)] {
                        cells.push((x, y));
                    }
                }
            }
            cells
        };

        for (text, area) in [
            ("25:49", Rect::new(0, 0, 120, 40)),
            ("1", Rect::new(0, 0, 80, 24)),
            ("12\n34", Rect::new(0, 0, 100, 30)),
            ("25:49", Rect::new(10, 5, 120, 40)),
        ] {
            let mut layer = digits(text);
            let bounds = layer.bounds(area).unwrap();
            let cells = changed_cells(&mut layer, area);
            // The bounds are exactly the extent of what was drawn.
            let extent = |pick: fn(&(u16, u16)) -> u16| {
                let values: Vec<u16> = cells.iter().map(pick).collect();
                (*values.iter().min().unwrap(), *values.iter().max().unwrap())
            };
            assert_eq!(
                extent(|c| c.0),
                (bounds.left(), bounds.right() - 1),
                "{text}"
            );
            assert_eq!(
                extent(|c| c.1),
                (bounds.top(), bounds.bottom() - 1),
                "{text}"
            );
        }

        // Too small an area draws nothing, and so has no bounds.
        let mut clock = digits("25:49");
        assert_eq!(clock.bounds(Rect::new(0, 0, 20, 8)), None);
        assert_eq!(clock.bounds(Rect::new(0, 0, 0, 0)), None);
    }
}
