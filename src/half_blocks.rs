use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};

use crate::color::{Rgb, rgb_to_oklab};

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
        self.width = width;
        self.height = height;
        self.cells.resize(
            usize::from(width) * usize::from(height),
            ColorPair::default(),
        );

        for row in 0..height {
            for column in 0..width {
                let x = (f32::from(column) + 0.5) / f32::from(width);
                let top_y = (2.0 * f32::from(row) + 0.5) / (2.0 * f32::from(height));
                let bottom_y = (2.0 * f32::from(row) + 1.5) / (2.0 * f32::from(height));
                self.cells[usize::from(row) * usize::from(width) + usize::from(column)] =
                    ColorPair {
                        top: sample(x, top_y),
                        bottom: sample(x, bottom_y),
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
