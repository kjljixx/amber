use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};

/// Displays two colors per terminal cell using the upper-half block character.
pub struct HalfBlocks {
    top: Color,
    bottom: Color,
}

impl HalfBlocks {
    pub const fn new(top: Color, bottom: Color) -> Self {
        Self { top, bottom }
    }
}

impl Widget for HalfBlocks {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        for row in area.top()..area.bottom() {
            for column in area.left()..area.right() {
                buffer[(column, row)]
                    .set_char('▀')
                    .set_fg(self.top)
                    .set_bg(self.bottom);
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

    #[test]
    fn writes_correct_halves_inside_an_offset_area() {
        let mut terminal = Terminal::new(TestBackend::new(5, 4)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(HalfBlocks::new(TOP, BOTTOM), Rect::new(1, 1, 3, 2));
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
            terminal.backend_mut().resize(width, height);
            terminal
                .draw(|frame| {
                    frame.render_widget(HalfBlocks::new(TOP, BOTTOM), frame.area());
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
        for area in [Rect::new(1, 1, 0, 1), Rect::new(1, 1, 1, 0)] {
            HalfBlocks::new(TOP, BOTTOM).render(area, &mut buffer);
            assert_eq!(buffer, original);
        }
    }
}
