use std::{io, time::Instant};

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout},
    widgets::Paragraph,
};

mod cli;

use amber::{GradientView, Recipe, presets};

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("amber: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    let command = cli::parse(std::env::args_os().skip(1)).map_err(|message| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{message}\n{}", cli::USAGE),
        )
    })?;
    let cli::Command::Run { recipe } = command else {
        println!("{}", cli::USAGE);
        return Ok(());
    };
    let mut presets = Vec::from(presets::presets());
    let mut mode = 0;
    if let Some(path) = recipe {
        presets.push(presets::Preset {
            name: "Loaded",
            recipe: Recipe::load(&path)?,
        });
        mode = presets.len() - 1;
    }
    let mut view =
        GradientView::new(&presets[mode].recipe, Instant::now()).map_err(io::Error::other)?;
    let mut redraw = true;
    let mut previous_frame_ms = 0.0;

    ratatui::run(|terminal| {
        loop {
            redraw |= view.update(Instant::now());

            if redraw {
                let frame_start = Instant::now();
                terminal.draw(|frame| {
                    let [preview, footer] =
                        Layout::vertical([Constraint::Fill(1), Constraint::Length(2)])
                            .areas(frame.area());

                    view.render(frame, preview);
                    let playback = view.playback();
                    frame.render_widget(
                        Paragraph::new(format!(
                            "Space: {} | w: {} | d: {} | q: quit\nm: {} | sampling: {:.1}ms | previous frame: {:.1}ms",
                            if playback.playing { "pause" } else { "resume" },
                            if playback.warped { "on" } else { "off" },
                            if playback.drifting { "on" } else { "off" },
                            presets[mode].name,
                            view.stats().sampling.as_secs_f64() * 1000.0,
                            previous_frame_ms,
                        )),
                        footer,
                    );
                })?;
                previous_frame_ms = frame_start.elapsed().as_secs_f64() * 1000.0;
                redraw = false;
            }

            // Animated: wait until input or the next frame deadline.
            // Paused/all motion off: block on input without periodic wakeups.
            if let Some(timeout) = view.wait_duration(Instant::now()) {
                if !event::poll(timeout)? {
                    continue;
                }
            }

            match event::read()? {
                Event::Key(key) if key.is_press() => {
                    let mut playback = view.playback();
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(());
                        }
                        KeyCode::Char('m') => {
                            mode = (mode + 1) % presets.len();
                            view.set_recipe(&presets[mode].recipe)
                                .map_err(io::Error::other)?;
                            redraw = true;
                        }
                        KeyCode::Char('d') => {
                            playback.drifting = !playback.drifting;
                            redraw = true;
                        }
                        KeyCode::Char('w') => {
                            playback.warped = !playback.warped;
                            redraw = true;
                        }
                        KeyCode::Char(' ') => {
                            playback.playing = !playback.playing;
                            redraw = true;
                        }
                        _ => {}
                    }
                    view.set_playback(playback, Instant::now());
                }
                Event::Resize(_, _) => redraw = true,
                _ => {}
            }
        }
    })
}
