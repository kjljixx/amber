use std::io;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout},
    style::Color,
    widgets::Paragraph,
};

mod half_blocks;

use half_blocks::HalfBlocks;

fn main() -> io::Result<()> {
    ratatui::run(|terminal| {
        loop {
            terminal.draw(|frame| {
                let [preview, footer] =
                    Layout::vertical([Constraint::Fill(1), Constraint::Length(1)])
                        .areas(frame.area());

                frame.render_widget(
                    HalfBlocks::new(Color::Rgb(45, 180, 210), Color::Rgb(245, 165, 65)),
                    preview,
                );
                frame.render_widget(
                    Paragraph::new("q / Esc / Ctrl+C: quit | top: cyan | bottom: amber"),
                    footer,
                );
            })?;

            if let Event::Key(key) = event::read()? {
                if key.is_press()
                    && (matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)))
                {
                    return Ok(());
                }
            }
        }
    })
}
