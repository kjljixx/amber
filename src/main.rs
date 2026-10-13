//! Big translucent digits over an animated gradient, in two views.
//!
//! Home is where it starts: the lyric for the moment over the gradient, and the
//! time of day (`hh:mm`, 24 hour) in the digits. Nothing counts down.
//!
//! Choosing a task with `t` moves to the task view, a pomodoro clock with four
//! bars along the bottom showing which work period of the cycle this is. The
//! clock starts from 25:00 at that point, and the focus checker only runs
//! here. Pressing Enter on an empty prompt (the "Back to Home" row) resets the
//! clock and goes back to home.
//!
//! The cycle is work, 5 minute break, three times over, then a fourth work
//! period and a 20 minute break, and then it starts again.
//!
//! `cargo run --offline`
//! t: choose the task, in a fresh box (Enter takes it, Esc cancels) | Space:
//! pause or resume the timer and the gradient | r: back to the start of the
//! cycle, 25:00 (Space and r work in the task view) | Up/Down: gradient speed |
//! Ctrl+C: quit (q and Esc never quit)
//!
//! The task is what the gradient is made for: Jev is given it along with the
//! moment (see below), and once the gradient is ready the task stays centered
//! above the clock until a new one replaces it. Typing in the box describes a
//! task of your own.
//!
//! `t` chooses from your Todoist account by default (see below).
//! `--tasks list.md` gives it a list from a file instead: the unfinished items of a
//! markdown file, such as `- [ ] write the report`, under whatever headings it
//! has. What is typed narrows the list, Up and Down move through it, and Enter
//! takes the highlighted row. The file is read again each time `t` is pressed.
//!
//! Todoist gives all your active tasks,
//! soonest due first and then most urgent, with when each is due, its labels and
//! its project's name at the right. It needs a personal API token (Todoist's
//! settings, Integrations, Developer) as `TODOIST_API_TOKEN` in the environment
//! or a `.env` file. `--todoist-filter "today | overdue"` reads only the tasks a
//! Todoist filter matches. `--no-todoist` turns it off. Todoist is only ever read,
//! never changed. Without a token the timer still starts and says so.
//!
//! Sound from the machine moves the gradient from the start; `a` turns it off
//! and on, and `--no-audio` starts with it off.
//!
//! The focus checker captures the primary screen every 20 seconds while a
//! chosen task is active during unpaused work phases. It pauses with the timer
//! and during breaks, then starts a fresh 20-second delay when monitoring
//! resumes. GPT-6 Luna Decisions checks whether the image looks off task; Amber
//! sends a desktop notification after two consecutive off-task decisions. Pass
//! `--no-focus-check` to disable it.
//!
//! The first gradient is made for the moment: the weekday, time of day, month
//! and weather, such as "Friday golden hour in October (autumn), light rain,
//! cool". Jev picks the lyric line that best fits it from `assets/phrases.txt`
//! (made by `05_build_phrase_pool.py`), and the gradient is made from that
//! lyric, which is shown above the clock. Every task after that is sent with
//! only a hint of the moment, its light and
//! weather ("golden hour, light rain"), so the task leads. The weather is for
//! wherever the machine's IP address seems to be; `--location Lisbon` names a
//! place instead, and `--no-weather` skips the lookup and uses the time alone.
//! The moment refreshes every five minutes while the timer runs.
//! Only those words go to Jev, never the place.
//!
//! When a phase ends, a chime plays and a desktop notification appears: a
//! climbing one when a work period ends, a falling one when a break does.
//! `cargo run --offline -- --alarm-test` plays both now, to check
//! your sound and notification settings.
//!
//! Jev needs `OPENROUTER_API_KEY` in the environment or a `.env` file; see
//! docs/generation.md.
use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[cfg(feature = "focus-check")]
use amber::focus::{Event as FocusEvent, FocusChecker, OFF_TASK_BODY, OFF_TASK_TITLE};
use amber::{
    Digits, Excitement, GradientView, Reactivity,
    ambient::{Ambient, Moment, Place},
    audio::Source,
    blend_samples, darken_around,
    generation::{GeneratedGradient, Generator},
    presets,
    tasks::{self, Loader, todoist::Todoist},
};
use chrono::{Local, NaiveTime, Timelike};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use tracing::{debug, info, warn};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

mod alarm;
mod picker;
mod speed_control;
use picker::Picker;
use speed_control::SpeedControl;

#[cfg(all(
    feature = "focus-check",
    any(target_os = "macos", target_os = "windows")
))]
mod focus_overlay;

/// Log target for sound the timer is waiting on, which goes to the audio log.
#[cfg(feature = "audio")]
const AUDIO: &str = "amber::audio";
/// Log target for the overlay's events, which go to the focus log.
#[cfg(feature = "focus-check")]
const FOCUS_OVERLAY: &str = "amber::focus_overlay";

const WORK: Duration = Duration::from_secs(25 * 60);
const SHORT_BREAK: Duration = Duration::from_secs(5 * 60);
const LONG_BREAK: Duration = Duration::from_secs(20 * 60);
const WORK_PERIODS: u8 = 4;

/// Opacity of the digits and of the lit part of the bars.
const ALPHA: f32 = 0.6;
/// Opacity of the unlit part of the bars.
const DIM_ALPHA: f32 = 0.25;
/// How far the darkening around the digits and bars reaches, in half-block
/// samples, and the lightest the gradient may be under them (OKLab, 0 to 1).
/// A white text at `ALPHA` is then at least 0.23 lighter than what is behind it.
const HALO_RADIUS: u16 = 6;
const HALO_LIMIT: f32 = 0.8;
const STRONG_HALO_LIMIT: f32 = 0.65;
/// Gradient animation rate before Jev suggests one, in radians per second.
const BASE_SPEED: f64 = 0.15;
/// Speed steps above 1x at the start: each is a factor of the square root of
/// two, so 6 steps is 8x. With the base rate that is 1.2 radians per second.
const DEFAULT_SPEED_STEPS: i32 = 6;
/// Longest task, leaving room under Jev's limit of 1,000 characters for the
/// moment that goes with it.
const MAX_TASK_CHARS: usize = 500;
/// Frame spacing normally, and while sound is moving the gradient: short enough
/// to follow a beat.
const CALM_FRAME: Duration = Duration::from_millis(50);
const LIVELY_FRAME: Duration = Duration::from_millis(33);
/// How long a message about the audio stays at the top.
const NOTICE: Duration = Duration::from_secs(4);
/// How long to listen to silence before suggesting something is wrong.
#[cfg(feature = "audio")]
const SILENCE_HINT_AFTER: Duration = Duration::from_secs(8);
/// How far the caption's backing is darkened, and how many cells it reaches
/// past each end of the text.
const SCRIM_ALPHA: f32 = 0.2;
const SCRIM_PADDING: u16 = 2;
const PURE_WHITE: &str = "pure white";
const WHITE: [u8; 3] = [255, 255, 255];
const PAUSED_ROSE: [u8; 3] = [255, 150, 150];
const BREAK_MINT: [u8; 3] = [150, 255, 150];
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

/// What the screen is for. Home is the ambient view: the lyric over the gradient
/// and the time of day. Task is the pomodoro for a chosen task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Home,
    Task,
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

/// `hh:mm` on a 24 hour clock.
fn clock_label(time: NaiveTime) -> String {
    format!("{:02}:{:02}", time.hour(), time.minute())
}

/// How long until `clock_label` shows a different value.
fn until_minute_changes(time: NaiveTime) -> Duration {
    Duration::from_secs(60 - u64::from(time.second()))
        .saturating_sub(Duration::from_nanos(time.nanosecond().into()))
}

/// The digits for `view`: the time of day at home, the time left in a task.
fn view_label(view: View, clock: &Clock, now: Instant) -> String {
    match view {
        View::Home => clock_label(Local::now().time()),
        View::Task => label(clock.remaining(now)),
    }
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

    /// Which bar the sample belongs to, if any.
    fn segment_at(&self, column: u32, row: u32) -> Option<usize> {
        if !(self.top..self.bottom).contains(&row) {
            return None;
        }
        self.segments
            .iter()
            .position(|(left, right)| (*left..*right).contains(&column))
    }

    /// `Some(true)` for a lit sample of a bar, `Some(false)` for an unlit one,
    /// `None` when the sample is not part of a bar.
    fn lit(&self, fill: &[f32; 4], column: u32, row: u32) -> Option<bool> {
        let index = self.segment_at(column, row)?;
        let (left, right) = self.segments[index];
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

/// Darken the gradient around the digits and the bars wherever it is too
/// pale for them to stand out, softly, and only as far as needed.
fn draw_halo(
    buffer: &mut Buffer,
    area: Rect,
    text: &mut Digits,
    stronger_halo: bool,
    with_bars: bool,
) -> usize {
    let bars = with_bars
        .then(|| Bars::layout(area.width, area.height))
        .flatten();
    let digits = text.covered(area);
    let halo_limit = if stronger_halo {STRONG_HALO_LIMIT} else {HALO_LIMIT};
    let mut darkened_samples = darken_around(buffer, area, HALO_RADIUS, halo_limit, |column, row| {
        digits(column, row)
    });
    darkened_samples += darken_around(buffer, area, HALO_RADIUS, HALO_LIMIT, |column, row| {
        bars
                .as_ref()
                .is_some_and(|bars| bars.segment_at(column.into(), row.into()).is_some())
    });
    return darkened_samples;
}

/// The text box for describing a gradient, and the description behind the
/// gradient on screen.
#[derive(Default)]
struct TaskLine {
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

impl TaskLine {
    fn push(&mut self, character: char) {
        if !character.is_control() && self.text.chars().count() < MAX_TASK_CHARS {
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

    /// A request has started for `task`: the one chosen, or the text typed.
    fn started_for(&mut self, task: String) {
        self.in_flight = Some(task);
        self.automatic = false;
    }

    /// A request has started for `description`, made without being typed.
    fn started_automatically(&mut self, description: String) {
        self.in_flight = Some(description);
        self.automatic = true;
    }

    /// The request in flight turned out to be made from `description`, such as
    /// the lyric chosen for the moment, and that is what is shown once it arrives.
    fn describe_as(&mut self, description: String) {
        if self.in_flight.is_some() {
            self.in_flight = Some(description);
        }
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
        fit(self.shown.as_deref().unwrap_or_default(), columns)
    }
}

/// The start of `text` that fits in `columns` terminal columns, ending in `…`
/// if it is cut.
fn fit(text: &str, columns: usize) -> String {
    if text.width() <= columns {
        return text.to_owned();
    }
    let mut fitted = String::new();
    for character in text.chars() {
        // Leave a column for the ellipsis.
        if fitted.width() + character.width().unwrap_or(0) >= columns {
            break;
        }
        fitted.push(character);
    }
    fitted.push('…');
    fitted
}

/// Where the task line goes: just above the digits and lined up with them,
/// with a blank row between when there is room. With nowhere above the digits,
/// or no digits on screen, it takes the top row.
fn task_row(area: Rect, digits: Option<Rect>) -> Option<Rect> {
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

/// The task line. While the box is open it is a chevron and the text being
/// edited on an opaque black line, with a cursor, so it stays legible while
/// typing. Otherwise it is the description behind the current gradient, as bare
/// text centered right on the gradient.
fn draw_task_line(frame: &mut Frame, row: Rect, task: &TaskLine) {
    if !task.open {
        draw_caption(
            frame.buffer_mut(),
            row,
            &task.caption(usize::from(row.width)),
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
        Span::raw(format!("› {}", task.tail(columns))),
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
    let style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    put_text(buffer, left, row.y, text, width, style);
}

/// Write `text`, at most `width` columns of it, starting at `left` on row `y`.
/// Over half blocks, a letter's cell takes the average of the two halves'
/// colors behind it, as one background has to serve for both.
fn put_text(buffer: &mut Buffer, left: u16, y: u16, text: &str, width: u16, style: Style) {
    let backgrounds: Vec<Option<Color>> = (left..left.saturating_add(width))
        .map(|x| {
            let cell = &buffer[(x, y)];
            match (cell.fg, cell.bg) {
                (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) if cell.symbol() == "▀" => {
                    let mean = |a: u8, b: u8| ((u16::from(a) + u16::from(b)) / 2) as u8;
                    Some(Color::Rgb(mean(r1, r2), mean(g1, g2), mean(b1, b2)))
                }
                _ => None,
            }
        })
        .collect();
    buffer.set_stringn(left, y, text, usize::from(width), style);
    for (x, background) in (left..).zip(backgrounds) {
        if let Some(background) = background {
            buffer[(x, y)].set_bg(background);
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
    view.set_reactivity(reactivity_for(gradient.design.energy));
    speed.set_base(gradient.radians_per_second, view)
}

/// How hard sound pushes the picture: the warp and the steering follow Jev's
/// energy score (0 to 4), at 0.6 times the default for a still, calm gradient
/// up to 1.2 times for a very energetic one. The speed boost is added in radians
/// per second and stays the same for all of them.
fn reactivity_for(energy: f32) -> Reactivity {
    let scale = 0.6 + 0.15 * energy.clamp(0.0, 4.0);
    let normal = Reactivity::default();
    Reactivity {
        warp: normal.warp * scale,
        steer: normal.steer * scale,
        ..normal
    }
}

/// What Jev is told: the task, with a hint of the moment when that is known.
/// The moment is a few words of light and weather, after the task, so it tints
/// the gradient and does not decide it: a task says little about color, and a
/// whole description of the moment would say much more.
fn gradient_description(task: &str, moment: Option<&Moment>) -> String {
    match moment {
        Some(moment) => format!("{task}, with a hint of {}", moment.brief),
        None => task.to_owned(),
    }
}

/// A list to choose from made of what `loader` has so far: its tasks, or
/// in their place why there are none yet.
fn picker_for(loader: &Loader) -> Picker {
    Picker::new(
        loader.tasks().unwrap_or_default().to_vec(),
        loader.message(),
    )
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
    info!(?phase, message, "phase announced");
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

fn timer_color(view: View, phase: Phase, paused: bool) -> [u8; 3] {
    if view == View::Home {
        WHITE
    } else if paused {
        PAUSED_ROSE
    } else if matches!(phase, Phase::Break(_)) {
        BREAK_MINT
    } else {
        WHITE
    }
}

fn main() -> std::process::ExitCode {
    #[cfg(all(
        feature = "focus-check",
        any(target_os = "macos", target_os = "windows")
    ))]
    if let Some((message, accent, task)) = focus_overlay_message() {
        return match focus_overlay::run(message, accent, task) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("amber overlay: {error}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("amber: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(all(
    feature = "focus-check",
    any(target_os = "macos", target_os = "windows")
))]
fn focus_overlay_message() -> Option<(String, Option<amber::Rgb>, Option<String>)> {
    let mut args = std::env::args().skip(1);
    if args.next()?.as_str() != "--focus-overlay" {
        return None;
    }
    let message = args.next().unwrap_or_else(|| OFF_TASK_TITLE.into());
    let (mut accent, mut task) = (None, None);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--accent" => {
                accent = args.next().and_then(|hex| {
                    let digits = u32::from_str_radix(hex.strip_prefix('#')?, 16).ok()?;
                    digits.to_be_bytes()[1..].try_into().ok()
                });
            }
            "--task" => task = args.next(),
            _ => {}
        }
    }
    Some((message, accent, task))
}

/// What the command line asks for.
#[derive(Debug, PartialEq)]
enum Command {
    AlarmTest,
    ListAudio,
    Clock(Options),
}

#[derive(Debug, PartialEq)]
struct Options {
    /// Start from a description of this moment and this place's weather; with no
    /// place, of the time alone.
    place: Option<Place>,
    /// Listen for sound from the start; `a` turns it off or on.
    listen: bool,
    source: Source,
    sensitivity: f32,
    /// A markdown file of tasks to choose from.
    tasks: Option<PathBuf>,
    /// Choose from the tasks in a Todoist account, by this filter or its usual
    /// one, unless a markdown file of tasks is named.
    todoist: bool,
    todoist_filter: Option<String>,
    /// Check screenshots against the selected task, every five seconds after
    /// an off-task result and every twenty seconds otherwise.
    focus_check: bool,
    /// Temporary, for testing contrast: start from the description "pure white".
    pure_white: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            place: Some(Place::Here),
            listen: true,
            source: Source::System,
            sensitivity: 1.0,
            tasks: None,
            todoist: true,
            todoist_filter: None,
            focus_check: cfg!(feature = "focus-check"),
            pure_white: false,
        }
    }
}

const USAGE: &str = "options: --location <city> | --no-weather | --no-audio | \
--audio-from <system|mic|device> | --sensitivity <number> | --tasks <markdown file> | \
--no-todoist | --todoist-filter <query> | --no-focus-check | --pure-white | --debug | --list-audio | --alarm-test";

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let mut options = Options::default();
    let mut todoist_requested = false;
    while let Some(arg) = args.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) => (name.to_string(), Some(value.to_string())),
            None => (arg.clone(), None),
        };
        let mut value = |what: &str| {
            inline
                .clone()
                .or_else(|| args.next())
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("{name} needs {what}"))
        };
        match name.as_str() {
            "--alarm-test" => return Ok(Command::AlarmTest),
            "--list-audio" => return Ok(Command::ListAudio),
            "--no-focus-check" => options.focus_check = false,
            "--pure-white" => options.pure_white = true,
            "--no-weather" => options.place = None,
            "--no-audio" => options.listen = false,
            "--location" => {
                options.place = Some(Place::Named(value("a place, such as --location Lisbon")?));
            }
            "--audio-from" => {
                options.listen = true;
                options.source = match value("system, mic or part of a device's name")?.as_str() {
                    "system" => Source::System,
                    "mic" | "microphone" => Source::Microphone,
                    device => Source::Named(device.to_string()),
                };
            }
            "--no-todoist" => options.todoist = false,
            "--todoist" => {
                options.todoist = true;
                todoist_requested = true;
            }
            "--todoist-filter" => {
                options.todoist = true;
                todoist_requested = true;
                options.todoist_filter =
                    Some(value("a Todoist filter, such as \"today | overdue\"")?);
            }
            "--tasks" => {
                options.tasks = Some(PathBuf::from(value("a markdown file of tasks")?));
            }
            "--sensitivity" => {
                options.sensitivity = value("a number such as 2")?
                    .parse::<f32>()
                    .ok()
                    .filter(|number| number.is_finite() && *number > 0.0)
                    .ok_or("--sensitivity needs a positive number such as 2")?;
            }
            _ => return Err(format!("unknown argument '{arg}'; {USAGE}")),
        }
        if inline.is_some()
            && !matches!(
                name.as_str(),
                "--location" | "--audio-from" | "--sensitivity" | "--tasks" | "--todoist-filter"
            )
        {
            return Err(format!("{name} takes no value; {USAGE}"));
        }
    }
    if options.tasks.is_some() {
        if todoist_requested {
            return Err("--tasks and --todoist both choose where tasks come from; use one".into());
        }
        options.todoist = false;
    }
    Ok(Command::Clock(options))
}

/// What the machine's sound is doing, and anything worth telling the user.
#[derive(Default)]
struct Heard {
    excitement: Excitement,
    notice: Option<String>,
    /// Listening cannot go on; drop the listener.
    stop: bool,
}

/// Turns the machine's sound into excitement for the gradient.
#[cfg(feature = "audio")]
struct Listener {
    capture: amber::audio::Capture,
    started: Option<Instant>,
    hinted: bool,
}

#[cfg(feature = "audio")]
impl Listener {
    fn start(source: Source, sensitivity: f32) -> Result<Self, String> {
        Ok(Self {
            capture: amber::audio::Capture::start(source, sensitivity),
            started: None,
            hinted: false,
        })
    }

    fn poll(&mut self, now: Instant) -> Heard {
        use amber::audio::Status;
        let mut heard = Heard::default();
        match self.capture.status().clone() {
            Status::Starting => return heard,
            Status::Failed(message) => {
                heard.notice = Some(format!("Audio: {message}"));
                heard.stop = true;
                return heard;
            }
            Status::Listening(device) => {
                if self.started.is_none() {
                    self.started = Some(now);
                    heard.notice = Some(format!("Listening to {device}"));
                }
            }
        }
        heard.excitement = self.capture.poll();
        let quiet_for = self
            .started
            .map_or(Duration::ZERO, |at| now.duration_since(at));
        if !self.hinted && quiet_for > SILENCE_HINT_AFTER && !self.capture.heard() {
            self.hinted = true;
            warn!(
                target: AUDIO,
                silent_secs = quiet_for.as_secs(),
                "no sound heard since listening started"
            );
            heard.notice = Some(if cfg!(target_os = "macos") {
                "No sound yet. Is something playing? Terminal needs permission to record system audio".into()
            } else {
                "No sound yet. Is something playing?".into()
            });
        }
        heard
    }
}

#[cfg(not(feature = "audio"))]
struct Listener;

#[cfg(not(feature = "audio"))]
impl Listener {
    fn start(_: Source, _: f32) -> Result<Self, String> {
        Err("this build has no audio support (build with --features audio)".into())
    }

    fn poll(&mut self, _: Instant) -> Heard {
        Heard::default()
    }
}

fn list_audio() -> io::Result<()> {
    #[cfg(feature = "audio")]
    {
        for device in amber::audio::list_devices().map_err(io::Error::other)? {
            println!("{device}");
        }
        Ok(())
    }
    #[cfg(not(feature = "audio"))]
    Err(io::Error::other("this build has no audio support"))
}

/// A short message drawn along the top of the screen.
struct Notice {
    text: String,
    until: Instant,
}

fn run() -> io::Result<()> {
    let (debug, args) = amber::log::take_debug_flag(std::env::args().skip(1));
    let options = match parse_args(args).map_err(io::Error::other)? {
        Command::AlarmTest => return alarm_test(),
        Command::ListAudio => return list_audio(),
        Command::Clock(options) => options,
    };
    let log_directory = amber::log::init(debug)?;
    info!(
        ?options,
        log_directory = %log_directory.display(),
        "amber started"
    );
    info!(
        halo_radius = HALO_RADIUS,
        halo_limit = HALO_LIMIT,
        "darkening around the digits and bars"
    );
    let start = Instant::now();
    let mut phases_completed = 0u32;
    let mut frames_drawn = 0u32;
    let mut frames_darkened = 0u32;
    let mut background =
        GradientView::new(&presets::presets()[1].recipe, start).map_err(io::Error::other)?;
    let mut speed = SpeedControl::new(BASE_SPEED);
    speed.change(DEFAULT_SPEED_STEPS, &mut background)?;
    let mut generator = Generator::new();
    let mut ambient = Ambient::start(options.place);
    let mut notice: Option<Notice> = None;
    let mut listener = None;
    if options.listen {
        match Listener::start(options.source.clone(), options.sensitivity) {
            Ok(started) => listener = Some(started),
            Err(message) => {
                notice = Some(Notice {
                    text: format!("Audio: {message}"),
                    until: start + NOTICE,
                });
            }
        }
    }
    let mut lively = false;
    let mut task = TaskLine::default();
    #[cfg(feature = "focus-check")]
    let mut focus_checker = if options.focus_check {
        Some(FocusChecker::from_env(Path::new(".env")).map_err(io::Error::other)?)
    } else {
        None
    };
    #[cfg(feature = "focus-check")]
    let mut focus_overlay: Option<alarm::FocusOverlay> = None;
    #[cfg(not(feature = "focus-check"))]
    if options.focus_check {
        return Err(io::Error::other("rebuild Amber with the focus-check feature").into());
    }
    // The moment, which goes to Jev with every task.
    let mut moment: Option<Moment> = None;
    if options.pure_white {
        info!("starting from the description \"pure white\"");
        generator.submit(PURE_WHITE);
        if generator.is_pending() {
            task.started_for(PURE_WHITE.into());
        }
    }
    let mut picker: Option<Picker> = None;
    let mut loader = match (&options.tasks, options.todoist) {
        (Some(path), _) => Some(Loader::new(tasks::Source::Markdown(path.clone()))),
        (None, true) => {
            match Todoist::from_env(Path::new(".env"), options.todoist_filter.as_deref()) {
                Ok(todoist) => Some(Loader::new(tasks::Source::Todoist(todoist))),
                Err(message) => {
                    notice = Some(Notice {
                        text: format!("Todoist: {message}"),
                        until: start + NOTICE,
                    });
                    None
                }
            }
        }
        (None, false) => None,
    };
    // Start fetching now so the list is there by the time it is wanted.
    if let Some(loader) = loader.as_mut() {
        loader.refresh();
    }
    let mut view = View::Home;
    info!(?view, "starting at home; the pomodoro starts with the first task");
    let mut clock = Clock::new(start);
    #[cfg(feature = "focus-check")]
    let mut focus_work_active = false;
    let mut shown = view_label(view, &clock, start);
    let mut shown_color = timer_color(view, clock.phase(), clock.is_paused());
    let mut text = digits(&shown)?;
    text.set_color(shown_color);
    let mut redraw = true;
    let result = ratatui::run(|terminal| {
        loop {
            let now = Instant::now();
            if view == View::Task && clock.advance(now) {
                phases_completed += 1;
                info!(
                    phase = ?clock.phase(),
                    phases_completed,
                    frames_drawn,
                    frames_darkened,
                    "phase changed"
                );
                announce(clock.phase());
            }
            #[cfg(feature = "focus-check")]
            {
                let work_active = view == View::Task
                    && matches!(clock.phase(), Phase::Work(_))
                    && !clock.is_paused();
                if work_active != focus_work_active {
                    focus_work_active = work_active;
                    info!(
                        target: FOCUS_OVERLAY,
                        work_active,
                        phase = ?clock.phase(),
                        paused = clock.is_paused(),
                        "focus monitoring follows the timer"
                    );
                    if let Some(checker) = focus_checker.as_mut() {
                        checker.set_work_active(work_active, now);
                    }
                    if !work_active {
                        if let Some(overlay) = focus_overlay.take() {
                            info!(
                                target: FOCUS_OVERLAY,
                                reason = if clock.is_paused() { "timer_paused" } else { "break_started" },
                                "overlay closed"
                            );
                            overlay.close();
                        }
                    }
                }
            }
            if let Some(found) = ambient.poll() {
                let description = found.full.clone();
                moment = Some(found);
                // Start from a gradient for the moment, unless a description has
                // already been asked for in the meantime.
                if task.shown.is_none() && task.in_flight.is_none() {
                    task.started_automatically(description.clone());
                    generator.submit_moment(&description);
                    if !generator.is_pending() {
                        task.failed();
                    }
                    redraw = true;
                }
            }
            if let Some(loader) = loader.as_mut() {
                if let Some(result) = loader.poll() {
                    if let Some(picker) = picker.as_mut() {
                        picker.set(
                            loader.tasks().unwrap_or_default().to_vec(),
                            loader.message(),
                        );
                    }
                    // The list in view stays when a refresh fails; say so.
                    if let (Err(message), Some(_)) = (result, loader.tasks()) {
                        notice = Some(Notice {
                            text: format!("Tasks: {message}"),
                            until: now + NOTICE,
                        });
                    }
                    redraw = true;
                }
            }
            if let Some(result) = generator.poll() {
                match result {
                    Ok(gradient) => {
                        apply(&mut background, &mut speed, &gradient)?;
                        if let Some(phrase) = gradient.phrase {
                            task.describe_as(phrase.text);
                        }
                        task.succeeded();
                    }
                    Err(message) => {
                        if task.automatic {
                            notice = Some(Notice {
                                text: message,
                                until: now + NOTICE,
                            });
                        }
                        task.failed();
                    }
                }
                redraw = true;
            }
            #[cfg(feature = "focus-check")]
            if focus_overlay
                .as_mut()
                .is_some_and(|overlay| !overlay.is_running())
            {
                info!(target: FOCUS_OVERLAY, reason = "process_exited", "overlay closed");
                focus_overlay = None;
            }
            #[cfg(feature = "focus-check")]
            if let Some(checker) = focus_checker.as_mut() {
                match checker.poll(now) {
                    Some(FocusEvent::OffTask) => {
                        info!(target: FOCUS_OVERLAY, "off-task reminder");
                        alarm::notify(OFF_TASK_TITLE, OFF_TASK_BODY);
                        if focus_overlay.is_none() {
                            match alarm::show_focus_overlay(
                                OFF_TASK_TITLE,
                                background.accent(),
                                checker.task(),
                            ) {
                                Ok(Some(overlay)) => {
                                    info!(target: FOCUS_OVERLAY, "overlay opened");
                                    focus_overlay = Some(overlay);
                                }
                                Ok(None) => {
                                    info!(target: FOCUS_OVERLAY, "overlay unavailable on this platform");
                                }
                                Err(error) => {
                                    warn!(target: FOCUS_OVERLAY, %error, "overlay could not open");
                                }
                            }
                        } else {
                            debug!(target: FOCUS_OVERLAY, "overlay already open");
                        }
                        notice = Some(Notice {
                            text: format!("{OFF_TASK_TITLE} - {OFF_TASK_BODY}"),
                            until: now + 2 * NOTICE,
                        });
                        redraw = true;
                    }
                    Some(FocusEvent::OnTask) if focus_overlay.is_some() => {
                        if let Some(overlay) = focus_overlay.take() {
                            info!(target: FOCUS_OVERLAY, reason = "on_task", "overlay closed");
                            overlay.close();
                        }
                        redraw = true;
                    }
                    Some(FocusEvent::OnTask) => {}
                    Some(FocusEvent::Error(message)) => {
                        warn!(target: FOCUS_OVERLAY, %message, "focus check reported an error");
                        notice = Some(Notice {
                            text: format!("Focus check: {message}"),
                            until: now + 2 * NOTICE,
                        });
                        redraw = true;
                    }
                    None => {}
                }
            }
            let current = view_label(view, &clock, now);
            if current != shown {
                text = digits(&current)?;
                text.set_color(shown_color);
                shown = current;
                redraw = true;
            }
            let current_color = timer_color(view, clock.phase(), clock.is_paused());
            if current_color != shown_color {
                shown_color = current_color;
                text.set_color(shown_color);
                redraw = true;
            }
            if notice.as_ref().is_some_and(|notice| now >= notice.until) {
                notice = None;
                redraw = true;
            }
            let mut excitement = Excitement::default();
            let mut stop = false;
            if let Some(active) = listener.as_mut() {
                let heard = active.poll(now);
                excitement = heard.excitement;
                stop = heard.stop;
                if let Some(text) = heard.notice {
                    let until = now + if stop { 2 * NOTICE } else { NOTICE };
                    notice = Some(Notice { text, until });
                    redraw = true;
                }
            }
            if stop {
                listener = None;
            }
            background.set_excitement(excitement);
            // Follow the beat with shorter frames, and go back to idle pacing in silence.
            let busy =
                excitement.level > 0.02 || excitement.pulse > 0.02 || excitement.activity > 0.02;
            if busy != lively {
                lively = busy;
                speed.set_interval(
                    if busy { LIVELY_FRAME } else { CALM_FRAME },
                    &mut background,
                )?;
            }
            redraw |= background.update(now);
            if redraw {
                terminal.draw(|frame| {
                    let area = frame.area();
                    // The gradient first, then the time and the bars over it.
                    background.render(frame, area);
                    frames_drawn += 1;
                    let task_view = view == View::Task;
                    let stronger_halo =
                        task_view && (matches!(clock.phase(), Phase::Break(_)) || clock.is_paused());
                    if draw_halo(frame.buffer_mut(), area, &mut text, stronger_halo, task_view) > 0 {
                        frames_darkened += 1;
                    }
                    frame.render_widget(&mut text, area);
                    if task_view {
                        draw_cycle(frame.buffer_mut(), area, clock.phase(), clock.progress(now));
                    }
                    // The task goes over everything else, above the digits.
                    if task.has_line() {
                        if let Some(row) = task_row(area, text.bounds(area)) {
                            draw_task_line(frame, row, &task);
                            if let (true, Some(picker)) = (task.open, picker.as_mut()) {
                                picker.draw(frame.buffer_mut(), row, &task.text);
                            }
                        }
                    }
                    if let Some(notice) = &notice {
                        let row = Rect::new(area.x, area.y, area.width, 1);
                        draw_caption(frame.buffer_mut(), row, &notice.text);
                    }
                })?;
                redraw = false;
            }

            // Wake for the next gradient frame or the next change of the clock,
            // whichever comes first.
            let timeout = [
                background.wait_duration(Instant::now()),
                match view {
                    View::Home => Some(until_minute_changes(Local::now().time())),
                    View::Task => clock.until_label_changes(Instant::now()),
                },
                // A pending request has to be polled even while paused.
                (generator.is_pending()
                    || ambient.is_pending()
                    || loader.as_ref().is_some_and(Loader::is_pending))
                .then_some(Duration::from_millis(25)),
                // Opening a device can take a moment, and silence is judged by the clock.
                listener.is_some().then_some(Duration::from_millis(50)),
                notice
                    .as_ref()
                    .map(|notice| notice.until.saturating_duration_since(Instant::now())),
                #[cfg(feature = "focus-check")]
                focus_checker
                    .as_ref()
                    .and_then(|checker| checker.time_until_next(Instant::now())),
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
                        info!("quit requested with Ctrl+C");
                        return Ok(());
                    }
                    if task.open {
                        match key.code {
                            KeyCode::Esc => task.cancel(),
                            KeyCode::Enter => {
                                let chosen = match &picker {
                                    Some(picker) => picker.choose(&task.text),
                                    None => Some(task.text.trim().to_owned()),
                                };
                                if view == View::Task && chosen.as_deref().is_none_or(str::is_empty) {
                                    view = View::Home;
                                    task.cancel();
                                    clock = Clock::new(Instant::now());
                                    let mut playback = background.playback();
                                    playback.playing = true;
                                    background.set_playback(playback, Instant::now());
                                    #[cfg(feature = "focus-check")]
                                    if let Some(checker) = focus_checker.as_mut() {
                                        checker.set_task(None, Instant::now());
                                    }
                                    match moment.as_ref() {
                                        Some(moment) if !generator.is_pending() => {
                                            task.started_automatically(moment.full.clone());
                                            generator.submit_moment(&moment.full);
                                            if !generator.is_pending() {
                                                task.failed();
                                            }
                                        }
                                        _ => warn!(
                                            moment_known = moment.is_some(),
                                            generating = generator.is_pending(),
                                            "no new lyric could be requested at home"
                                        ),
                                    }
                                    info!(?view, "empty prompt; pomodoro reset, back to home");
                                } else if let Some(chosen) =
                                    chosen.filter(|chosen| !chosen.is_empty())
                                {
                                    let chosen: String =
                                        chosen.chars().take(MAX_TASK_CHARS).collect();
                                    #[cfg(feature = "focus-check")]
                                    if let Some(checker) = focus_checker.as_mut() {
                                        checker.set_task(Some(chosen.clone()), Instant::now());
                                    }
                                    let description =
                                        gradient_description(&chosen, moment.as_ref());
                                    let was_pending = generator.is_pending();
                                    let cached = generator.submit(&description);
                                    let started = cached.is_some()
                                        || (!was_pending && generator.is_pending());
                                    if let Some(cached) = cached {
                                        apply(&mut background, &mut speed, &cached)?;
                                        task.started_for(chosen);
                                        task.succeeded();
                                    } else if started {
                                        task.started_for(chosen);
                                    }
                                    if started && view == View::Home {
                                        view = View::Task;
                                        clock = Clock::new(Instant::now());
                                        info!(?view, "first task chosen; pomodoro reset and started");
                                    }
                                }
                            }
                            KeyCode::Up | KeyCode::Down => {
                                if let Some(picker) = picker.as_mut() {
                                    picker.step(key.code == KeyCode::Down, &task.text);
                                }
                            }
                            KeyCode::Backspace => {
                                task.text.pop();
                                if let Some(picker) = picker.as_mut() {
                                    picker.reset();
                                }
                            }
                            KeyCode::Char(c)
                                if !key
                                    .modifiers
                                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                            {
                                task.push(c);
                                if let Some(picker) = picker.as_mut() {
                                    picker.reset();
                                }
                            }
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('t' | 'T') => {
                                picker = loader.as_mut().map(|loader| {
                                    loader.refresh();
                                    let mut picker = picker_for(loader);
                                    picker.offer_home(view == View::Task);
                                    picker
                                });
                                task.begin();
                            }
                            KeyCode::Char('a' | 'A') => {
                                let now = Instant::now();
                                let (text, started) = if listener.is_some() {
                                    ("Audio off".to_string(), None)
                                } else {
                                    match Listener::start(
                                        options.source.clone(),
                                        options.sensitivity,
                                    ) {
                                        Ok(started) => {
                                            ("Opening audio...".to_string(), Some(started))
                                        }
                                        Err(message) => (format!("Audio: {message}"), None),
                                    }
                                };
                                info!(
                                    listening = started.is_some(),
                                    notice = text,
                                    "audio toggled"
                                );
                                listener = started;
                                notice = Some(Notice {
                                    text,
                                    until: now + NOTICE,
                                });
                            }
                            KeyCode::Up | KeyCode::Down => {
                                let step = if key.code == KeyCode::Up { 1 } else { -1 };
                                speed.change(step, &mut background)?;
                                debug!(step, "gradient speed changed");
                            }
                            KeyCode::Char(' ') if view == View::Task => {
                                let now = Instant::now();
                                clock.toggle_pause(now);
                                info!(
                                    paused = clock.is_paused(),
                                    phase = ?clock.phase(),
                                    "pause toggled"
                                );
                                let mut playback = background.playback();
                                playback.playing = !clock.is_paused();
                                background.set_playback(playback, now);
                            }
                            KeyCode::Char('r' | 'R') if view == View::Task => {
                                clock.reset(Instant::now());
                                info!("cycle reset");
                            }
                            _ => {}
                        }
                    }
                    redraw = true;
                }
                Event::Paste(text) if task.open => {
                    task.paste(&text);
                    if let Some(picker) = picker.as_mut() {
                        picker.reset();
                    }
                    redraw = true;
                }
                Event::Resize(_, _) => redraw = true,
                _ => {}
            }
        }
    });
    info!(
        elapsed_secs = start.elapsed().as_secs(),
        phases_completed,
        ok = result.is_ok(),
        "amber stopped"
    );
    println!("Logs: {}", log_directory.display());
    result
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
    fn the_time_of_day_is_hours_and_minutes_on_a_24_hour_clock() {
        let time = |h, m, s, n| NaiveTime::from_hms_nano_opt(h, m, s, n).unwrap();
        assert_eq!(clock_label(time(0, 5, 59, 0)), "00:05");
        assert_eq!(clock_label(time(17, 30, 0, 0)), "17:30");
        assert_eq!(clock_label(time(23, 59, 59, 0)), "23:59");
    }

    #[test]
    fn the_time_of_day_wakes_exactly_when_the_minute_changes() {
        let time = |s, n| NaiveTime::from_hms_nano_opt(10, 0, s, n).unwrap();
        assert_eq!(until_minute_changes(time(0, 0)), Duration::from_secs(60));
        assert_eq!(
            until_minute_changes(time(59, 250_000_000)),
            Duration::from_millis(750)
        );
    }

    #[test]
    fn the_home_view_is_always_white_and_the_task_view_follows_the_phase() {
        assert_eq!(timer_color(View::Home, Phase::Break(1), true), WHITE);
        assert_eq!(timer_color(View::Task, Phase::Break(1), false), BREAK_MINT);
        assert_eq!(timer_color(View::Task, Phase::Work(1), true), PAUSED_ROSE);
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
    fn the_halo_keeps_the_digits_and_bars_apart_from_a_white_gradient() {
        let (width, rows) = (100, 30);
        let area = Rect::new(0, 0, width, rows);
        let white = Color::Rgb(255, 255, 255);
        let mut buffer = Buffer::empty(area);
        for cell in &mut buffer.content {
            cell.set_char('▀').set_fg(white).set_bg(white);
        }
        let mut text = digits("25:00").unwrap();
        assert!(draw_halo(&mut buffer, area, &mut text, false, true) > 0);
        let bars = Bars::layout(width, rows).unwrap();
        let (left, _) = bars.segments[0];
        let below_bar = &buffer[(left as u16, (bars.top / 2) as u16)];
        assert_ne!(below_bar.fg, white);
        let digits = text.bounds(area).unwrap();
        let in_digits = &buffer[(digits.x + digits.width / 2, digits.y + digits.height / 2)];
        assert_ne!(in_digits.fg, white);
        // Far from both, the gradient is as it was.
        assert_eq!(buffer[(0, 0)].fg, white);
    }

    #[test]
    fn drawing_the_bars_on_a_tiny_screen_does_nothing() {
        let (mut buffer, area) = grey_screen(30, 6);
        draw_cycle(&mut buffer, area, Phase::Work(1), 0.5);
        assert!(buffer.content.iter().all(|c| c.fg == GREY && c.bg == GREY));
    }

    // ---- the task line and gradient speed ----

    #[test]
    fn the_task_edits_text_within_limits() {
        let mut task = TaskLine::default();
        for character in "a sunset\n over\tthe sea".chars() {
            task.push(character);
        }
        // Control characters are dropped.
        assert_eq!(task.text, "a sunset overthe sea");
        task.paste("\u{7}!\n?");
        assert_eq!(task.text, "a sunset overthe sea!?");

        // At most 1000 characters, counting characters and not bytes.
        let mut long = TaskLine::default();
        long.paste(&"é".repeat(1500));
        assert_eq!(long.text.chars().count(), MAX_TASK_CHARS);
        long.push('x');
        assert_eq!(long.text.chars().count(), MAX_TASK_CHARS);
    }

    #[test]
    fn the_task_shows_the_end_of_the_text_that_fits() {
        let mut task = TaskLine::default();
        task.paste("glacier ice");
        assert_eq!(task.tail(40), "glacier ice");
        assert_eq!(task.tail(5), "r ice");
        assert_eq!(task.tail(0), "");
        // Wide characters count as two columns.
        let mut wide = TaskLine::default();
        wide.paste("日本語");
        assert_eq!(wide.tail(4), "本語");
        assert_eq!(wide.tail(5), "本語");
    }

    /// The task drawn over grey half blocks, with the clock's digits laid out
    /// in the same area, and the row it was given.
    fn draw(task: &TaskLine, width: u16, rows: u16) -> (Buffer, Option<Rect>, Rect) {
        let area = Rect::new(0, 0, width, rows);
        let mut clock = digits("25:49").unwrap();
        let bounds = clock.bounds(area);
        let mut terminal = Terminal::new(TestBackend::new(width, rows)).unwrap();
        terminal
            .draw(|frame| {
                for cell in &mut frame.buffer_mut().content {
                    cell.set_char('▀').set_fg(GREY).set_bg(GREY);
                }
                if let Some(row) = task_row(area, bounds) {
                    draw_task_line(frame, row, task);
                }
            })
            .unwrap();
        let row = task_row(area, bounds).unwrap();
        (terminal.backend().buffer().clone(), bounds, row)
    }

    fn row_text(buffer: &Buffer, row: Rect) -> String {
        (row.left()..row.right())
            .map(|column| buffer[(column, row.y)].symbol())
            .collect::<String>()
    }

    #[test]
    fn the_task_line_sits_above_the_digits_lined_up_with_them() {
        let area = Rect::new(0, 0, 100, 30);
        let digits = Rect::new(20, 10, 60, 9);
        // A blank row between them when there is room...
        assert_eq!(task_row(area, Some(digits)), Some(Rect::new(20, 8, 60, 1)));
        // ...directly above when there is only one row...
        let tight = Rect::new(20, 1, 60, 9);
        assert_eq!(task_row(area, Some(tight)), Some(Rect::new(20, 0, 60, 1)));
        // ...and along the top edge when there is no room above, or no digits.
        let flush = Rect::new(20, 0, 60, 9);
        assert_eq!(task_row(area, Some(flush)), Some(Rect::new(0, 0, 100, 1)));
        assert_eq!(task_row(area, None), Some(Rect::new(0, 0, 100, 1)));
        assert_eq!(task_row(Rect::new(0, 0, 0, 0), None), None);
        // An offset screen keeps its offset.
        let offset = Rect::new(5, 3, 100, 30);
        assert_eq!(task_row(offset, None), Some(Rect::new(5, 3, 100, 1)));
        assert_eq!(
            task_row(offset, Some(Rect::new(25, 13, 60, 9))),
            Some(Rect::new(25, 11, 60, 1))
        );
    }

    #[test]
    fn the_task_is_only_a_chevron_and_the_text_just_above_the_clock() {
        let mut task = TaskLine {
            open: true,
            ..TaskLine::default()
        };
        task.paste("misty forest");
        let (buffer, digits, row) = draw(&task, 100, 30);
        let digits = digits.unwrap();
        assert_eq!(row.x, digits.x);
        assert_eq!(row.width, digits.width);
        assert!(row.bottom() < digits.y);

        // A chevron, the text and a cursor, and nothing else.
        assert!(row_text(&buffer, row).starts_with("› misty forest "));
        assert_eq!(row_text(&buffer, row).trim_end(), "› misty forest");
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
    fn a_long_task_scrolls_to_show_its_end() {
        let mut task = TaskLine {
            open: true,
            ..TaskLine::default()
        };
        task.paste(
            "a misty forest at dawn with a long thin river running quietly through it \
             and a small wooden bridge over the water near the very end",
        );
        let (buffer, _, row) = draw(&task, 100, 30);
        let text = row_text(&buffer, row);
        assert!(text.starts_with("› ") && text.contains("near the very end"));
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
            phrase: None,
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
        let mut task = TaskLine::default();
        assert!(!task.has_line());
        task.begin();
        assert!(task.has_line());

        // Sending a description: it shows once the gradient has arrived.
        task.paste("  a misty forest  ");
        task.started_for(task.text.trim().to_owned());
        assert_eq!(task.shown, None);
        task.paste("x"); // typed while waiting
        task.succeeded();
        assert_eq!(task.shown.as_deref(), Some("a misty forest"));
        assert!(!task.open && task.has_line());
        assert_eq!(task.text, "");

        // Opening the box again starts empty, and cancelling leaves the
        // description on screen as it was.
        task.begin();
        assert!(task.open);
        assert_eq!(task.text, "");
        assert_eq!(task.shown.as_deref(), Some("a misty forest"));
        task.paste("with fog");
        task.cancel();
        assert_eq!(task.text, "");
        assert_eq!(task.shown.as_deref(), Some("a misty forest"));
        assert!(!task.open && task.has_line());

        // A failed request leaves the old gradient and description alone, and
        // the box open with what was typed so it can be sent again.
        task.begin();
        task.paste("something else");
        task.started_for(task.text.trim().to_owned());
        task.failed();
        assert!(task.open);
        assert_eq!(task.text, "something else");
        assert_eq!(task.shown.as_deref(), Some("a misty forest"));
        assert_eq!(task.in_flight, None);

        // A new description replaces it, and only then.
        task.started_for(task.text.trim().to_owned());
        task.succeeded();
        assert_eq!(task.shown.as_deref(), Some("something else"));
        assert_eq!(task.text, "");
    }

    #[test]
    fn the_lyric_chosen_for_the_moment_replaces_its_description() {
        let mut task = TaskLine::default();
        task.started_automatically("Friday morning in May".into());
        task.describe_as("morning has broken".into());
        task.succeeded();
        assert_eq!(task.shown.as_deref(), Some("morning has broken"));

        // With no request in flight there is nothing to rename.
        let mut task = TaskLine::default();
        task.describe_as("morning has broken".into());
        task.succeeded();
        assert_eq!(task.shown, None);
    }

    #[test]
    fn a_startup_description_does_not_disturb_a_box_being_typed_in() {
        let mut task = TaskLine::default();
        task.started_automatically("Friday morning in May".into());
        task.begin();
        task.paste("half a thought");
        task.succeeded();
        assert_eq!(task.shown.as_deref(), Some("Friday morning in May"));
        assert!(task.open);
        assert_eq!(task.text, "half a thought");

        // A failed one shows nothing and leaves the box alone too; the next
        // typed request behaves as usual.
        let mut task = TaskLine::default();
        task.started_automatically("Friday morning in May".into());
        task.failed();
        assert_eq!(task.shown, None);
        assert!(!task.has_line());
        task.begin();
        task.paste("a misty forest");
        task.started_for(task.text.trim().to_owned());
        task.succeeded();
        assert!(!task.open);
        assert_eq!(task.text, "");
        assert_eq!(task.shown.as_deref(), Some("a misty forest"));
    }

    #[test]
    fn the_command_line_picks_the_place_to_describe() {
        let parse = |args: &[&str]| parse_args(args.iter().map(|a| a.to_string()));
        let at = |place| {
            Ok(Command::Clock(Options {
                place,
                ..Options::default()
            }))
        };
        assert_eq!(parse(&[]), at(Some(Place::Here)));
        assert_eq!(parse(&["--no-weather"]), at(None));
        assert_eq!(parse(&["--alarm-test"]), Ok(Command::AlarmTest));
        let lisbon = at(Some(Place::Named("Lisbon".into())));
        assert_eq!(parse(&["--location", "Lisbon"]), lisbon);
        assert_eq!(parse(&["--location=Lisbon"]), lisbon);
        // The last choice wins.
        assert_eq!(parse(&["--location", "Lisbon", "--no-weather"]), at(None));
        assert!(parse(&["--location"]).is_err());
        assert!(parse(&["--location", " "]).is_err());
        assert!(parse(&["--location="]).is_err());
        assert!(parse(&["--bogus"]).unwrap_err().contains("--bogus"));
        assert_eq!(
            parse(&["--pure-white"]),
            Ok(Command::Clock(Options {
                pure_white: true,
                ..Options::default()
            }))
        );
        assert!(parse(&["--pure-white=1"]).is_err());
    }

    #[test]
    fn energy_sets_how_hard_sound_warps_and_steers_but_not_how_fast_it_speeds_up() {
        let normal = Reactivity::default();
        let calm = reactivity_for(0.0);
        let wild = reactivity_for(4.0);
        assert!((calm.warp - 0.6 * normal.warp).abs() < 1e-6);
        assert!((calm.steer - 0.6 * normal.steer).abs() < 1e-6);
        assert!((wild.warp - 1.2 * normal.warp).abs() < 1e-6);
        assert!((wild.steer - 1.2 * normal.steer).abs() < 1e-6);
        for scaled in [calm, wild] {
            assert_eq!(
                (scaled.speed, scaled.kick, scaled.busy),
                (normal.speed, normal.kick, normal.busy)
            );
        }
        // Out-of-range scores are held to the ends.
        assert_eq!(reactivity_for(9.0), wild);
        assert_eq!(reactivity_for(-3.0), calm);
    }

    #[test]
    fn the_command_line_picks_what_to_listen_to() {
        let parse = |args: &[&str]| parse_args(args.iter().map(|a| a.to_string()));
        let listening = |source, sensitivity| {
            Ok(Command::Clock(Options {
                listen: true,
                source,
                sensitivity,
                ..Options::default()
            }))
        };
        // Sound is on unless turned off.
        assert!(Options::default().listen);
        assert_eq!(
            parse(&["--sensitivity", "2"]),
            listening(Source::System, 2.0)
        );
        assert_eq!(
            parse(&["--no-audio"]),
            Ok(Command::Clock(Options {
                listen: false,
                ..Options::default()
            }))
        );
        assert_eq!(
            parse(&["--audio-from", "mic"]),
            listening(Source::Microphone, 1.0)
        );
        assert_eq!(
            parse(&["--audio-from=system"]),
            listening(Source::System, 1.0)
        );
        assert_eq!(
            parse(&["--audio-from", "BlackHole", "--sensitivity=0.5"]),
            listening(Source::Named("BlackHole".into()), 0.5)
        );
        assert_eq!(parse(&["--list-audio"]), Ok(Command::ListAudio));
        for bad in [
            &["--audio-from"][..],
            &["--sensitivity", "loud"],
            &["--sensitivity", "0"],
            &["--sensitivity=-1"],
            &["--sensitivity", "NaN"],
            &["--no-audio=on"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_command_line_names_a_file_of_tasks() {
        let parse = |args: &[&str]| parse_args(args.iter().map(|a| a.to_string()));
        let with = |path: &str| {
            Ok(Command::Clock(Options {
                tasks: Some(PathBuf::from(path)),
                todoist: false,
                ..Options::default()
            }))
        };
        assert_eq!(parse(&[]), Ok(Command::Clock(Options::default())));
        assert_eq!(parse(&["--tasks", "list.md"]), with("list.md"));
        assert_eq!(parse(&["--tasks=my list.md"]), with("my list.md"));
        assert!(parse(&["--tasks"]).is_err());
        assert!(parse(&["--tasks="]).is_err());
    }

    #[test]
    fn jev_is_told_the_task_with_a_hint_of_the_moment_when_it_knows_it() {
        let moment = Moment {
            full: "Friday golden hour in October (autumn), light rain, cool".into(),
            brief: "golden hour, light rain".into(),
        };
        let description = gradient_description("write the report", Some(&moment));
        assert_eq!(
            description,
            "write the report, with a hint of golden hour, light rain"
        );
        // Only the brief form goes with a task, and the task comes first.
        assert!(!description.contains("Friday") && !description.contains("cool"));
        assert_eq!(
            gradient_description("write the report", None),
            "write the report"
        );
        // A task and the moment fit in what Jev accepts.
        let longest = Moment {
            full: String::new(),
            brief: "y".repeat(200),
        };
        let longest = gradient_description(&"x".repeat(MAX_TASK_CHARS), Some(&longest));
        assert!(longest.chars().count() < 1000);
    }

    #[test]
    fn the_command_line_can_name_a_todoist_filter() {
        let parse = |args: &[&str]| parse_args(args.iter().map(|a| a.to_string()));
        let todoist = |filter: Option<&str>| {
            Ok(Command::Clock(Options {
                todoist: true,
                todoist_filter: filter.map(str::to_owned),
                ..Options::default()
            }))
        };
        // Todoist is where tasks come from unless a file or --no-todoist says otherwise.
        assert!(Options::default().todoist);
        assert_eq!(parse(&["--todoist"]), todoist(None));
        assert_eq!(
            parse(&["--no-todoist"]),
            Ok(Command::Clock(Options {
                todoist: false,
                ..Options::default()
            }))
        );
        assert_eq!(
            parse(&["--todoist-filter", "#Work & p1"]),
            todoist(Some("#Work & p1"))
        );
        assert_eq!(
            parse(&["--todoist", "--todoist-filter=today"]),
            todoist(Some("today"))
        );
        assert!(parse(&["--todoist-filter"]).is_err());
        assert!(parse(&["--todoist=yes"]).is_err());
        // One place for tasks to come from.
        assert!(parse(&["--todoist", "--tasks", "list.md"]).is_err());
        assert!(parse(&["--tasks", "list.md", "--todoist-filter", "today"]).is_err());
    }

    #[test]
    fn the_picker_shows_what_the_loader_has_or_why_it_has_nothing() {
        let path = std::env::temp_dir().join(format!("amber-timer-{}.md", std::process::id()));
        std::fs::write(&path, "# Done\n- [x] all of it\n").unwrap();
        let mut loader = Loader::new(tasks::Source::Markdown(path.clone()));
        let row = |picker: &mut Picker| {
            let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 10));
            picker.draw(&mut buffer, Rect::new(5, 8, 70, 1), "");
            (0..80).map(|x| buffer[(x, 9)].symbol()).collect::<String>()
        };
        assert!(row(&mut picker_for(&loader)).contains("Loading tasks"));

        loader.refresh();
        while loader.poll().is_none() {
            std::thread::sleep(Duration::from_millis(2));
        }
        std::fs::remove_file(&path).unwrap();
        assert!(row(&mut picker_for(&loader)).contains("No open tasks in"));

        // The file is gone now. Failing to read it again leaves the list that
        // was already fetched, which is empty.
        loader.refresh();
        while loader.poll().is_none() {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(row(&mut picker_for(&loader)).contains("No open tasks in"));

        // With nothing fetched ever, the reason takes the list's place, and what
        // is typed still works as a task.
        let mut never = Loader::new(tasks::Source::Markdown(path));
        never.refresh();
        while never.poll().is_none() {
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut picker = picker_for(&never);
        assert!(row(&mut picker).contains("Can't read"));
        assert_eq!(picker.choose("by hand").as_deref(), Some("by hand"));
    }

    #[test]
    fn opening_the_box_always_starts_empty() {
        let mut task = TaskLine {
            text: "left over".into(),
            shown: Some("a quiet harbor".into()),
            ..TaskLine::default()
        };
        task.begin();
        assert_eq!(task.text, "");
        assert_eq!(task.shown.as_deref(), Some("a quiet harbor"));
        // While the box is open and empty, the line is the empty edit box, not
        // the description.
        assert!(task.open && task.has_line());
    }

    #[test]
    fn cancelling_with_nothing_shown_empties_the_box_and_the_line() {
        let mut task = TaskLine::default();
        task.open = true;
        task.paste("never sent");
        task.cancel();
        assert_eq!(task.text, "");
        assert!(!task.has_line());
        // A late arrival with nothing in flight changes nothing.
        task.succeeded();
        assert_eq!(task.shown, None);
    }

    #[test]
    fn the_caption_shows_the_start_and_marks_where_it_is_cut() {
        let with = |text: &str| TaskLine {
            shown: Some(text.into()),
            ..TaskLine::default()
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
        assert_eq!(TaskLine::default().caption(10), "");
    }

    #[test]
    fn once_set_the_description_is_bare_centered_text_above_the_clock() {
        let task = TaskLine {
            text: "misty forest".into(),
            shown: Some("misty forest".into()),
            ..TaskLine::default()
        };
        assert!(!task.open && task.has_line());
        let (buffer, digits, row) = draw(&task, 100, 30);
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
        let long = TaskLine {
            shown: Some("a misty forest at dawn ".repeat(8)),
            ..TaskLine::default()
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
