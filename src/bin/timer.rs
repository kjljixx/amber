//! A pomodoro clock: big translucent digits over an animated gradient, with
//! four bars along the bottom showing which work period of the cycle this is.
//!
//! The cycle is work, 5 minute break, three times over, then a fourth work
//! period and a 20 minute break, and then it starts again.
//!
//! `cargo run --offline --bin timer`
//! Space: pause or resume the timer and the gradient | r: back to the start of
//! the cycle, 25:00 | t: describe a gradient for Jev to make, in a fresh box (Enter
//! sends it, Esc cancels) | Up/Down: gradient speed | q, Esc or Ctrl+C: quit
//!
//! Once Jev has made a gradient, the description stays centered above the clock
//! until a new one replaces it.
//!
//! The first gradient is made for the moment: the weekday, time of day, month
//! and weather, such as "Friday golden hour in October (autumn), light rain,
//! cool". The weather is for wherever the machine's IP address seems to be;
//! `--location Lisbon` names a place instead, and `--no-weather` skips the
//! lookup and describes the time alone. Only that description goes to Jev, never
//! the place.
//!
//! When a phase ends, a chime plays and a desktop notification appears: a
//! climbing one when a work period ends, a falling one when a break does.
//! `cargo run --offline --bin timer -- --alarm-test` plays both now, to check
//! your sound and notification settings.
//!
//! Jev needs `OPENROUTER_API_KEY` in the environment or a `.env` file; see
//! docs/generation.md.
use std::{
    io,
    time::{Duration, Instant},
};

use amber::{
    Digits, GradientView,
    ambient::{Ambient, Place},
    blend_samples,
    generation::{GeneratedGradient, Generator},
    presets,
};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[path = "../alarm.rs"]
mod alarm;
#[path = "../speed_control.rs"]
mod speed_control;
use speed_control::SpeedControl;

const WORK: Duration = Duration::from_secs(25 * 60);
const SHORT_BREAK: Duration = Duration::from_secs(5 * 60);
const LONG_BREAK: Duration = Duration::from_secs(20 * 60);
const WORK_PERIODS: u8 = 4;

/// Opacity of the digits and of the lit part of the bars.
const ALPHA: f32 = 0.6;
/// Opacity of the unlit part of the bars.
const DIM_ALPHA: f32 = 0.25;
/// Gradient animation rate before Jev suggests one, in radians per second.
const BASE_SPEED: f64 = 0.15;
/// Speed steps above 1x at the start: each is a factor of the square root of
/// two, so 6 steps is 8x. With the base rate that is 1.2 radians per second.
const DEFAULT_SPEED_STEPS: i32 = 6;
const MAX_PROMPT_CHARS: usize = 1000;
/// How far the caption's backing is darkened, and how many cells it reaches
/// past each end of the text.
const SCRIM_ALPHA: f32 = 0.3;
const SCRIM_PADDING: u16 = 2;
const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];

/// One stretch of the cycle. The number says which work period it is in or
/// follows, from 1 to 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Work(u8),
    Break(u8),
}

impl Phase {
    fn length(self) -> Duration {
        match self {
            Self::Work(_) => WORK,
            Self::Break(WORK_PERIODS) => LONG_BREAK,
            Self::Break(_) => SHORT_BREAK,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Work(n) => Self::Break(n),
            Self::Break(WORK_PERIODS) => Self::Work(1),
            Self::Break(n) => Self::Work(n + 1),
        }
    }
}

/// Counts down through the cycle, moving on to the next phase as each one ends.
/// While paused it holds the time left, and resuming sets a new deadline that
/// far ahead.
struct Clock {
    phase: Phase,
    deadline: Instant,
    paused: Option<Duration>,
}

impl Clock {
    fn new(now: Instant) -> Self {
        Self {
            phase: Phase::Work(1),
            deadline: now + Phase::Work(1).length(),
            paused: None,
        }
    }

    fn phase(&self) -> Phase {
        self.phase
    }

    fn is_paused(&self) -> bool {
        self.paused.is_some()
    }

    /// Move past every phase that has ended, carrying any overshoot into the
    /// next so the cycle does not drift. Returns whether the phase changed.
    fn advance(&mut self, now: Instant) -> bool {
        if self.is_paused() {
            return false;
        }
        let mut changed = false;
        while now >= self.deadline {
            self.phase = self.phase.next();
            self.deadline += self.phase.length();
            changed = true;
        }
        changed
    }

    fn remaining(&self, now: Instant) -> Duration {
        self.paused
            .unwrap_or_else(|| self.deadline.saturating_duration_since(now))
    }

    /// How much of the current phase has passed, 0 to 1.
    fn progress(&self, now: Instant) -> f32 {
        let length = self.phase.length().as_secs_f32();
        (1.0 - self.remaining(now).as_secs_f32() / length).clamp(0.0, 1.0)
    }

    fn toggle_pause(&mut self, now: Instant) {
        match self.paused.take() {
            Some(left) => self.deadline = now + left,
            None => self.paused = Some(self.remaining(now)),
        }
    }

    /// Back to 25:00 at the start of the cycle. A paused clock stays paused.
    fn reset(&mut self, now: Instant) {
        self.phase = Phase::Work(1);
        self.deadline = now + self.phase.length();
        if self.is_paused() {
            self.paused = Some(self.phase.length());
        }
    }

    /// How long until `label` shows a different value, or `None` when paused.
    fn until_label_changes(&self, now: Instant) -> Option<Duration> {
        let remaining = self.remaining(now);
        if self.is_paused() {
            None
        } else if remaining.subsec_nanos() > 0 {
            Some(Duration::from_nanos(remaining.subsec_nanos().into()))
        } else {
            Some(Duration::from_secs(1).min(remaining))
        }
    }
}

/// `mm:ss`, rounding up so a phase reads 25:00 for its whole first second.
fn label(remaining: Duration) -> String {
    let seconds = remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0);
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// How full each of the four bars is, 0 to 1. Finished work periods are full,
/// the current one fills as it goes, and the rest are empty. During a break no
/// bar is part full, which is how a break looks different from work.
fn cycle_fill(phase: Phase, progress: f32) -> [f32; 4] {
    let (finished, current) = match phase {
        Phase::Work(n) => (n - 1, progress),
        Phase::Break(n) => (n, 0.0),
    };
    std::array::from_fn(|index| match (index as u8).cmp(&finished) {
        std::cmp::Ordering::Less => 1.0,
        std::cmp::Ordering::Equal => current,
        std::cmp::Ordering::Greater => 0.0,
    })
}

/// Where the four bars sit, in half-block samples relative to the screen.
#[derive(Debug, PartialEq, Eq)]
struct Bars {
    top: u32,
    bottom: u32,
    /// `[left, right)` of each bar.
    segments: [(u32, u32); 4],
}

impl Bars {
    /// A strip along the bottom edge, clear of the digits, which never reach
    /// below nine tenths of the height. `None` when the screen is too small.
    fn layout(width: u16, rows: u16) -> Option<Self> {
        let height = 2 * u32::from(rows);
        if height < 16 {
            return None;
        }
        let thickness = (height / 24).clamp(1, 3);
        let bottom = height - thickness;
        let width = u32::from(width);
        let span = width - 2 * (width / 10);
        let gap = (span / 40).max(1);
        let each = span.saturating_sub(3 * gap) / 4;
        if each == 0 {
            return None;
        }
        let left = (width - (4 * each + 3 * gap)) / 2;
        Some(Self {
            top: bottom - thickness,
            bottom,
            segments: std::array::from_fn(|index| {
                let start = left + index as u32 * (each + gap);
                (start, start + each)
            }),
        })
    }

    /// `Some(true)` for a lit sample of a bar, `Some(false)` for an unlit one,
    /// `None` when the sample is not part of a bar.
    fn lit(&self, fill: &[f32; 4], column: u32, row: u32) -> Option<bool> {
        if !(self.top..self.bottom).contains(&row) {
            return None;
        }
        let (index, &(left, right)) = self
            .segments
            .iter()
            .enumerate()
            .find(|(_, (left, right))| (*left..*right).contains(&column))?;
        let filled = (fill[index] * (right - left) as f32).round() as u32;
        Some(column - left < filled)
    }
}

/// Draw the four bars over the half-block cells already in `buffer`.
fn draw_cycle(buffer: &mut Buffer, area: Rect, phase: Phase, progress: f32) {
    let Some(bars) = Bars::layout(area.width, area.height) else {
        return;
    };
    let fill = cycle_fill(phase, progress);
    let samples = |lit: bool| {
        let (bars, fill) = (&bars, &fill);
        move |column: u16, row: u16| bars.lit(fill, column.into(), row.into()) == Some(lit)
    };
    blend_samples(buffer, area, WHITE, ALPHA, samples(true));
    blend_samples(buffer, area, WHITE, DIM_ALPHA, samples(false));
}

/// The text box for describing a gradient, and the description behind the
/// gradient on screen.
#[derive(Default)]
struct Prompt {
    open: bool,
    /// What is being typed. Empty whenever the box opens, so a new description
    /// starts from nothing; the one on screen is `shown`.
    text: String,
    /// The description that made the current gradient, kept above the clock
    /// until a new one replaces it.
    shown: Option<String>,
    /// The description of a request that has not finished.
    in_flight: Option<String>,
    /// Whether that request was made at startup, not typed. The box may be
    /// open meanwhile with something half typed, which its arrival leaves alone.
    automatic: bool,
}

impl Prompt {
    fn push(&mut self, character: char) {
        if !character.is_control() && self.text.chars().count() < MAX_PROMPT_CHARS {
            self.text.push(character);
        }
    }

    fn paste(&mut self, pasted: &str) {
        for character in pasted.chars() {
            self.push(character);
        }
    }

    /// Whether there is a line to draw: the box is open, or a description is
    /// being shown above the clock.
    fn has_line(&self) -> bool {
        self.open || self.shown.is_some()
    }

    /// Open an empty box. The description on screen stays where it is until a
    /// new one has been made.
    fn begin(&mut self) {
        self.open = true;
        self.text.clear();
    }

    /// Close the box and drop what was typed. The description on screen stays.
    fn cancel(&mut self) {
        self.open = false;
        self.text.clear();
    }

    /// A request has started for the text in the box.
    fn started(&mut self) {
        self.in_flight = Some(self.text.trim().to_owned());
        self.automatic = false;
    }

    /// A request has started for `description`, made without being typed.
    fn started_automatically(&mut self, description: String) {
        self.in_flight = Some(description);
        self.automatic = true;
    }

    /// The gradient for the request in flight has arrived: its description is
    /// now the one shown, whatever has been typed since.
    fn succeeded(&mut self) {
        if let Some(description) = self.in_flight.take() {
            self.shown = Some(description);
        }
        if !std::mem::take(&mut self.automatic) {
            self.open = false;
            self.text.clear();
        }
    }

    /// The request failed. The old gradient and description stay.
    fn failed(&mut self) {
        self.in_flight = None;
        self.automatic = false;
    }

    /// The end of the text being edited that fits in `columns` terminal
    /// columns, where the cursor is.
    fn tail(&self, columns: usize) -> &str {
        let mut visible = self.text.as_str();
        while visible.width() > columns {
            let first = visible.chars().next().map_or(0, char::len_utf8);
            visible = &visible[first..];
        }
        visible
    }

    /// The start of the description on screen, ending in `…` if it is cut.
    fn caption(&self, columns: usize) -> String {
        let text = self.shown.as_deref().unwrap_or_default();
        if text.width() <= columns {
            return text.to_owned();
        }
        let mut caption = String::new();
        for character in text.chars() {
            // Leave a column for the ellipsis.
            if caption.width() + character.width().unwrap_or(0) >= columns {
                break;
            }
            caption.push(character);
        }
        caption.push('…');
        caption
    }
}

/// Where the prompt line goes: just above the digits and lined up with them,
/// with a blank row between when there is room. With nowhere above the digits,
/// or no digits on screen, it takes the top row.
fn prompt_row(area: Rect, digits: Option<Rect>) -> Option<Rect> {
    if area.is_empty() {
        return None;
    }
    if let Some(digits) = digits {
        let above = digits.y.saturating_sub(area.y);
        if above >= 1 {
            let y = digits.y - above.min(2);
            return Some(Rect::new(digits.x, y, digits.width, 1));
        }
    }
    Some(Rect::new(area.x, area.y, area.width, 1))
}

/// The prompt line. While the box is open it is a chevron and the text being
/// edited on an opaque black line, with a cursor, so it stays legible while
/// typing. Otherwise it is the description behind the current gradient, as bare
/// text centered right on the gradient.
fn draw_prompt(frame: &mut Frame, row: Rect, prompt: &Prompt) {
    if !prompt.open {
        draw_caption(
            frame.buffer_mut(),
            row,
            &prompt.caption(usize::from(row.width)),
        );
        return;
    }
    // The chevron, a space and the cursor take three columns.
    let columns = usize::from(row.width.saturating_sub(3));
    frame.render_widget(Clear, row);
    let style = Style::default().fg(Color::White).bg(Color::Black);
    // A reversed space is the cursor, so the terminal's own cursor can stay
    // hidden instead of flickering while the gradient animates.
    let line = Line::from(vec![
        Span::raw(format!("> {}", prompt.tail(columns))),
        Span::styled(" ", style.add_modifier(Modifier::REVERSED)),
    ]);
    frame.render_widget(Paragraph::new(line).style(style), row);
}

/// White bold text over the half blocks, centered in `row`, on a scrim: the
/// gradient behind it is darkened so the text stays legible over pale areas.
/// The scrim is translucent, so the gradient still shows through it, and it
/// reaches a little past each end of the text.
///
/// A letter's cell has one background color where the gradient has two, a top
/// half and a bottom half, so each cell under the text takes their average. The
/// cells of the scrim around the text keep both halves.
fn draw_caption(buffer: &mut Buffer, row: Rect, text: &str) {
    let row = row.intersection(buffer.area);
    if row.is_empty() {
        return;
    }
    let width = u16::try_from(text.width())
        .unwrap_or(u16::MAX)
        .min(row.width);
    let left = row.x + (row.width - width) / 2;
    if width == 0 {
        return;
    }

    let scrim = Rect::new(
        left.saturating_sub(SCRIM_PADDING),
        row.y,
        width + 2 * SCRIM_PADDING,
        1,
    );
    blend_samples(buffer, scrim, BLACK, SCRIM_ALPHA, |_, _| true);

    let backgrounds: Vec<Option<Color>> = (left..left + width)
        .map(|x| {
            let cell = &buffer[(x, row.y)];
            match (cell.fg, cell.bg) {
                (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) if cell.symbol() == "▀" => {
                    let mean = |a: u8, b: u8| ((u16::from(a) + u16::from(b)) / 2) as u8;
                    Some(Color::Rgb(mean(r1, r2), mean(g1, g2), mean(b1, b2)))
                }
                _ => None,
            }
        })
        .collect();
    let style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    buffer.set_stringn(left, row.y, text, usize::from(width), style);
    for (x, background) in (left..).zip(backgrounds) {
        if let Some(background) = background {
            buffer[(x, row.y)].set_bg(background);
        }
    }
}

/// Show a generated gradient and play it at the speed already chosen, scaled
/// from the rate Jev suggests for it.
fn apply(
    view: &mut GradientView,
    speed: &mut SpeedControl,
    gradient: &GeneratedGradient,
) -> io::Result<()> {
    view.set_recipe(&gradient.recipe)
        .map_err(io::Error::other)?;
    speed.set_base(gradient.radians_per_second, view)
}

/// What to play and say when `phase` has just begun.
fn announcement(phase: Phase) -> (alarm::Chime, String) {
    let minutes = phase.length().as_secs() / 60;
    match phase {
        Phase::Break(_) => (
            alarm::Chime::WorkDone,
            format!("Time for a {minutes} minute break"),
        ),
        Phase::Work(n) => (
            alarm::Chime::BreakDone,
            format!("Back to work: period {n} of {WORK_PERIODS}, {minutes} minutes"),
        ),
    }
}

/// Sound and show the end of `phase`'s predecessor, without waiting for it.
fn announce(phase: Phase) {
    let (chime, message) = announcement(phase);
    alarm::play(chime);
    alarm::notify("Pomodoro", &message);
}

/// `--alarm-test`: play both chimes with their notifications, one after the
/// other, and say how each was played.
fn alarm_test() -> io::Result<()> {
    for phase in [Phase::Break(1), Phase::Work(2)] {
        let (chime, message) = announcement(phase);
        println!("{message}");
        let notified = alarm::notify("Pomodoro", &message);
        match alarm::play(chime)
            .join()
            .map_err(|_| io::Error::other("the sound thread failed"))?
        {
            alarm::Played::Player(player) => println!("  chime played with {player}"),
            alarm::Played::Bell => println!("  no audio player worked, so the terminal bell rang"),
        }
        match notified.join() {
            Ok(true) => println!("  notification shown"),
            _ => println!("  notification failed (check your system's notification settings)"),
        }
    }
    Ok(())
}

fn digits(text: &str) -> io::Result<Digits> {
    Digits::new(text, WHITE, ALPHA).map_err(io::Error::other)
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("timer: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// What the command line asks for.
#[derive(Debug, PartialEq)]
enum Command {
    AlarmTest,
    /// Run the clock, starting from a description of this moment and `place`'s
    /// weather; with no place, of the time alone.
    Clock(Option<Place>),
}

const USAGE: &str = "options: --location <city> | --no-weather | --alarm-test";

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let mut place = Some(Place::Here);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--alarm-test" => return Ok(Command::AlarmTest),
            "--no-weather" => place = None,
            "--location" => {
                let name = args.next().filter(|name| !name.trim().is_empty());
                place = Some(Place::Named(
                    name.ok_or("--location needs a place, such as --location Lisbon")?,
                ));
            }
            other => match other.strip_prefix("--location=") {
                Some(name) if !name.trim().is_empty() => place = Some(Place::Named(name.into())),
                _ => return Err(format!("unknown argument '{other}'; {USAGE}")),
            },
        }
    }
    Ok(Command::Clock(place))
}

fn run() -> io::Result<()> {
    let place = match parse_args(std::env::args().skip(1)).map_err(io::Error::other)? {
        Command::AlarmTest => return alarm_test(),
        Command::Clock(place) => place,
    };
    let start = Instant::now();
    let mut background =
        GradientView::new(&presets::presets()[1].recipe, start).map_err(io::Error::other)?;
    let mut speed = SpeedControl::new(BASE_SPEED);
    speed.change(DEFAULT_SPEED_STEPS, &mut background)?;
    let mut generator = Generator::new();
    let mut ambient = Ambient::start(place);
    let mut prompt = Prompt::default();
    let mut clock = Clock::new(start);
    let mut shown = label(clock.phase().length());
    let mut text = digits(&shown)?;
    let mut redraw = true;
    ratatui::run(|terminal| {
        loop {
            let now = Instant::now();
            if clock.advance(now) {
                announce(clock.phase());
            }
            if let Some(description) = ambient.poll() {
                // Start from a gradient for the moment, unless a description has
                // already been asked for in the meantime.
                if prompt.shown.is_none() && prompt.in_flight.is_none() {
                    prompt.started_automatically(description.clone());
                    if let Some(cached) = generator.submit(&description) {
                        apply(&mut background, &mut speed, &cached)?;
                        prompt.succeeded();
                    } else if !generator.is_pending() {
                        prompt.failed();
                    }
                    redraw = true;
                }
            }
            if let Some(result) = generator.poll() {
                match result {
                    Ok(gradient) => {
                        apply(&mut background, &mut speed, &gradient)?;
                        prompt.succeeded();
                    }
                    Err(_) => prompt.failed(),
                }
                redraw = true;
            }
            let current = label(clock.remaining(now));
            if current != shown {
                text = digits(&current)?;
                shown = current;
                redraw = true;
            }
            redraw |= background.update(now);
            if redraw {
                terminal.draw(|frame| {
                    let area = frame.area();
                    // The gradient first, then the time and the bars over it.
                    background.render(frame, area);
                    frame.render_widget(&mut text, area);
                    draw_cycle(frame.buffer_mut(), area, clock.phase(), clock.progress(now));
                    // The prompt goes over everything else, above the digits.
                    if prompt.has_line() {
                        if let Some(row) = prompt_row(area, text.bounds(area)) {
                            draw_prompt(frame, row, &prompt);
                        }
                    }
                })?;
                redraw = false;
            }

            // Wake for the next gradient frame or the next change of the clock,
            // whichever comes first.
            let timeout = [
                background.wait_duration(Instant::now()),
                clock.until_label_changes(Instant::now()),
                // A pending request has to be polled even while paused.
                (generator.is_pending() || ambient.is_pending())
                    .then_some(Duration::from_millis(25)),
            ]
            .into_iter()
            .flatten()
            .min();
            if let Some(timeout) = timeout {
                if !event::poll(timeout)? {
                    continue;
                }
            }
            match event::read()? {
                Event::Key(key) if key.is_press() => {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        return Ok(());
                    }
                    if prompt.open {
                        match key.code {
                            KeyCode::Esc => prompt.cancel(),
                            KeyCode::Enter => {
                                let was_pending = generator.is_pending();
                                if let Some(cached) = generator.submit(&prompt.text) {
                                    apply(&mut background, &mut speed, &cached)?;
                                    prompt.started();
                                    prompt.succeeded();
                                } else if !was_pending && generator.is_pending() {
                                    prompt.started();
                                }
                            }
                            KeyCode::Backspace => {
                                prompt.text.pop();
                            }
                            KeyCode::Char(c)
                                if !key
                                    .modifiers
                                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                            {
                                prompt.push(c);
                            }
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                            KeyCode::Char('t' | 'T') => prompt.begin(),
                            KeyCode::Up | KeyCode::Down => {
                                let step = if key.code == KeyCode::Up { 1 } else { -1 };
                                speed.change(step, &mut background)?;
                            }
                            KeyCode::Char(' ') => {
                                let now = Instant::now();
                                clock.toggle_pause(now);
                                let mut playback = background.playback();
                                playback.playing = !clock.is_paused();
                                background.set_playback(playback, now);
                            }
                            KeyCode::Char('r' | 'R') => clock.reset(Instant::now()),
                            _ => {}
                        }
                    }
                    redraw = true;
                }
                Event::Paste(text) if prompt.open => {
                    prompt.paste(&text);
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
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    fn minutes(value: u64) -> Duration {
        Duration::from_secs(value * 60)
    }

    fn seconds(value: f64) -> Duration {
        Duration::from_secs_f64(value)
    }

    /// A clock whose cycle began at `start`, advanced to `start + elapsed`.
    fn at(start: Instant, elapsed: Duration) -> (Clock, Instant) {
        let mut clock = Clock::new(start);
        let now = start + elapsed;
        clock.advance(now);
        (clock, now)
    }

    #[test]
    fn the_label_rounds_up_and_reaches_zero_only_at_the_end() {
        assert_eq!(label(WORK), "25:00");
        assert_eq!(label(seconds(1499.5)), "25:00");
        assert_eq!(label(seconds(1499.0)), "24:59");
        assert_eq!(label(seconds(61.0)), "01:01");
        assert_eq!(label(seconds(0.2)), "00:01");
        assert_eq!(label(Duration::ZERO), "00:00");
    }

    #[test]
    fn the_cycle_is_four_works_with_short_breaks_then_a_long_one() {
        let mut phase = Phase::Work(1);
        let mut seen = vec![(phase, phase.length())];
        for _ in 0..7 {
            phase = phase.next();
            seen.push((phase, phase.length()));
        }
        assert_eq!(
            seen,
            [
                (Phase::Work(1), minutes(25)),
                (Phase::Break(1), minutes(5)),
                (Phase::Work(2), minutes(25)),
                (Phase::Break(2), minutes(5)),
                (Phase::Work(3), minutes(25)),
                (Phase::Break(3), minutes(5)),
                (Phase::Work(4), minutes(25)),
                (Phase::Break(4), minutes(20)),
            ]
        );
        // And then it starts over.
        assert_eq!(phase.next(), Phase::Work(1));
    }

    #[test]
    fn the_clock_moves_through_the_cycle_without_drifting() {
        let start = Instant::now();
        let check = |elapsed: Duration, phase: Phase, shown: &str| {
            let (clock, now) = at(start, elapsed);
            assert_eq!(clock.phase(), phase, "{elapsed:?}");
            assert_eq!(label(clock.remaining(now)), shown, "{elapsed:?}");
        };
        check(Duration::ZERO, Phase::Work(1), "25:00");
        check(seconds(1499.0), Phase::Work(1), "00:01");
        check(minutes(25), Phase::Break(1), "05:00");
        check(minutes(25) + seconds(1.0), Phase::Break(1), "04:59");
        check(minutes(30), Phase::Work(2), "25:00");
        check(minutes(60), Phase::Work(3), "25:00");
        check(minutes(90), Phase::Work(4), "25:00");
        check(minutes(115), Phase::Break(4), "20:00");
        // The whole cycle is 135 minutes, then it repeats.
        check(minutes(135), Phase::Work(1), "25:00");
        check(minutes(135) + minutes(30), Phase::Work(2), "25:00");
        // A long suspend skips straight to the right place: 2 cycles + 100 min.
        check(minutes(2 * 135 + 100), Phase::Work(4), "15:00");
    }

    #[test]
    fn progress_runs_from_zero_to_one_through_each_phase() {
        let start = Instant::now();
        let progress = |elapsed| {
            let (clock, now) = at(start, elapsed);
            clock.progress(now)
        };
        assert_eq!(progress(Duration::ZERO), 0.0);
        assert!((progress(seconds(750.0)) - 0.5).abs() < 1e-6);
        // A new phase starts again from zero; a 5 minute break is halfway at 2:30.
        assert!(progress(minutes(25)) < 1e-6);
        assert!((progress(minutes(25) + seconds(150.0)) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_clock_wakes_exactly_when_the_label_changes() {
        let now = Instant::now();
        for left in [seconds(1499.5), seconds(61.25), seconds(1.0), seconds(0.2)] {
            let clock = Clock {
                phase: Phase::Work(1),
                deadline: now + left,
                paused: None,
            };
            let wait = clock.until_label_changes(now).unwrap();
            assert_ne!(label(left), label(left - wait), "{left:?}");
            assert_eq!(label(left), label(left - wait + seconds(0.001)));
        }
    }

    #[test]
    fn pausing_freezes_the_time_and_resuming_continues_from_it() {
        let start = Instant::now();
        let mut clock = Clock::new(start);
        let after = |seconds| start + Duration::from_secs(seconds);

        clock.toggle_pause(after(60));
        assert!(clock.is_paused());
        assert_eq!(label(clock.remaining(after(60))), "24:00");
        // An hour later it still reads the same, stays in the same phase and
        // nothing is scheduled.
        assert!(!clock.advance(after(3660)));
        assert_eq!(clock.phase(), Phase::Work(1));
        assert_eq!(label(clock.remaining(after(3660))), "24:00");
        assert_eq!(clock.until_label_changes(after(3660)), None);

        clock.toggle_pause(after(3660));
        assert!(!clock.is_paused());
        assert_eq!(label(clock.remaining(after(3670))), "23:50");
        assert!(clock.until_label_changes(after(3670)).is_some());
        // Pausing during a break works the same way.
        let (mut clock, now) = at(start, minutes(26));
        clock.toggle_pause(now);
        assert_eq!(clock.phase(), Phase::Break(1));
        assert_eq!(label(clock.remaining(now + minutes(100))), "04:00");
    }

    #[test]
    fn reset_returns_to_the_start_of_the_cycle_keeping_the_pause_state() {
        let start = Instant::now();
        let (mut running, now) = at(start, minutes(100));
        assert_eq!(running.phase(), Phase::Work(4));
        running.reset(now);
        assert_eq!(running.phase(), Phase::Work(1));
        assert!(!running.is_paused());
        assert_eq!(label(running.remaining(now + seconds(30.0))), "24:30");

        let (mut paused, now) = at(start, minutes(100));
        paused.toggle_pause(now);
        paused.reset(now + minutes(5));
        assert!(paused.is_paused());
        assert_eq!(paused.phase(), Phase::Work(1));
        assert_eq!(label(paused.remaining(now + minutes(50))), "25:00");
    }

    #[test]
    fn the_bars_show_finished_current_and_upcoming_periods() {
        assert_eq!(cycle_fill(Phase::Work(1), 0.0), [0.0; 4]);
        assert_eq!(cycle_fill(Phase::Work(1), 0.4), [0.4, 0.0, 0.0, 0.0]);
        assert_eq!(cycle_fill(Phase::Work(3), 0.5), [1.0, 1.0, 0.5, 0.0]);
        assert_eq!(cycle_fill(Phase::Work(4), 1.0), [1.0; 4]);
        // After work 2 the second bar is full; no bar is part full in a break.
        assert_eq!(cycle_fill(Phase::Break(2), 0.7), [1.0, 1.0, 0.0, 0.0]);
        assert_eq!(cycle_fill(Phase::Break(4), 0.3), [1.0; 4]);
    }

    #[test]
    fn the_bars_sit_in_a_clear_strip_along_the_bottom() {
        // Too small to fit under the digits without touching them.
        assert_eq!(Bars::layout(100, 7), None);
        assert_eq!(Bars::layout(4, 40), None);
        for (width, rows) in [(80, 24), (120, 40), (200, 60), (40, 8)] {
            let bars = Bars::layout(width, rows).unwrap();
            let height = 2 * u32::from(rows);
            // Below the 90% line, where the digits never reach, and above the edge.
            assert!(bars.top * 10 >= height * 9 - 10, "{width}x{rows}: {bars:?}");
            assert!(bars.bottom <= height);
            // Four bars, left to right, inside the margins and not overlapping.
            let w = u32::from(width);
            assert!(bars.segments[0].0 >= w / 10 && bars.segments[3].1 <= w - w / 10);
            for pair in bars.segments.windows(2) {
                assert!(pair[0].1 < pair[1].0);
                assert_eq!(pair[0].1 - pair[0].0, pair[1].1 - pair[1].0);
            }
        }
    }

    // ---- drawn over half blocks ----

    const GREY: Color = Color::Rgb(100, 100, 100);

    fn grey_screen(width: u16, rows: u16) -> (Buffer, Rect) {
        let area = Rect::new(0, 0, width, rows);
        let mut buffer = Buffer::empty(area);
        for cell in &mut buffer.content {
            cell.set_char('▀').set_fg(GREY).set_bg(GREY);
        }
        (buffer, area)
    }

    /// Per bar, how many of its samples differ from the plain grey, and how
    /// many of those are lit (brighter than the dim ones).
    fn bar_samples(buffer: &Buffer, bars: &Bars, rows: u16) -> Vec<(usize, usize)> {
        let value = |color: Color| match color {
            Color::Rgb(r, ..) => r,
            _ => 0,
        };
        bars.segments
            .iter()
            .map(|&(left, right)| {
                let mut changed = 0;
                let mut lit = 0;
                for sample_row in 0..2 * u32::from(rows) {
                    for column in left..right {
                        let cell = &buffer[(column as u16, (sample_row / 2) as u16)];
                        let shade = value(if sample_row % 2 == 0 {
                            cell.fg
                        } else {
                            cell.bg
                        });
                        if shade != 100 {
                            changed += 1;
                        }
                        if shade > 150 {
                            lit += 1;
                        }
                    }
                }
                (changed, lit)
            })
            .collect()
    }

    #[test]
    fn drawn_bars_fill_up_through_the_work_period_and_leave_the_rest_alone() {
        let (width, rows) = (100, 30);
        let bars = Bars::layout(width, rows).unwrap();
        let each = (bars.segments[0].1 - bars.segments[0].0) as usize;
        let thickness = (bars.bottom - bars.top) as usize;

        let (mut buffer, area) = grey_screen(width, rows);
        draw_cycle(&mut buffer, area, Phase::Work(2), 0.5);
        let samples = bar_samples(&buffer, &bars, rows);
        // Every bar is drawn: the first fully lit, the second half lit, the
        // others dim.
        for (changed, _) in &samples {
            assert_eq!(*changed, each * thickness);
        }
        assert_eq!(samples[0].1, each * thickness);
        assert_eq!(samples[1].1, (each + 1) / 2 * thickness);
        assert_eq!(samples[2].1, 0);
        assert_eq!(samples[3].1, 0);

        // Nothing outside the strip changed.
        let strip = bars.top / 2..bars.bottom.div_ceil(2);
        for y in 0..rows {
            if strip.contains(&u32::from(y)) {
                continue;
            }
            for x in 0..width {
                assert_eq!((buffer[(x, y)].fg, buffer[(x, y)].bg), (GREY, GREY));
            }
        }

        // In a break after work 2: two bars full, two unlit, none part full.
        let (mut buffer, area) = grey_screen(width, rows);
        draw_cycle(&mut buffer, area, Phase::Break(2), 0.5);
        let lit: Vec<usize> = bar_samples(&buffer, &bars, rows)
            .iter()
            .map(|s| s.1)
            .collect();
        assert_eq!(lit, [each * thickness, each * thickness, 0, 0]);
    }

    #[test]
    fn drawing_the_bars_on_a_tiny_screen_does_nothing() {
        let (mut buffer, area) = grey_screen(30, 6);
        draw_cycle(&mut buffer, area, Phase::Work(1), 0.5);
        assert!(buffer.content.iter().all(|c| c.fg == GREY && c.bg == GREY));
    }

    // ---- the Jev prompt and gradient speed ----

    #[test]
    fn the_prompt_edits_text_within_limits() {
        let mut prompt = Prompt::default();
        for character in "a sunset\n over\tthe sea".chars() {
            prompt.push(character);
        }
        // Control characters are dropped.
        assert_eq!(prompt.text, "a sunset overthe sea");
        prompt.paste("\u{7}!\n?");
        assert_eq!(prompt.text, "a sunset overthe sea!?");

        // At most 1000 characters, counting characters and not bytes.
        let mut long = Prompt::default();
        long.paste(&"é".repeat(1500));
        assert_eq!(long.text.chars().count(), MAX_PROMPT_CHARS);
        long.push('x');
        assert_eq!(long.text.chars().count(), MAX_PROMPT_CHARS);
    }

    #[test]
    fn the_prompt_shows_the_end_of_the_text_that_fits() {
        let mut prompt = Prompt::default();
        prompt.paste("glacier ice");
        assert_eq!(prompt.tail(40), "glacier ice");
        assert_eq!(prompt.tail(5), "r ice");
        assert_eq!(prompt.tail(0), "");
        // Wide characters count as two columns.
        let mut wide = Prompt::default();
        wide.paste("日本語");
        assert_eq!(wide.tail(4), "本語");
        assert_eq!(wide.tail(5), "本語");
    }

    /// The prompt drawn over grey half blocks, with the clock's digits laid out
    /// in the same area, and the row it was given.
    fn draw(prompt: &Prompt, width: u16, rows: u16) -> (Buffer, Option<Rect>, Rect) {
        let area = Rect::new(0, 0, width, rows);
        let mut clock = digits("25:49").unwrap();
        let bounds = clock.bounds(area);
        let mut terminal = Terminal::new(TestBackend::new(width, rows)).unwrap();
        terminal
            .draw(|frame| {
                for cell in &mut frame.buffer_mut().content {
                    cell.set_char('▀').set_fg(GREY).set_bg(GREY);
                }
                if let Some(row) = prompt_row(area, bounds) {
                    draw_prompt(frame, row, prompt);
                }
            })
            .unwrap();
        let row = prompt_row(area, bounds).unwrap();
        (terminal.backend().buffer().clone(), bounds, row)
    }

    fn row_text(buffer: &Buffer, row: Rect) -> String {
        (row.left()..row.right())
            .map(|column| buffer[(column, row.y)].symbol())
            .collect::<String>()
    }

    #[test]
    fn the_prompt_line_sits_above_the_digits_lined_up_with_them() {
        let area = Rect::new(0, 0, 100, 30);
        let digits = Rect::new(20, 10, 60, 9);
        // A blank row between them when there is room...
        assert_eq!(
            prompt_row(area, Some(digits)),
            Some(Rect::new(20, 8, 60, 1))
        );
        // ...directly above when there is only one row...
        let tight = Rect::new(20, 1, 60, 9);
        assert_eq!(prompt_row(area, Some(tight)), Some(Rect::new(20, 0, 60, 1)));
        // ...and along the top edge when there is no room above, or no digits.
        let flush = Rect::new(20, 0, 60, 9);
        assert_eq!(prompt_row(area, Some(flush)), Some(Rect::new(0, 0, 100, 1)));
        assert_eq!(prompt_row(area, None), Some(Rect::new(0, 0, 100, 1)));
        assert_eq!(prompt_row(Rect::new(0, 0, 0, 0), None), None);
        // An offset screen keeps its offset.
        let offset = Rect::new(5, 3, 100, 30);
        assert_eq!(prompt_row(offset, None), Some(Rect::new(5, 3, 100, 1)));
        assert_eq!(
            prompt_row(offset, Some(Rect::new(25, 13, 60, 9))),
            Some(Rect::new(25, 11, 60, 1))
        );
    }

    #[test]
    fn the_prompt_is_only_a_chevron_and_the_text_just_above_the_clock() {
        let mut prompt = Prompt {
            open: true,
            ..Prompt::default()
        };
        prompt.paste("misty forest");
        let (buffer, digits, row) = draw(&prompt, 100, 30);
        let digits = digits.unwrap();
        assert_eq!(row.x, digits.x);
        assert_eq!(row.width, digits.width);
        assert!(row.bottom() < digits.y);

        // A chevron, the text and a cursor, and nothing else.
        assert!(row_text(&buffer, row).starts_with("> misty forest "));
        assert_eq!(row_text(&buffer, row).trim_end(), "> misty forest");
        let cursor = row.x + 2 + 12;
        assert!(
            buffer[(cursor, row.y)]
                .modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            !buffer[(cursor - 1, row.y)]
                .modifier
                .contains(Modifier::REVERSED)
        );
        // Opaque black behind it, for legibility.
        assert!((row.left()..row.right()).all(|x| buffer[(x, row.y)].bg == Color::Black));
        // Everything else on the screen is untouched.
        for y in 0..30 {
            for x in 0..100 {
                if y != row.y || !(row.left()..row.right()).contains(&x) {
                    assert_eq!((buffer[(x, y)].fg, buffer[(x, y)].bg), (GREY, GREY));
                }
            }
        }
    }

    #[test]
    fn a_long_prompt_scrolls_to_show_its_end() {
        let mut prompt = Prompt {
            open: true,
            ..Prompt::default()
        };
        prompt.paste(
            "a misty forest at dawn with a long thin river running quietly through it \
             and a small wooden bridge over the water near the very end",
        );
        let (buffer, _, row) = draw(&prompt, 100, 30);
        let text = row_text(&buffer, row);
        assert!(text.starts_with("> ") && text.contains("near the very end"));
        assert!(!text.contains("misty"));
    }

    #[test]
    fn gradients_play_at_eight_times_the_base_rate_by_default() {
        let mut view = GradientView::new(&presets::presets()[1].recipe, Instant::now()).unwrap();
        let mut speed = SpeedControl::new(BASE_SPEED);
        speed.change(DEFAULT_SPEED_STEPS, &mut view).unwrap();
        assert!((speed.multiplier() - 8.0).abs() < 1e-9);
        // 8 times Jev's starting rate: the 1.2 radians per second the clock
        // has always used.
        assert!((BASE_SPEED * speed.multiplier() - 1.2).abs() < 1e-9);
        // Up and Down still move it from there.
        speed.change(1, &mut view).unwrap();
        assert!(speed.multiplier() > 11.0 && speed.multiplier() < 11.4);
    }

    fn generated(recipe: amber::Recipe, radians_per_second: f64) -> GeneratedGradient {
        GeneratedGradient {
            recipe,
            radians_per_second,
            design: amber::generation::Design {
                hues: ["blue".into(), "pink".into(), "amber".into()],
                top: "blue".into(),
                layout: amber::generation::Layout::Sky,
                lightness: 2.0,
                saturation: 2.0,
                contrast: 1.5,
                energy: 2.0,
                focus: 1.5,
            },
            cost_usd: None,
        }
    }

    #[test]
    fn a_generated_gradient_replaces_the_picture_and_keeps_the_chosen_speed() {
        let mut view = GradientView::new(&presets::presets()[1].recipe, Instant::now()).unwrap();
        let mut speed = SpeedControl::new(BASE_SPEED);
        speed.change(DEFAULT_SPEED_STEPS, &mut view).unwrap();
        let render = |view: &mut GradientView| {
            let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
            terminal.draw(|f| view.render(f, f.area())).unwrap();
            terminal.backend().buffer().clone()
        };
        let before = render(&mut view);

        let gradient = generated(presets::presets()[0].recipe.clone(), 0.3);
        apply(&mut view, &mut speed, &gradient).unwrap();
        assert_ne!(render(&mut view), before);
        // Still 8x, now of the rate Jev suggested for this gradient.
        assert!((speed.multiplier() - 8.0).abs() < 1e-9);
    }

    #[test]
    fn the_description_stays_until_a_new_one_replaces_it() {
        let mut prompt = Prompt::default();
        assert!(!prompt.has_line());
        prompt.begin();
        assert!(prompt.has_line());

        // Sending a description: it shows once the gradient has arrived.
        prompt.paste("  a misty forest  ");
        prompt.started();
        assert_eq!(prompt.shown, None);
        prompt.paste("x"); // typed while waiting
        prompt.succeeded();
        assert_eq!(prompt.shown.as_deref(), Some("a misty forest"));
        assert!(!prompt.open && prompt.has_line());
        assert_eq!(prompt.text, "");

        // Opening the box again starts empty, and cancelling leaves the
        // description on screen as it was.
        prompt.begin();
        assert!(prompt.open);
        assert_eq!(prompt.text, "");
        assert_eq!(prompt.shown.as_deref(), Some("a misty forest"));
        prompt.paste("with fog");
        prompt.cancel();
        assert_eq!(prompt.text, "");
        assert_eq!(prompt.shown.as_deref(), Some("a misty forest"));
        assert!(!prompt.open && prompt.has_line());

        // A failed request leaves the old gradient and description alone, and
        // the box open with what was typed so it can be sent again.
        prompt.begin();
        prompt.paste("something else");
        prompt.started();
        prompt.failed();
        assert!(prompt.open);
        assert_eq!(prompt.text, "something else");
        assert_eq!(prompt.shown.as_deref(), Some("a misty forest"));
        assert_eq!(prompt.in_flight, None);

        // A new description replaces it, and only then.
        prompt.started();
        prompt.succeeded();
        assert_eq!(prompt.shown.as_deref(), Some("something else"));
        assert_eq!(prompt.text, "");
    }

    #[test]
    fn a_startup_description_does_not_disturb_a_box_being_typed_in() {
        let mut prompt = Prompt::default();
        prompt.started_automatically("Friday morning in May".into());
        prompt.begin();
        prompt.paste("half a thought");
        prompt.succeeded();
        assert_eq!(prompt.shown.as_deref(), Some("Friday morning in May"));
        assert!(prompt.open);
        assert_eq!(prompt.text, "half a thought");

        // A failed one shows nothing and leaves the box alone too; the next
        // typed request behaves as usual.
        let mut prompt = Prompt::default();
        prompt.started_automatically("Friday morning in May".into());
        prompt.failed();
        assert_eq!(prompt.shown, None);
        assert!(!prompt.has_line());
        prompt.begin();
        prompt.paste("a misty forest");
        prompt.started();
        prompt.succeeded();
        assert!(!prompt.open);
        assert_eq!(prompt.text, "");
        assert_eq!(prompt.shown.as_deref(), Some("a misty forest"));
    }

    #[test]
    fn the_command_line_picks_the_place_to_describe() {
        let parse = |args: &[&str]| parse_args(args.iter().map(|a| a.to_string()));
        assert_eq!(parse(&[]), Ok(Command::Clock(Some(Place::Here))));
        assert_eq!(parse(&["--no-weather"]), Ok(Command::Clock(None)));
        assert_eq!(parse(&["--alarm-test"]), Ok(Command::AlarmTest));
        let lisbon = Ok(Command::Clock(Some(Place::Named("Lisbon".into()))));
        assert_eq!(parse(&["--location", "Lisbon"]), lisbon);
        assert_eq!(parse(&["--location=Lisbon"]), lisbon);
        // The last choice wins.
        assert_eq!(
            parse(&["--location", "Lisbon", "--no-weather"]),
            Ok(Command::Clock(None))
        );
        assert!(parse(&["--location"]).is_err());
        assert!(parse(&["--location", " "]).is_err());
        assert!(parse(&["--location="]).is_err());
        assert!(parse(&["--bogus"]).unwrap_err().contains("--bogus"));
    }

    #[test]
    fn opening_the_box_always_starts_empty() {
        let mut prompt = Prompt {
            text: "left over".into(),
            shown: Some("a quiet harbor".into()),
            ..Prompt::default()
        };
        prompt.begin();
        assert_eq!(prompt.text, "");
        assert_eq!(prompt.shown.as_deref(), Some("a quiet harbor"));
        // While the box is open and empty, the line is the empty edit box, not
        // the description.
        assert!(prompt.open && prompt.has_line());
    }

    #[test]
    fn cancelling_with_nothing_shown_empties_the_box_and_the_line() {
        let mut prompt = Prompt::default();
        prompt.open = true;
        prompt.paste("never sent");
        prompt.cancel();
        assert_eq!(prompt.text, "");
        assert!(!prompt.has_line());
        // A late arrival with nothing in flight changes nothing.
        prompt.succeeded();
        assert_eq!(prompt.shown, None);
    }

    #[test]
    fn the_caption_shows_the_start_and_marks_where_it_is_cut() {
        let with = |text: &str| Prompt {
            shown: Some(text.into()),
            ..Prompt::default()
        };
        assert_eq!(with("glacier ice").caption(40), "glacier ice");
        assert_eq!(with("glacier ice").caption(11), "glacier ice");
        let cut = with("glacier ice").caption(8);
        assert_eq!(cut, "glacier…");
        assert_eq!(cut.width(), 8);
        // Wide characters are counted by columns, and never overrun them.
        let wide = with("日本語の氷河").caption(7);
        assert!(wide.width() <= 7 && wide.ends_with('…'), "{wide:?}");
        assert_eq!(with("x").caption(0), "…");
        assert_eq!(Prompt::default().caption(10), "");
    }

    #[test]
    fn once_set_the_description_is_bare_centered_text_above_the_clock() {
        let prompt = Prompt {
            text: "misty forest".into(),
            shown: Some("misty forest".into()),
            ..Prompt::default()
        };
        assert!(!prompt.open && prompt.has_line());
        let (buffer, digits, row) = draw(&prompt, 100, 30);
        assert!(row.bottom() < digits.unwrap().y);

        // No chevron, centered over the clock's width.
        let text = row_text(&buffer, row);
        let left = (row.width - 12) / 2;
        assert!(!text.contains('>'));
        assert_eq!(
            text,
            format!(
                "{}misty forest{}",
                "▀".repeat(left.into()),
                "▀".repeat((row.width - 12 - left).into())
            )
        );
        // No cursor. The gradient shows through darkened: flat under the text
        // (both halves averaged), and still two-toned in the padding around it.
        let text_cells = row.x + left..row.x + left + 12;
        let shade = |color: Color| match color {
            Color::Rgb(r, ..) => r,
            _ => panic!("{color:?}"),
        };
        for x in text_cells.clone() {
            let cell = &buffer[(x, row.y)];
            assert!(!cell.modifier.contains(Modifier::REVERSED));
            assert!(shade(cell.bg) > 0 && shade(cell.bg) < 100, "column {x}");
        }
        assert_eq!(buffer[(text_cells.start, row.y)].fg, Color::White);
        assert!(
            buffer[(text_cells.start, row.y)]
                .modifier
                .contains(Modifier::BOLD)
        );
        // The padding is dimmed but still half blocks.
        let pad = SCRIM_PADDING;
        for x in [
            text_cells.start - 1,
            text_cells.start - pad,
            text_cells.end,
            text_cells.end + pad - 1,
        ] {
            let cell = &buffer[(x, row.y)];
            assert_eq!(cell.symbol(), "▀", "column {x}");
            assert!(shade(cell.fg) > 0 && shade(cell.fg) < 100, "column {x}");
        }
        // Past the padding nothing has changed.
        for x in [text_cells.start - pad - 1, text_cells.end + pad] {
            let cell = &buffer[(x, row.y)];
            assert_eq!(
                (cell.symbol(), cell.fg, cell.bg),
                ("▀", GREY, GREY),
                "column {x}"
            );
        }

        // A long description is cut at the end with an ellipsis.
        let long = Prompt {
            shown: Some("a misty forest at dawn ".repeat(8)),
            ..Prompt::default()
        };
        let (buffer, _, row) = draw(&long, 100, 30);
        let text = row_text(&buffer, row);
        assert!(text.starts_with("a misty forest at dawn"));
        // Cut to fill the clock's width, with the ellipsis as its last character.
        assert!(text.ends_with("…") && !text.contains('▀'), "{text:?}");
    }

    #[test]
    fn the_caption_is_centered_on_a_scrim_and_flattens_only_the_cells_under_the_text() {
        let area = Rect::new(0, 0, 30, 3);
        let (top, bottom) = (Color::Rgb(40, 60, 90), Color::Rgb(100, 140, 200));
        let mut buffer = Buffer::empty(area);
        for cell in &mut buffer.content {
            cell.set_char('▀').set_fg(top).set_bg(bottom);
        }
        // "ab cd" is 5 wide in a row of 20 starting at column 2: it starts at 9.
        draw_caption(&mut buffer, Rect::new(2, 1, 20, 1), "ab cd");
        assert_eq!(row_text(&buffer, Rect::new(9, 1, 5, 1)), "ab cd");

        let rgb = |color: Color| match color {
            Color::Rgb(r, g, b) => [r, g, b],
            _ => panic!("{color:?}"),
        };
        let darker = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(a, b)| *a < b);
        // Under the letters and the space, one flat color: darker than both
        // original halves' average.
        let flat = rgb(buffer[(9, 1)].bg);
        for x in 9..14 {
            assert_eq!(rgb(buffer[(x, 1)].bg), flat, "column {x}");
        }
        assert!(darker(flat, [70, 100, 145]));
        // In the padding both halves are kept and both are darker.
        for x in [7, 8, 14, 15] {
            let cell = &buffer[(x, 1)];
            assert_eq!(cell.symbol(), "▀");
            assert!(darker(rgb(cell.fg), [40, 60, 90]) && darker(rgb(cell.bg), [100, 140, 200]));
            assert_ne!(cell.fg, cell.bg);
        }
        // Beyond the padding, and on other rows, nothing changed.
        for (x, y) in [(6, 1), (16, 1), (2, 1), (21, 1), (9, 0), (9, 2)] {
            let cell = &buffer[(x, y)];
            assert_eq!(
                (cell.symbol(), cell.fg, cell.bg),
                ("▀", top, bottom),
                "({x}, {y})"
            );
        }
        // Text longer than the row is cut to it, and the scrim stops at the
        // buffer's edge. A row off the buffer, or empty text, is ignored.
        draw_caption(&mut buffer, Rect::new(26, 0, 4, 1), "something long");
        assert_eq!(row_text(&buffer, Rect::new(26, 0, 4, 1)), "some");
        draw_caption(&mut buffer, Rect::new(0, 9, 5, 1), "off");
        let before = buffer.clone();
        draw_caption(&mut buffer, Rect::new(0, 2, 10, 1), "");
        assert_eq!(buffer, before);
    }

    #[test]
    fn each_phase_change_announces_itself_with_the_matching_chime() {
        let say = |phase| announcement(phase);
        assert_eq!(
            say(Phase::Break(1)),
            (
                alarm::Chime::WorkDone,
                "Time for a 5 minute break".to_owned()
            )
        );
        assert_eq!(
            say(Phase::Break(4)),
            (
                alarm::Chime::WorkDone,
                "Time for a 20 minute break".to_owned()
            )
        );
        assert_eq!(
            say(Phase::Work(2)),
            (
                alarm::Chime::BreakDone,
                "Back to work: period 2 of 4, 25 minutes".to_owned()
            )
        );
        assert_eq!(say(Phase::Work(1)).0, alarm::Chime::BreakDone);
    }

    #[test]
    fn advance_reports_a_change_exactly_when_a_phase_ends() {
        let start = Instant::now();
        let mut clock = Clock::new(start);
        let after = |seconds: u64| start + Duration::from_secs(seconds);
        // Nothing to announce at the start or just before the end.
        assert!(!clock.advance(start));
        assert!(!clock.advance(after(25 * 60 - 1)));
        // The work period ends: announce once, not again on the next pass.
        assert!(clock.advance(after(25 * 60)));
        assert_eq!(clock.phase(), Phase::Break(1));
        assert!(!clock.advance(after(25 * 60 + 1)));
        // Skipping several phases at once (a suspended laptop) is one change.
        assert!(clock.advance(after(100 * 60)));
        assert_eq!(clock.phase(), Phase::Work(4));
        // A paused clock never changes, however long it sits.
        clock.toggle_pause(after(100 * 60));
        assert!(!clock.advance(after(1000 * 60)));
        // Resetting is not a phase ending.
        clock.reset(after(1000 * 60));
        assert!(!clock.advance(after(1000 * 60)));
    }
}
