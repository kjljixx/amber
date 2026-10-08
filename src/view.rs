use std::time::{Duration, Instant};

use ratatui::{Frame, buffer::Buffer, layout::Rect, widgets::Widget};

use crate::{CompiledRecipe, HalfBlocks, Preview, Recipe, animation::Animation};

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

/// One independently controlled gradient panel. Owns no terminal or event loop.
pub struct GradientView {
    recipe: CompiledRecipe,
    preview: Preview,
    animation: Animation,
    playback: Playback,
    speed: f64,
    dirty: bool,
    stats: RenderStats,
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
            dirty: true,
            stats: RenderStats::default(),
        }
    }

    /// Failed validation leaves the current recipe and preview intact.
    pub fn set_recipe(&mut self, recipe: &Recipe) -> Result<(), &'static str> {
        self.set_compiled(recipe.compile()?);
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
            let field = self.recipe.prepare(
                aspect,
                self.animation.phase(),
                self.playback.warped,
                self.playback.drifting,
            );
            self.preview
                .rebuild(area.width, area.height, |x, y| field.sample(x, y));
        } else {
            self.preview
                .rebuild(area.width, area.height, |_, _| [0, 0, 0]);
        }
        self.stats.sampling = start.elapsed();
        self.stats.rebuilds += 1;
        self.dirty = false;
    }
}

impl Widget for &mut GradientView {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        self.rebuild(area);
        HalfBlocks(&self.preview).render(area, buffer);
    }
}
