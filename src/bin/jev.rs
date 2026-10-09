use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use amber::{
    GradientView,
    generation::{GeneratedGradient, Generator, JevClient},
    presets,
};
use crossterm::{
    QueueableCommand,
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate},
};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

#[path = "../speed_control.rs"]
mod speed_control;
use speed_control::SpeedControl;

/// Keep the input cursor out of the gradient while its cells are being updated.
/// End synchronization even when drawing fails, so the terminal is not held.
fn synchronized_draw<W: io::Write>(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<W>>,
    render: impl FnOnce(&mut ratatui::Frame),
) -> io::Result<()> {
    terminal.backend_mut().queue(BeginSynchronizedUpdate)?;
    let draw = terminal
        .hide_cursor()
        .and_then(|_| terminal.draw(render).map(|_| ()));
    let finish = crossterm::execute!(terminal.backend_mut(), EndSynchronizedUpdate);
    draw?;
    finish
}

const HELP: &str = "Usage: jev [--prompt TEXT] [--output FILE]\n\nWithout --output, opens an interactive gradient generator.\nWith --output, generates a JSON recipe and exits. Requires --prompt.\n\nEnter: generate | Ctrl+P: pause | Up/Down: speed | Esc / Ctrl+C: quit\nReads OPENROUTER_API_KEY and optional JEV_MODEL from the environment or .env.";

#[derive(Default)]
struct Options {
    prompt: String,
    output: Option<PathBuf>,
    help: bool,
}

fn options(args: impl IntoIterator<Item = std::ffi::OsString>) -> io::Result<Options> {
    let mut args = args.into_iter();
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            options.help = true;
            return Ok(options);
        } else if arg == "--prompt" {
            if !options.prompt.is_empty() {
                return Err(io::Error::other("--prompt may only be supplied once"));
            }
            options.prompt = args
                .next()
                .and_then(|arg| arg.into_string().ok())
                .filter(|arg| !arg.starts_with("--"))
                .ok_or_else(|| io::Error::other("--prompt requires text"))?;
        } else if arg == "--output" {
            if options.output.is_some() {
                return Err(io::Error::other("--output may only be supplied once"));
            }
            options.output = Some(
                args.next()
                    .filter(|arg| !arg.to_string_lossy().starts_with("--"))
                    .ok_or_else(|| io::Error::other("--output requires a file path"))?
                    .into(),
            );
        } else {
            return Err(io::Error::other(format!(
                "Unknown argument '{}'.\n{HELP}",
                arg.to_string_lossy()
            )));
        }
    }
    if options.output.is_some() && options.prompt.trim().is_empty() {
        return Err(io::Error::other("--output requires --prompt TEXT"));
    }
    if options.prompt.chars().count() > 1000 {
        return Err(io::Error::other(
            "Keep the description under 1,000 characters",
        ));
    }
    Ok(options)
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jev: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    let options = options(std::env::args_os().skip(1))?;
    if options.help {
        println!("{HELP}");
        return Ok(());
    }
    if let Some(path) = options.output {
        // Reject an occupied destination before paying for a request.
        if path.exists() {
            return Err(io::Error::other(
                "Output file already exists; choose a new path",
            ));
        }
        let client = JevClient::from_env(Path::new(".env")).map_err(io::Error::other)?;
        let gradient = client.generate(&options.prompt).map_err(io::Error::other)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        serde_json::to_writer_pretty(&mut file, &gradient.recipe)?;
        println!("Saved gradient recipe to {}", path.display());
        return Ok(());
    }
    interactive(options.prompt)
}

fn apply(
    view: &mut GradientView,
    gradient: &GeneratedGradient,
    speed: &mut SpeedControl,
) -> io::Result<()> {
    view.set_recipe(&gradient.recipe)
        .map_err(io::Error::other)?;
    speed.set_base(gradient.radians_per_second, view)
}

fn interactive(mut prompt: String) -> io::Result<()> {
    prompt.retain(|c| !c.is_control());
    let mut view = GradientView::new(&presets::presets()[1].recipe, Instant::now())
        .map_err(io::Error::other)?;
    let mut speed = SpeedControl::new(0.15);
    speed.apply(&mut view)?;
    let mut generator = Generator::new();
    if !prompt.is_empty() {
        generator.submit(&prompt);
    }
    let mut redraw = true;
    ratatui::run(|terminal| {
        loop {
            if let Some(result) = generator.poll() {
                if let Ok(gradient) = result {
                    apply(&mut view, &gradient, &mut speed)?;
                }
                redraw = true;
            }
            redraw |= view.update(Instant::now());
            if redraw {
                synchronized_draw(terminal, |frame| {
                    let area = frame.area();
                    view.render(frame, area);
                    let [_, panel] =
                        Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(area);
                    // Opaque input panel keeps text legible over any generated palette.
                    frame.render_widget(Clear, panel);
                    let style = Style::default().fg(Color::White).bg(Color::Black);
                    let [help, input, status] =
                        Layout::vertical([Constraint::Length(1); 3]).areas(panel);
                    frame.render_widget(
                        Paragraph::new(
                            "Describe a gradient · Enter: generate · Ctrl+P: pause · Up/Down: speed · Esc: quit",
                        )
                        .style(style),
                        help,
                    );
                    let available = usize::from(input.width.saturating_sub(3));
                    let mut visible = prompt.as_str();
                    while visible.width() > available {
                        visible =
                            &visible[visible.chars().next().map(char::len_utf8).unwrap_or(0)..];
                    }
                    frame.render_widget(Paragraph::new(format!("> {visible}")).style(style), input);
                    frame.render_widget(
                        Paragraph::new(format!(
                            "Speed {:.2}x · {}",
                            speed.multiplier(),
                            generator.status()
                        ))
                        .style(style),
                        status,
                    );
                    if input.width >= 3 && input.height > 0 {
                        frame.set_cursor_position((input.x + 2 + visible.width() as u16, input.y));
                    }
                })?;
                redraw = false;
            }
            // A pending request must be polled even when the gradient is paused.
            let timeout = match (view.wait_duration(Instant::now()), generator.is_pending()) {
                (Some(delay), true) => Some(delay.min(Duration::from_millis(25))),
                (None, true) => Some(Duration::from_millis(25)),
                (delay, false) => delay,
            };
            if let Some(timeout) = timeout {
                if !event::poll(timeout)? {
                    continue;
                }
            }
            match event::read()? {
                Event::Key(key) if key.is_press() => {
                    match key.code {
                        KeyCode::Esc => return Ok(()),
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(());
                        }
                        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            let mut playback = view.playback();
                            playback.playing = !playback.playing;
                            view.set_playback(playback, Instant::now());
                        }
                        KeyCode::Up | KeyCode::Down => {
                            speed
                                .change(if key.code == KeyCode::Up { 1 } else { -1 }, &mut view)?;
                        }
                        KeyCode::Enter => {
                            if let Some(cached) = generator.submit(&prompt) {
                                apply(&mut view, &cached, &mut speed)?;
                            }
                        }
                        KeyCode::Backspace => {
                            prompt.pop();
                        }
                        KeyCode::Char(c)
                            if !c.is_control()
                                && !key
                                    .modifiers
                                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                        {
                            if prompt.chars().count() < 1000 {
                                prompt.push(c);
                            }
                        }
                        _ => {}
                    }
                    redraw = true;
                }
                Event::Paste(text) => {
                    for c in text
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(1000 - prompt.chars().count())
                    {
                        prompt.push(c);
                    }
                    redraw = true;
                }
                Event::Resize(_, _) => redraw = true,
                _ => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synchronized_frame_hides_cursor_before_painting_and_restores_it_afterward() {
        let mut bytes = Vec::<u8>::new();
        let backend = ratatui::backend::CrosstermBackend::new(&mut bytes);
        let mut terminal = ratatui::Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, 10, 4)),
            },
        )
        .unwrap();
        synchronized_draw(&mut terminal, |frame| {
            frame.render_widget(Paragraph::new("paint"), frame.area());
            frame.set_cursor_position((2, 3));
        })
        .unwrap();
        drop(terminal);
        let output = String::from_utf8(bytes).unwrap();
        let begin = output.find("\x1b[?2026h").unwrap();
        let hide = output.find("\x1b[?25l").unwrap();
        let paint = output.find("paint").unwrap();
        let show = output.find("\x1b[?25h").unwrap();
        let end = output.find("\x1b[?2026l").unwrap();
        assert!(begin < hide && hide < paint && paint < show && show < end);
    }
    fn args(values: &[&str]) -> Vec<std::ffi::OsString> {
        values.iter().map(std::ffi::OsString::from).collect()
    }

    #[test]
    fn cli_supports_interactive_and_export_modes() {
        assert!(options(args(&[])).unwrap().output.is_none());
        let export = options(args(&["--prompt", "glacier ice", "--output", "ice.json"])).unwrap();
        assert_eq!(export.prompt, "glacier ice");
        assert_eq!(export.output, Some(PathBuf::from("ice.json")));
        assert!(options(args(&["--output", "ice.json"])).is_err());
        assert!(options(args(&["--prompt"])).is_err());
        assert!(options(args(&["--unknown"])).is_err());
    }
}
