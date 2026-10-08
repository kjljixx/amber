use std::{
    io,
    time::{Duration, Instant},
};

use amber::GradientView;

/// Host UI setting, independent of the recipe's suggested animation rate.
pub struct SpeedControl {
    base: f64,
    step: i32,
}

impl SpeedControl {
    pub fn new(base: f64) -> Self {
        Self { base, step: 0 }
    }

    pub fn multiplier(&self) -> f64 {
        2.0_f64.powf(f64::from(self.step) / 2.0)
    }

    pub fn apply(&self, view: &mut GradientView) -> io::Result<()> {
        view.set_timing(
            Duration::from_millis(50),
            self.base * self.multiplier(),
            Instant::now(),
        )
        .map_err(io::Error::other)
    }

    pub fn change(&mut self, delta: i32, view: &mut GradientView) -> io::Result<()> {
        self.step = (self.step + delta).clamp(-6, 8);
        self.apply(view)
    }

    pub fn set_base(&mut self, base: f64, view: &mut GradientView) -> io::Result<()> {
        self.base = base;
        self.apply(view)
    }
}
