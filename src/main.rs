use std::io;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::widgets::Paragraph;

fn main() -> io::Result<()> {
    ratatui::run(|terminal| {
        loop {
            terminal.draw(|frame| {
                frame.render_widget(
                    Paragraph::new("Hello, Amber!\n\nPress q, Esc, or Ctrl+C to quit."),
                    frame.area(),
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
