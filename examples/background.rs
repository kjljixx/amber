//! A host TUI drawing its own panel over an animated gradient background.
use std::{io, time::Instant};

use amber::{GradientView, presets};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Clear, Paragraph, Wrap},
};

fn main() -> io::Result<()> {
    let mut background = GradientView::new(&presets::presets()[1].recipe, Instant::now())
        .map_err(io::Error::other)?;
    let mut redraw = true;
    ratatui::run(|terminal| {
        loop {
            redraw |= background.update(Instant::now());
            if redraw {
                terminal.draw(|frame| {
                    let area = frame.area();
                    // Paint the background first, then the host application's UI.
                    background.render(frame, area);
                    let width = area.width.saturating_sub(4).min(48);
                    let height = area.height.saturating_sub(2).min(7);
                    let panel = Rect::new(area.x + (area.width - width) / 2,
                        area.y + (area.height - height) / 2, width, height);
                    frame.render_widget(Clear, panel);
                    frame.render_widget(Paragraph::new(
                        "Gradient background\n\nYour TUI owns this panel and keyboard input.\n\nSpace: pause / resume\nq: quit"
                    ).wrap(Wrap { trim: false }).style(Style::default().fg(Color::White).bg(Color::Black)), panel);
                })?;
                redraw = false;
            }
            if let Some(timeout) = background.wait_duration(Instant::now()) {
                if !event::poll(timeout)? {
                    continue;
                }
            }
            match event::read()? {
                Event::Key(key) if key.is_press() => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    KeyCode::Char(' ') => {
                        let mut playback = background.playback();
                        playback.playing = !playback.playing;
                        background.set_playback(playback, Instant::now());
                        redraw = true;
                    }
                    _ => {}
                },
                Event::Resize(_, _) => redraw = true,
                _ => {}
            }
        }
    })
}
