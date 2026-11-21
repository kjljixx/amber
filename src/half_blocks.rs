use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};

use crate::color::{Rgb, rgb_to_oklab};

/// Samples on a grid that starts at pixel column `origin`, with `step` pixels
/// between nodes.
struct CoarseGrid {
    origin: usize,
    step: usize,
    columns: usize,
    nodes: Vec<Rgb>,
}

impl CoarseGrid {
    fn new(
        origin: usize,
        step: usize,
        width: usize,
        height: usize,
        sample: impl Fn(usize, usize) -> Rgb,
    ) -> Self {
        let columns = (width - origin - 1) / step + 2;
        let rows = (height - 1) / step + 2;
        let mut nodes = Vec::with_capacity(columns * rows);
        for row in 0..rows {
            for column in 0..columns {
                nodes.push(sample(origin + column * step, row * step));
            }
        }
        Self {
            origin,
            step,
            columns,
            nodes,
        }
    }

    fn at(&self, x: usize, y: usize) -> Rgb {
        let across = x - self.origin;
        let (column, row) = (across / self.step, y / self.step);
        let along_x = (across % self.step) as f32 / self.step as f32;
        let along_y = (y % self.step) as f32 / self.step as f32;
        let node = |column: usize, row: usize| self.nodes[row * self.columns + column];
        let corners = [
            node(column, row),
            node(column + 1, row),
            node(column, row + 1),
            node(column + 1, row + 1),
        ];
        std::array::from_fn(|channel| {
            let value = |corner: usize| f32::from(corners[corner][channel]);
            let top = lerp(value(0), value(1), along_x);
            let bottom = lerp(value(2), value(3), along_x);
            lerp(top, bottom, along_y).round() as u8
        })
    }
}

fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    from + (to - from) * amount
}

#[derive(Clone, Copy, Default)]
struct ColorPair {
    top: Rgb,
    bottom: Rgb,
}

/// Cached samples, stored as one pair of RGB colors per terminal cell.
#[derive(Default)]
pub struct Preview {
    width: u16,
    height: u16,
    cells: Vec<ColorPair>,
}

impl Preview {
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// The most colorful sample, so a border drawn from it never turns grey.
    pub fn most_vivid(&self) -> Option<Rgb> {
        let chroma = |rgb: Rgb| {
            let [_, a, b] = rgb_to_oklab(rgb);
            a.hypot(b)
        };
        self.cells
            .iter()
            .flat_map(|pair| [pair.top, pair.bottom])
            .max_by(|one, other| chroma(*one).total_cmp(&chroma(*other)))
    }

    pub fn rebuild(&mut self, width: u16, height: u16, sample: impl Fn(f32, f32) -> Rgb) {
        self.rebuild_with_step(width, height, 1, sample);
    }

    /// Samples every `step` pixels on both axes and interpolates between them.
    pub fn rebuild_with_step(
        &mut self,
        width: u16,
        height: u16,
        step: usize,
        sample: impl Fn(f32, f32) -> Rgb,
    ) {
        self.width = width;
        self.height = height;
        self.cells.resize(
            usize::from(width) * usize::from(height),
            ColorPair::default(),
        );

        let (width_pixels, height_pixels) = (usize::from(width), 2 * usize::from(height));
        let x_at = |x: usize| ((x as f32 + 0.5) / width_pixels as f32).min(1.0);
        let y_at = |y: usize| ((y as f32 + 0.5) / height_pixels as f32).min(1.0);
        let exact = |x: usize, y: usize| sample(x_at(x), y_at(y));
        let grid = (step > 1).then(|| CoarseGrid::new(0, step, width_pixels, height_pixels, exact));
        let color = |x: usize, y: usize| match &grid {
            Some(grid) => grid.at(x, y),
            None => exact(x, y),
        };

        for row in 0..usize::from(height) {
            for column in 0..width_pixels {
                self.cells[row * width_pixels + column] = ColorPair {
                    top: color(column, 2 * row),
                    bottom: color(column, 2 * row + 1),
                };
            }
        }
    }
}

/// Displays two cached colors per terminal cell using the upper-half block.
pub struct HalfBlocks<'a>(pub &'a Preview);

impl Widget for HalfBlocks<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let preview = self.0;
        for row in 0..area.height.min(preview.height) {
            for column in 0..area.width.min(preview.width) {
                let pair = preview.cells
                    [usize::from(row) * usize::from(preview.width) + usize::from(column)];
                let [r_top, g_top, b_top] = pair.top;
                let [r_bottom, g_bottom, b_bottom] = pair.bottom;
                buffer[(area.x + column, area.y + row)]
                    .set_char('▀')
                    .set_fg(Color::Rgb(r_top, g_top, b_top))
                    .set_bg(Color::Rgb(r_bottom, g_bottom, b_bottom));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn coarse_grid_reproduces_a_linear_field() {
        let field = |x: usize, y: usize| [(2 * x + 3 * y) as u8, x as u8, y as u8];
        let grid = CoarseGrid::new(10, 2, 20, 8, field);
        for y in 0..8 {
            for x in 10..20 {
                assert_eq!(grid.at(x, y), field(x, y), "pixel ({x}, {y})");
            }
        }
    }

    const TOP: Color = Color::Rgb(45, 180, 210);
    const BOTTOM: Color = Color::Rgb(245, 165, 65);

    fn test_preview(width: u16, height: u16) -> Preview {
        let mut preview = Preview::default();
        preview.rebuild(width, height, |_, y| {
            if (y * 2.0 * f32::from(height)).floor() as u16 % 2 == 0 {
                [45, 180, 210]
            } else {
                [245, 165, 65]
            }
        });
        preview
    }

    #[test]
    fn writes_correct_halves_inside_an_offset_area() {
        let mut terminal = Terminal::new(TestBackend::new(5, 4)).unwrap();
        let preview = test_preview(3, 2);
        terminal
            .draw(|frame| {
                frame.render_widget(HalfBlocks(&preview), Rect::new(1, 1, 3, 2));
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        for row in 0..4 {
            for column in 0..5 {
                let cell = &buffer[(column, row)];
                if (1..4).contains(&column) && (1..3).contains(&row) {
                    assert_eq!(cell.symbol(), "▀");
                    assert_eq!(cell.fg, TOP);
                    assert_eq!(cell.bg, BOTTOM);
                } else {
                    assert_eq!(cell.symbol(), " ");
                    assert_eq!(cell.fg, Color::Reset);
                    assert_eq!(cell.bg, Color::Reset);
                }
            }
        }
    }

    #[test]
    fn fills_resized_and_single_cell_previews() {
        let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
        for (width, height) in [(1, 1), (8, 3), (2, 1)] {
            let preview = test_preview(width, height);
            terminal.backend_mut().resize(width, height);
            terminal
                .draw(|frame| {
                    frame.render_widget(HalfBlocks(&preview), frame.area());
                })
                .unwrap();

            let buffer = terminal.backend().buffer();
            assert_eq!(buffer.area, Rect::new(0, 0, width, height));
            for cell in &buffer.content {
                assert_eq!(cell.symbol(), "▀");
                assert_eq!(cell.fg, TOP);
                assert_eq!(cell.bg, BOTTOM);
            }
        }
    }

    #[test]
    fn empty_areas_leave_the_buffer_unchanged() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 2));
        let original = buffer.clone();
        let preview = test_preview(2, 2);
        for area in [Rect::new(1, 1, 0, 1), Rect::new(1, 1, 1, 0)] {
            HalfBlocks(&preview).render(area, &mut buffer);
            assert_eq!(buffer, original);
        }
    }

    #[test]
    fn samples_half_cell_centers_and_handles_empty_previews() {
        let mut preview = Preview::default();
        preview.rebuild(1, 1, |x, y| {
            assert_eq!(x, 0.5);
            [(y * 100.0) as u8, 0, 0]
        });
        assert_eq!(preview.cells[0].top, [25, 0, 0]);
        assert_eq!(preview.cells[0].bottom, [75, 0, 0]);

        for (width, height) in [(0, 1), (1, 0), (0, 0)] {
            preview.rebuild(width, height, |_, _| {
                panic!("empty previews must not sample")
            });
            assert!(preview.cells.is_empty());
        }
    }
}
