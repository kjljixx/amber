//! Big digits over an animated gradient.
//!
//! `cargo run --offline --example digits -- 25:49 [alpha]`
//! Digits, `:`, `.`, `-` and spaces only; use \n for several lines. Space
//! pauses the gradient; `q` exits.
use std::{io, time::Instant};

use amber::{Digits, GradientView, presets};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};

fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let text = args
        .next()
        .unwrap_or_else(|| "25:49".into())
        .replace("\\n", "\n");
    let alpha = match args.next() {
        Some(alpha) => alpha.parse().map_err(io::Error::other)?,
        None => 0.85,
    };
    let mut digits = Digits::new(&text, [255, 255, 255], alpha).map_err(io::Error::other)?;
    let mut background = GradientView::new(&presets::presets()[1].recipe, Instant::now())
        .map_err(io::Error::other)?;
    let mut redraw = true;
    ratatui::run(|terminal| {
        loop {
            redraw |= background.update(Instant::now());
            if redraw {
                terminal.draw(|frame| {
                    // The gradient first, then the digits blended over it.
                    background.render(frame, frame.area());
                    frame.render_widget(&mut digits, frame.area());
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
