use std::{io, time::Instant};

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout},
    widgets::Paragraph,
};

mod animation;
mod color;
mod gradient;
mod half_blocks;
mod warp;

use animation::Animation;
use gradient::{Gradient, Point};
use half_blocks::{HalfBlocks, Preview};
use warp::Warp;

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
    let warp = Warp::new(1.0, 0.45, 58)
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
    let mut warped = true;
    let mut playing = true;
    let mut preview_dirty = true;
    let mut redraw = true;

    ratatui::run(|terminal| {
        let mut animation = Animation::new(Instant::now());
        loop {
            if animation.tick(Instant::now()) {
                preview_dirty = true;
                redraw = true;
            }

            if redraw {
                terminal.draw(|frame| {
                    let [preview, footer] =
                        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)])
                            .areas(frame.area());

                    if preview_dirty || cached_preview.size() != (preview.width, preview.height) {
                        if preview.width > 0 && preview.height > 0 {
                            let aspect =
                                f32::from(preview.width) / (2.0 * f32::from(preview.height));
                            let field = gradient.prepare(aspect).with_warp(if warped {
                                warp.at_time(animation.phase())
                            } else {
                                Warp::default()
                            });
                            cached_preview
                                .rebuild(preview.width, preview.height, |x, y| field.sample(x, y));
                        } else {
                            cached_preview.rebuild(preview.width, preview.height, |_, _| [0, 0, 0]);
                        }
                        preview_dirty = false;
                    }
                    frame.render_widget(HalfBlocks(&cached_preview), preview);
                    frame.render_widget(
                        Paragraph::new(format!(
                            "Space: {} | w: warp {} | q / Esc / Ctrl+C: quit",
                            if playing { "pause" } else { "resume" },
                            if warped { "on" } else { "off" }
                        )),
                        footer,
                    );
                })?;
                redraw = false;
            }

            // Animated: wait until input or the next frame deadline.
            // Paused/warp off: block on input without periodic wakeups.
            if let Some(timeout) = animation.wait_duration(Instant::now()) {
                if !event::poll(timeout)? {
                    continue;
                }
            }

            match event::read()? {
                Event::Key(key) if key.is_press() => {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(());
                        }
                        KeyCode::Char('w') => {
                            warped = !warped;
                            preview_dirty = true;
                            redraw = true;
                        }
                        KeyCode::Char(' ') => {
                            playing = !playing;
                            redraw = true;
                        }
                        _ => {}
                    }
                    animation.set_running(playing && warped, Instant::now());
                }
                Event::Resize(_, _) => redraw = true,
                _ => {}
            }
        }
    })
}
