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
mod presets;
mod warp;

use animation::Animation;
use half_blocks::{HalfBlocks, Preview};
use warp::Warp;

fn main() -> io::Result<()> {
    let presets = presets::presets()
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
    let mut mode = 0;
    let mut cached_preview = Preview::default();
    let mut warped = true;
    let mut playing = true;
    let mut preview_dirty = true;
    let mut redraw = true;
    let mut sampling_ms = 0.0;
    let mut previous_frame_ms = 0.0;

    ratatui::run(|terminal| {
        let mut animation = Animation::new(Instant::now());
        loop {
            if animation.tick(Instant::now()) {
                preview_dirty = true;
                redraw = true;
            }

            if redraw {
                let frame_start = Instant::now();
                terminal.draw(|frame| {
                    let [preview, footer] =
                        Layout::vertical([Constraint::Fill(1), Constraint::Length(2)])
                            .areas(frame.area());

                    if preview_dirty || cached_preview.size() != (preview.width, preview.height) {
                        let sampling_start = Instant::now();
                        if preview.width > 0 && preview.height > 0 {
                            let aspect =
                                f32::from(preview.width) / (2.0 * f32::from(preview.height));
                            let preset = &presets[mode];
                            let field = preset.gradient.prepare(aspect)
                                .with_warp(if warped {
                                    preset.warp.at_time(animation.phase())
                                } else {
                                    Warp::default()
                                });
                            cached_preview
                                .rebuild(preview.width, preview.height, |x, y| field.sample(x, y));
                        } else {
                            cached_preview.rebuild(preview.width, preview.height, |_, _| [0, 0, 0]);
                        }
                        sampling_ms = sampling_start.elapsed().as_secs_f64() * 1000.0;
                        preview_dirty = false;
                    }
                    frame.render_widget(HalfBlocks(&cached_preview), preview);
                    frame.render_widget(
                        Paragraph::new(format!(
                            "Space: {} | w: {} | q: quit\nm: {} | sampling: {:.1}ms | previous frame: {:.1}ms",
                            if playing { "pause" } else { "resume" },
                            if warped { "on" } else { "off" },
                            presets[mode].name,
                            sampling_ms,
                            previous_frame_ms,
                        )),
                        footer,
                    );
                })?;
                previous_frame_ms = frame_start.elapsed().as_secs_f64() * 1000.0;
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
                        KeyCode::Char('m') => {
                            mode = (mode + 1) % presets.len();
                            preview_dirty = true;
                            redraw = true;
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
