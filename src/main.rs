use std::io;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout},
    widgets::Paragraph,
};

mod color;
mod gradient;
mod half_blocks;

use gradient::{Gradient, Point};
use half_blocks::{HalfBlocks, Preview};

fn main() -> io::Result<()> {
    let gradient = Gradient::new(
        &[
            Point {
                position: [0.15, 0.15],
                color: [48, 96, 230],
            },
            Point {
                position: [0.85, 0.2],
                color: [236, 100, 170],
            },
            Point {
                position: [0.5, 0.85],
                color: [245, 165, 65],
            },
        ],
        0.7,
    )
    .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
    let mut cached_preview = Preview::default();

    ratatui::run(|terminal| {
        loop {
            terminal.draw(|frame| {
                let [preview, footer] =
                    Layout::vertical([Constraint::Fill(1), Constraint::Length(1)])
                        .areas(frame.area());

                if cached_preview.size() != (preview.width, preview.height) {
                    if preview.width > 0 && preview.height > 0 {
                        let aspect = f32::from(preview.width) / (2.0 * f32::from(preview.height));
                        let field = gradient.prepare(aspect);
                        cached_preview
                            .rebuild(preview.width, preview.height, |x, y| field.sample(x, y));
                    } else {
                        cached_preview.rebuild(preview.width, preview.height, |_, _| [0, 0, 0]);
                    }
                }
                frame.render_widget(HalfBlocks(&cached_preview), preview);
                frame.render_widget(
                    Paragraph::new("q / Esc / Ctrl+C: quit | static gradient | softness: 0.70"),
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
