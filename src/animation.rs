use std::time::{Duration, Instant};

const FRAME_INTERVAL: Duration = Duration::from_millis(50);
// Fast motion makes uneven frame pacing easier to see while testing.
const PHASE_PER_SECOND: f64 = 3.0;

/// Advances by elapsed time and schedules one frame at a time, without a backlog.
pub struct Animation {
    phase: f64,
    travel: f64,
    running: bool,
    last_frame: Instant,
    next_frame: Instant,
    interval: Duration,
    speed: f64,
    /// Radians per second added to `speed` for as long as something (audio) is
    /// exciting the view.
    boost: f64,
}

impl Animation {
    pub fn new(now: Instant) -> Self {
        Self {
            phase: 0.0,
            travel: 0.0,
            running: true,
            last_frame: now,
            next_frame: now + FRAME_INTERVAL,
            interval: FRAME_INTERVAL,
            speed: PHASE_PER_SECOND,
            boost: 0.0,
        }
    }

    pub fn travel(&self) -> f64 {
        self.travel
    }

    pub fn phase(&self) -> f32 {
        self.phase as f32
    }

    pub fn set_timing(
        &mut self,
        interval: Duration,
        speed: f64,
        now: Instant,
    ) -> Result<(), &'static str> {
        if interval.is_zero() || now.checked_add(interval).is_none() {
            return Err("frame interval must be positive and fit the clock");
        }
        if !speed.is_finite() || speed < 0.0 {
            return Err("animation speed must be finite and nonnegative");
        }
        self.interval = interval;
        self.speed = speed;
        self.last_frame = now;
        self.next_frame = now + interval;
        Ok(())
    }

    /// Takes effect from the next tick; time already elapsed keeps the old boost.
    pub fn set_boost(&mut self, radians_per_second: f64) {
        self.boost = if radians_per_second.is_finite() {
            radians_per_second.max(0.0)
        } else {
            0.0
        };
    }

    /// Moves the picture by `radians` at once, backward if negative.
    pub fn nudge(&mut self, radians: f64) {
        if radians.is_finite() {
            self.travel += radians;
            self.phase = (self.phase + radians).rem_euclid(std::f64::consts::TAU);
        }
    }

    pub fn set_running(&mut self, running: bool, now: Instant) {
        if running != self.running {
            self.running = running;
            // Freeze the displayed phase; resuming excludes time spent paused.
            self.last_frame = now;
            self.next_frame = now + self.interval;
        }
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        if !self.running || now < self.next_frame {
            return false;
        }
        let elapsed = now.duration_since(self.last_frame).as_secs_f64();
        let speed = self.speed + self.boost;
        self.travel += elapsed * speed;
        // Reduce elapsed time first so large finite speeds cannot overflow.
        let advance = if speed > 0.0 {
            (elapsed % (std::f64::consts::TAU / speed)) * speed
        } else {
            0.0
        };
        self.phase = (self.phase + advance) % std::f64::consts::TAU;
        self.last_frame = now;
        self.next_frame = now + self.interval;
        true
    }

    /// None means the app can block on input with no periodic wakeups.
    pub fn wait_duration(&self, now: Instant) -> Option<Duration> {
        self.running
            .then(|| self.next_frame.saturating_duration_since(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_clock_survives_cycles_and_excludes_paused_time() {
        let start = Instant::now();
        let mut animation = Animation::new(start);
        animation.tick(start + Duration::from_secs(20));
        assert_eq!(animation.travel(), 60.0);
        assert!(animation.phase() < std::f32::consts::TAU);
        animation.set_running(false, start + Duration::from_secs(20));
        animation.tick(start + Duration::from_secs(100));
        assert_eq!(animation.travel(), 60.0);
        animation.set_running(true, start + Duration::from_secs(100));
        animation.tick(start + Duration::from_secs(101));
        assert_eq!(animation.travel(), 63.0);
    }

    #[test]
    fn a_boost_adds_to_the_speed_from_the_next_tick() {
        let start = Instant::now();
        let mut animation = Animation::new(start);
        animation.set_boost(6.0);
        animation.tick(start + Duration::from_secs(1));
        assert_eq!(animation.travel(), 9.0);
        animation.set_boost(0.0);
        animation.tick(start + Duration::from_secs(2));
        assert_eq!(animation.travel(), 12.0);
        animation.set_boost(f64::NAN);
        animation.tick(start + Duration::from_secs(3));
        assert_eq!(animation.travel(), 15.0);
    }

    #[test]
    fn a_nudge_moves_both_ways_without_leaving_the_cycle() {
        let start = Instant::now();
        let mut animation = Animation::new(start);
        animation.nudge(1.0);
        assert_eq!((animation.travel(), animation.phase()), (1.0, 1.0));
        animation.nudge(-2.0);
        assert_eq!(animation.travel(), -1.0);
        assert!((animation.phase() - (std::f32::consts::TAU - 1.0)).abs() < 1e-5);
        animation.nudge(f64::NAN);
        assert_eq!(animation.travel(), -1.0);
    }

    #[test]
    fn waits_until_the_frame_deadline() {
        let start = Instant::now();
        let mut animation = Animation::new(start);
        assert!(!animation.tick(start + Duration::from_millis(49)));
        assert_eq!(animation.phase(), 0.0);
        assert!(animation.tick(start + FRAME_INTERVAL));
        assert!((animation.phase() - 0.15).abs() < 0.000001);
    }

    #[test]
    fn late_frames_use_elapsed_time_without_replaying_missed_frames() {
        let start = Instant::now();
        let mut animation = Animation::new(start);
        let late = start + Duration::from_millis(250);
        assert!(animation.tick(late));
        assert!((animation.phase() - 0.75).abs() < 0.000001);
        assert!(!animation.tick(late));
        assert_eq!(animation.wait_duration(late), Some(FRAME_INTERVAL));
    }

    #[test]
    fn pause_blocks_and_resume_does_not_jump() {
        let start = Instant::now();
        let mut animation = Animation::new(start);
        animation.tick(start + FRAME_INTERVAL);
        let frozen = animation.phase();
        animation.set_running(false, start + Duration::from_millis(75));
        let later = start + Duration::from_secs(3600);
        assert_eq!(animation.wait_duration(later), None);
        assert!(!animation.tick(later));
        assert_eq!(animation.phase(), frozen);

        animation.set_running(true, later);
        assert_eq!(animation.phase(), frozen);
        assert_eq!(animation.wait_duration(later), Some(FRAME_INTERVAL));
        assert!(animation.tick(later + FRAME_INTERVAL));
        assert!((animation.phase() - frozen - 0.15).abs() < 0.000001);
    }

    #[test]
    fn input_does_not_reset_an_active_deadline() {
        let start = Instant::now();
        let mut animation = Animation::new(start);
        animation.set_running(true, start + Duration::from_millis(25));
        assert_eq!(
            animation.wait_duration(start + Duration::from_millis(25)),
            Some(Duration::from_millis(25))
        );
    }
}
