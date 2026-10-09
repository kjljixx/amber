//! A pomodoro clock: big translucent digits over an animated gradient, with
//! four bars along the bottom showing which work period of the cycle this is.
//!
//! The cycle is work, 5 minute break, three times over, then a fourth work
//! period and a 20 minute break, and then it starts again.
//!
//! `cargo run --offline --bin timer`
//! Space: pause or resume the timer and the gradient | r: back to the start of
//! the cycle, 25:00 | q, Esc or Ctrl+C: quit
use std::{
    io,
    time::{Duration, Instant},
};

use amber::{Digits, GradientView, blend_samples, presets};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{buffer::Buffer, layout::Rect};

const WORK: Duration = Duration::from_secs(25 * 60);
const SHORT_BREAK: Duration = Duration::from_secs(5 * 60);
const LONG_BREAK: Duration = Duration::from_secs(20 * 60);
const WORK_PERIODS: u8 = 4;

/// Opacity of the digits and of the lit part of the bars.
const ALPHA: f32 = 0.6;
/// Opacity of the unlit part of the bars.
const DIM_ALPHA: f32 = 0.25;
/// Gradient animation: radians per second, at 20 frames per second.
const SPEED: f64 = 1.2;
const FRAME_INTERVAL: Duration = Duration::from_millis(50);
const WHITE: [u8; 3] = [255, 255, 255];

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

fn digits(text: &str) -> io::Result<Digits> {
    Digits::new(text, WHITE, ALPHA).map_err(io::Error::other)
}

fn main() -> io::Result<()> {
    let start = Instant::now();
    let mut background =
        GradientView::new(&presets::presets()[1].recipe, start).map_err(io::Error::other)?;
    background
        .set_timing(FRAME_INTERVAL, SPEED, start)
        .map_err(io::Error::other)?;
    let mut clock = Clock::new(start);
    let mut shown = label(clock.phase().length());
    let mut text = digits(&shown)?;
    let mut redraw = true;
    ratatui::run(|terminal| {
        loop {
            let now = Instant::now();
            clock.advance(now);
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
                })?;
                redraw = false;
            }

            // Wake for the next gradient frame or the next change of the clock,
            // whichever comes first.
            let timeout = [
                background.wait_duration(Instant::now()),
                clock.until_label_changes(Instant::now()),
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
                Event::Key(key) if key.is_press() => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    KeyCode::Char(' ') => {
                        let now = Instant::now();
                        clock.toggle_pause(now);
                        let mut playback = background.playback();
                        playback.playing = !clock.is_paused();
                        background.set_playback(playback, now);
                        redraw = true;
                    }
                    KeyCode::Char('r' | 'R') => {
                        clock.reset(Instant::now());
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

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

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
}
