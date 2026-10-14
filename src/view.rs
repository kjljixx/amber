use std::time::{Duration, Instant};

use ratatui::{Frame, buffer::Buffer, layout::Rect, widgets::Widget};
use tracing::{info, warn};

use crate::{
    CompiledRecipe, HalfBlocks, Preview, Recipe, Rgb, animation::Animation,
    color::with_min_lightness,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Playback {
    pub playing: bool,
    pub warped: bool,
    pub drifting: bool,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            playing: true,
            warped: true,
            drifting: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    /// Time spent preparing and sampling the latest preview, excluding terminal output.
    pub sampling: Duration,
    pub rebuilds: u64,
}

/// How often the time spent drawing is summed up in the log.
const SUMMARY_EVERY: Duration = Duration::from_secs(10);
/// Keeps the accent readable on the overlay's near-black panel.
const ACCENT_MIN_LIGHTNESS: f32 = 0.75;

/// The rebuilds since the last log line.
struct RenderWindow {
    since: Instant,
    rebuilds: u32,
    sampling_total: Duration,
    sampling_max: Duration,
}

impl RenderWindow {
    fn new() -> Self {
        Self {
            since: Instant::now(),
            rebuilds: 0,
            sampling_total: Duration::ZERO,
            sampling_max: Duration::ZERO,
        }
    }

    fn add(&mut self, sampling: Duration, area: Rect) {
        self.rebuilds += 1;
        self.sampling_total += sampling;
        self.sampling_max = self.sampling_max.max(sampling);
        if self.since.elapsed() >= SUMMARY_EVERY {
            info!(
                seconds = self.since.elapsed().as_secs(),
                rebuilds = self.rebuilds,
                sampling_mean_ms =
                    self.sampling_total.as_secs_f64() * 1000.0 / f64::from(self.rebuilds),
                sampling_max_ms = self.sampling_max.as_secs_f64() * 1000.0,
                width = area.width,
                height = area.height,
                "gradient rendered"
            );
            *self = Self::new();
        }
    }
}

/// How strongly something outside the view (audio, say) is stirring it up.
/// Every field is clamped to 0..=1; the default is calm.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Excitement {
    /// Sustained loudness: how lively the whole picture is.
    pub level: f32,
    /// A hit that decays on its own, such as a beat: 1 at the strike.
    pub pulse: f32,
    /// How much is changing right now, such as a run of notes: continuous where
    /// `pulse` only marks hits.
    pub activity: f32,
    /// Where the sound sits between dark and bright. Its changes steer the
    /// picture, forward as it rises and back as it falls, so a melody pushes it
    /// around while a held note leaves it be. `None` for no steering.
    pub tone: Option<f32>,
}

impl Excitement {
    fn clamped(self) -> Self {
        let unit = |value: f32| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        Self {
            level: unit(self.level),
            pulse: unit(self.pulse),
            activity: unit(self.activity),
            tone: self
                .tone
                .filter(|tone| tone.is_finite())
                .map(|tone| tone.clamp(0.0, 1.0)),
        }
    }
}

/// What full excitement does to the view. The speeds are radians per second
/// added to whatever speed the view already has, so the music moves every
/// gradient by the same amount however fast it runs on its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reactivity {
    /// Extra speed at `level` 1.
    pub speed: f32,
    /// Extra speed at `pulse` 1: the surge forward on a beat.
    pub kick: f32,
    /// Extra speed at `activity` 1.
    pub busy: f32,
    /// Strength of the localized pulse ripple at `pulse` 1. Sustained `activity`
    /// and `level` still increase the whole gradient's warp.
    pub warp: f32,
    /// Radians the picture moves when `tone` crosses its whole range.
    pub steer: f32,
}

impl Default for Reactivity {
    fn default() -> Self {
        Self {
            speed: 1.5,
            kick: 4.5,
            busy: 2.5,
            warp: 0.75,
            steer: 5.0,
        }
    }
}

/// One independently controlled gradient panel. Owns no terminal or event loop.
pub struct GradientView {
    recipe: CompiledRecipe,
    preview: Preview,
    animation: Animation,
    playback: Playback,
    speed: f64,
    excitement: Excitement,
    reactivity: Reactivity,
    dirty: bool,
    stats: RenderStats,
    window: RenderWindow,
}

impl Drop for GradientView {
    fn drop(&mut self) {
        info!(rebuilds = self.stats.rebuilds, "gradient view closed");
    }
}

impl GradientView {
    pub fn new(recipe: &Recipe, now: Instant) -> Result<Self, &'static str> {
        Ok(Self::from_compiled(recipe.compile()?, now))
    }

    pub fn from_compiled(recipe: CompiledRecipe, now: Instant) -> Self {
        Self {
            recipe,
            preview: Preview::default(),
            animation: Animation::new(now),
            playback: Playback::default(),
            speed: 3.0,
            excitement: Excitement::default(),
            reactivity: Reactivity::default(),
            dirty: true,
            stats: RenderStats::default(),
            window: RenderWindow::new(),
        }
    }

    /// Failed validation leaves the current recipe and preview intact.
    pub fn set_recipe(&mut self, recipe: &Recipe) -> Result<(), &'static str> {
        let compiled = recipe.compile().inspect_err(|message| {
            warn!(message, "recipe rejected; the current one stays");
        })?;
        info!(
            points = recipe.points.len(),
            softness = recipe.softness,
            distortion = recipe.distortion,
            swirl = recipe.swirl,
            motion = recipe.motion,
            "recipe applied"
        );
        self.set_compiled(compiled);
        Ok(())
    }

    pub fn set_compiled(&mut self, recipe: CompiledRecipe) {
        self.recipe = recipe;
        self.dirty = true;
    }

    pub fn playback(&self) -> Playback {
        self.playback
    }

    pub fn set_playback(&mut self, playback: Playback, now: Instant) {
        if playback != self.playback {
            info!(?playback, "playback changed");
            self.playback = playback;
            self.dirty = true;
            self.sync_clock(now);
        }
    }

    /// Configure pacing without changing the displayed animation phase.
    /// Speed is radians per second; zero freezes motion without periodic wakeups.
    pub fn set_timing(
        &mut self,
        interval: Duration,
        speed: f64,
        now: Instant,
    ) -> Result<(), &'static str> {
        self.animation.set_timing(interval, speed, now)?;
        self.speed = speed;
        self.sync_clock(now);
        Ok(())
    }

    pub fn excitement(&self) -> Excitement {
        self.excitement
    }

    /// Stir the view up (or calm it again). Speeds the animation, strengthens
    /// the warp and steers it with the tone; all return to the recipe's own
    /// values at zero. A paused view
    /// stays still. Call it as often as the input changes: it takes effect on
    /// the next `update`, and the host should keep a short frame interval while
    /// the excitement is above zero.
    pub fn set_excitement(&mut self, excitement: Excitement) {
        let excitement = excitement.clamped();
        if excitement == self.excitement {
            return;
        }
        // A change of tone pushes the picture along; the first reading after
        // none has nothing to compare with and does not.
        if let (Some(before), Some(after)) = (self.excitement.tone, excitement.tone)
            && self.playback.playing
        {
            self.animation
                .nudge(f64::from((after - before) * self.reactivity.steer));
        }
        self.excitement = excitement;
        self.animation.set_boost(self.speed_boost());
        self.dirty |= self.playback.playing;
    }

    pub fn reactivity(&self) -> Reactivity {
        self.reactivity
    }

    pub fn set_reactivity(&mut self, reactivity: Reactivity) {
        self.reactivity = reactivity;
        self.animation.set_boost(self.speed_boost());
        self.dirty |= self.playback.playing;
    }

    /// Radians per second the excitement adds to the animation's own speed.
    fn speed_boost(&self) -> f64 {
        let Excitement {
            level,
            pulse,
            activity,
            ..
        } = self.excitement;
        let Reactivity {
            speed, kick, busy, ..
        } = self.reactivity;
        f64::from(speed * level + busy * activity + kick * pulse)
    }

    /// Warp multiplier, capped so a loud passage cannot tear the picture apart.
    fn warp_intensity(&self) -> f32 {
        let Excitement {
            level, activity, ..
        } = self.excitement;
        (1.0 + self.reactivity.warp * (0.125 * level + 0.25 * activity)).clamp(0.0, 4.0)
    }

    fn sync_clock(&mut self, now: Instant) {
        self.animation.set_running(
            self.playback.playing
                && self.speed > 0.0
                && (self.playback.warped || self.playback.drifting),
            now,
        );
    }

    /// Returns true when the host should redraw this view.
    pub fn update(&mut self, now: Instant) -> bool {
        if self.animation.tick(now) {
            self.dirty = true;
        }
        self.dirty
    }

    /// The host may use this deadline alongside other components' deadlines.
    /// None means this view requires no animation wakeup.
    pub fn wait_duration(&self, now: Instant) -> Option<Duration> {
        self.animation.wait_duration(now)
    }

    /// The most colorful color currently on screen, once the view has been drawn.
    pub fn accent(&self) -> Option<Rgb> {
        self.preview
            .most_vivid()
            .map(|rgb| with_min_lightness(rgb, ACCENT_MIN_LIGHTNESS))
    }

    pub fn stats(&self) -> RenderStats {
        self.stats
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(self, area);
    }

    fn rebuild(&mut self, area: Rect) {
        if !self.dirty && self.preview.size() == (area.width, area.height) {
            return;
        }
        let start = Instant::now();
        if area.width > 0 && area.height > 0 {
            let aspect = f32::from(area.width) / (2.0 * f32::from(area.height));
            let mut field = self.recipe.prepare_intensified(
                aspect,
                self.animation.phase(),
                self.animation.travel(),
                self.playback.warped,
                self.playback.drifting,
                self.warp_intensity(),
            );
            if self.playback.warped && self.excitement.pulse > 0.0 {
                field = field.with_local_pulse(
                    self.excitement.pulse * self.reactivity.warp,
                    self.animation.travel(),
                    self.recipe.seed(),
                    self.recipe.pulses_from_center(),
                );
            }
            self.preview
                .rebuild(area.width, area.height, |x, y| field.sample(x, y));
        } else {
            self.preview
                .rebuild(area.width, area.height, |_, _| [0, 0, 0]);
        }
        self.stats.sampling = start.elapsed();
        self.stats.rebuilds += 1;
        self.window.add(self.stats.sampling, area);
        self.dirty = false;
    }
}

impl Widget for &mut GradientView {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        self.rebuild(area);
        HalfBlocks(&self.preview).render(area, buffer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painted(view: &mut GradientView) -> Buffer {
        let area = Rect::new(0, 0, 24, 8);
        let mut buffer = Buffer::empty(area);
        Widget::render(view, area, &mut buffer);
        buffer
    }

    fn view(now: Instant) -> GradientView {
        GradientView::new(&crate::presets::presets()[1].recipe, now).unwrap()
    }

    #[test]
    fn calm_excitement_changes_nothing() {
        let now = Instant::now();
        let mut plain = view(now);
        let mut calm = view(now);
        calm.set_excitement(Excitement {
            level: 0.8,
            pulse: 1.0,
            ..Excitement::default()
        });
        calm.set_excitement(Excitement::default());
        let later = now + Duration::from_secs(1);
        plain.update(later);
        calm.update(later);
        assert_eq!(painted(&mut plain), painted(&mut calm));
    }

    #[test]
    fn excitement_strengthens_the_warp_and_hastens_the_animation() {
        let now = Instant::now();
        let mut plain = view(now);
        let mut excited = view(now);
        assert_eq!(painted(&mut plain), painted(&mut excited));

        excited.set_excitement(Excitement {
            level: 1.0,
            pulse: 1.0,
            ..Excitement::default()
        });
        assert!(
            excited.update(now),
            "a change of excitement asks for a redraw"
        );
        assert_ne!(painted(&mut plain), painted(&mut excited));

        let later = now + Duration::from_secs(1);
        plain.update(later);
        excited.update(later);
        // 3 rad/s of its own, plus 2.25 for the level and 4.5 for the pulse.
        assert!(excited.animation.travel() > 2.0 * plain.animation.travel());
    }

    #[test]
    fn excitement_is_clamped_and_ignores_garbage() {
        let mut view = view(Instant::now());
        view.set_excitement(Excitement {
            level: 7.0,
            pulse: f32::NAN,
            activity: -2.0,
            tone: Some(f32::INFINITY),
        });
        assert_eq!(
            view.excitement(),
            Excitement {
                level: 1.0,
                pulse: 0.0,
                activity: 0.0,
                tone: None,
            }
        );
        assert!(view.warp_intensity() <= 4.0);
    }

    #[test]
    fn a_paused_view_stays_still_when_excited() {
        let now = Instant::now();
        let mut view = view(now);
        view.set_playback(
            Playback {
                playing: false,
                ..Playback::default()
            },
            now,
        );
        painted(&mut view);
        assert!(!view.update(now));
        view.set_excitement(Excitement {
            level: 1.0,
            pulse: 1.0,
            ..Excitement::default()
        });
        assert!(!view.update(now + Duration::from_secs(1)));
    }

    #[test]
    fn a_changing_tone_steers_and_a_held_one_does_not() {
        let now = Instant::now();
        let mut view = view(now);
        let tone = |tone| Excitement {
            tone,
            ..Excitement::default()
        };
        let steer = f64::from(view.reactivity().steer);
        // The first reading has nothing to compare with.
        view.set_excitement(tone(Some(0.3)));
        assert_eq!(view.animation.travel(), 0.0);
        view.set_excitement(tone(Some(0.5)));
        assert!((view.animation.travel() - 0.2 * steer).abs() < 1e-5);
        // Falling goes back; holding stays; silence forgets the tone.
        view.set_excitement(tone(Some(0.4)));
        assert!((view.animation.travel() - 0.1 * steer).abs() < 1e-5);
        let held = view.animation.travel();
        view.set_excitement(Excitement {
            level: 0.5,
            tone: Some(0.4),
            ..Excitement::default()
        });
        assert_eq!(view.animation.travel(), held);
        view.set_excitement(tone(None));
        view.set_excitement(tone(Some(0.9)));
        assert_eq!(view.animation.travel(), held);
    }

    #[test]
    fn activity_quickens_and_warps_without_any_beat() {
        let now = Instant::now();
        let mut plain = view(now);
        let mut busy = view(now);
        busy.set_excitement(Excitement {
            activity: 1.0,
            ..Excitement::default()
        });
        assert!(busy.update(now));
        assert_ne!(painted(&mut plain), painted(&mut busy));
        let later = now + Duration::from_secs(1);
        plain.update(later);
        busy.update(later);
        // 3 rad/s of its own, plus 3.75 for the activity.
        assert!(busy.animation.travel() > 1.5 * plain.animation.travel());
    }

    #[test]
    fn a_paused_view_is_not_steered() {
        let now = Instant::now();
        let mut view = view(now);
        view.set_playback(
            Playback {
                playing: false,
                ..Playback::default()
            },
            now,
        );
        for tone in [0.2, 0.8] {
            view.set_excitement(Excitement {
                tone: Some(tone),
                ..Excitement::default()
            });
        }
        assert_eq!(view.animation.travel(), 0.0);
    }
}
