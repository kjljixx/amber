use std::{io, time::Instant};

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout},
    widgets::Paragraph,
};
use tracing::info;

mod cli;
mod speed_control;
use speed_control::SpeedControl;

use amber::{GradientView, Recipe, Scene, presets, scene};

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("amber: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// An existing file wins; otherwise the name must be a bundled scene.
fn load_scene(name: &str) -> io::Result<Recipe> {
    let path = std::path::Path::new(name);
    let scene = if path.is_file() {
        Scene::load(path)?
    } else {
        scene::template(name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no scene file or bundled scene named '{name}' (bundled: {})",
                    scene::template_names().collect::<Vec<_>>().join(", ")
                ),
            )
        })?
    };
    scene.compile().map_err(io::Error::other)
}

fn run() -> io::Result<()> {
    let (debug, args) = amber::log::take_debug_flag(std::env::args_os().skip(1));
    let command = cli::parse(args).map_err(|message| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{message}\n{}", cli::USAGE),
        )
    })?;
    let cli::Command::Run { recipe, scene } = command else {
        println!("{}", cli::USAGE);
        return Ok(());
    };
    let log_directory = amber::log::init(debug)?;
    info!(
        recipe = ?recipe,
        scene = ?scene,
        logs = %log_directory.display(),
        "amber starting"
    );
    let result = view_recipes(recipe, scene);
    match &result {
        Ok(()) => info!("amber finished"),
        Err(error) => tracing::warn!(%error, "amber failed"),
    }
    println!("Logs: {}", log_directory.display());
    result
}

fn view_recipes(recipe: Option<std::path::PathBuf>, scene: Option<String>) -> io::Result<()> {
    let mut presets = Vec::from(presets::presets());
    let mut mode = 0;
    if let Some(path) = recipe {
        presets.push(presets::Preset {
            name: "Loaded",
            recipe: Recipe::load(&path)?,
        });
        mode = presets.len() - 1;
        info!(path = %path.display(), "loaded recipe");
    }
    if let Some(name) = scene {
        presets.push(presets::Preset {
            name: "Scene",
            recipe: load_scene(&name)?,
        });
        mode = presets.len() - 1;
        info!(%name, "loaded scene");
    }
    let mut view =
        GradientView::new(&presets[mode].recipe, Instant::now()).map_err(io::Error::other)?;
    let mut speed = SpeedControl::new(3.0);
    speed.set_base(3.0, &mut view)?;
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
                            "Space: {} | w: {} | d: {} | Up/Down: speed | q: quit\nm: {} | speed: {:.2}x | sampling: {:.1}ms | previous frame: {:.1}ms",
                            if playback.playing { "pause" } else { "resume" },
                            if playback.warped { "on" } else { "off" },
                            if playback.drifting { "on" } else { "off" },
                            presets[mode].name,
                            speed.multiplier(),
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
                        KeyCode::Up | KeyCode::Down => {
                            speed
                                .change(if key.code == KeyCode::Up { 1 } else { -1 }, &mut view)?;
                            redraw = true;
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
