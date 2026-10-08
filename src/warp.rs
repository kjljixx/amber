/// Domain warping: change the sampling position, not the color points.
#[derive(Clone, Copy, Default)]
pub struct Warp {
    seed: u16,
    distortion: f32,
    swirl: f32,
    seed_offset: [f32; 2],
    time: f32,
    noise_offset: [f32; 2],
    fold_offset: [f32; 2],
    swirl_pulse: f32,
}

impl Warp {
    pub fn new(distortion: f32, swirl: f32, seed: u16) -> Result<Self, &'static str> {
        if !distortion.is_finite() || !(0.0..=1.5).contains(&distortion) {
            return Err("distortion must be finite and between 0 and 1.5");
        }
        if !swirl.is_finite() || !(-1.0..=1.0).contains(&swirl) {
            return Err("swirl must be finite and between -1 and 1");
        }
        if seed > 999 {
            return Err("seed must be between 0 and 999");
        }
        let seed_id = seed;
        let seed = f32::from(seed);
        Ok(Self {
            seed: seed_id,
            distortion,
            swirl,
            seed_offset: [fract(seed * 0.1371) * 40.0, fract(seed * 0.2113) * 40.0],
            time: 0.0,
            noise_offset: [0.0, 0.0],
            fold_offset: [0.0, 0.0],
            swirl_pulse: 1.0,
        })
    }

    pub fn at_time(mut self, time: f32) -> Self {
        let (sin, cos) = time.sin_cos();
        self.time = time;
        self.fold_offset = [0.0, 0.0];
        // Anchor the orbit at zero so the first frame matches the static warp.
        self.noise_offset = [(cos - 1.0) * 0.12, sin * 0.12];
        self.swirl_pulse = 1.0 + 0.2 * sin;
        self
    }

    pub fn seed(&self) -> u16 {
        self.seed
    }

    /// Advect the distortion field while retaining periodic swirl pulsing.
    pub fn at_flow(mut self, phase: f32, travel: f64, motion: f32) -> Self {
        self.time = 0.0;
        let distance = (travel * f64::from(motion) * 0.08) as f32;
        self.noise_offset = [-distance, -distance * 0.35];
        self.fold_offset = self.noise_offset;
        self.swirl_pulse = 1.0 + 0.2 * phase.sin();
        self
    }

    pub fn apply(&self, [mut x, mut y]: [f32; 2]) -> [f32; 2] {
        if self.swirl != 0.0 {
            // Strongest at the center, tapering toward the edges.
            let angle = self.swirl * 3.2 * (-1.4 * (x * x + y * y)).exp() * self.swirl_pulse;
            let (sin, cos) = angle.sin_cos();
            (x, y) = (cos * x + sin * y, -sin * x + cos * y);
        }

        if self.distortion == 0.0 {
            return [x, y];
        }

        let [sx, sy] = self.seed_offset;
        let t = self.time;
        let [ox, oy] = self.noise_offset;
        let [fx, fy] = self.fold_offset;
        // Broad folds, followed by two layers of organic noise displacement.
        y += self.distortion
            * (0.13 * ((x + fx) * 2.3 + sx + t).sin() + 0.05 * ((x + fx) * 4.7 + sy - t).sin());
        x += self.distortion * 0.06 * ((y + fy) * 2.9 + sy + t).sin();

        let nx = fbm([x * 1.3 + sx + ox, y * 1.3 + sy + oy]) - 0.45;
        let ny = fbm([x * 1.3 + sy + 7.3 - ox, y * 1.3 + sx + 7.3 - oy]) - 0.45;
        let mx = fbm([x * 2.4 + nx * 1.5 + sy + 3.1, y * 2.4 + ny * 1.5 + sx + 3.1]) - 0.45;
        let my = fbm([x * 2.4 + nx * 1.5 + sx + 9.7, y * 2.4 + ny * 1.5 + sy + 9.7]) - 0.45;

        [
            x + (nx * 0.35 + mx * 0.18) * self.distortion,
            y + (ny * 0.35 + my * 0.18) * self.distortion,
        ]
    }
}

// GLSL fract is always nonnegative, unlike Rust's fract for negative inputs.
fn fract(value: f32) -> f32 {
    value - value.floor()
}

fn hash([x, y]: [f32; 2]) -> f32 {
    let x = fract(x * 123.34);
    let y = fract(y * 456.21);
    let dot = x * (x + 45.32) + y * (y + 45.32);
    fract((x + dot) * (y + dot))
}

fn noise([x, y]: [f32; 2]) -> f32 {
    let ix = x.floor();
    let iy = y.floor();
    let fade = |f: f32| f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let ux = fade(fract(x));
    let uy = fade(fract(y));
    let mix = |a: f32, b: f32, amount: f32| a + (b - a) * amount;

    mix(
        mix(hash([ix, iy]), hash([ix + 1.0, iy]), ux),
        mix(hash([ix, iy + 1.0]), hash([ix + 1.0, iy + 1.0]), ux),
        uy,
    )
}

fn fbm([mut x, mut y]: [f32; 2]) -> f32 {
    let mut value = 0.0;
    let mut amplitude = 0.55;
    for _ in 0..3 {
        value += amplitude * noise([x, y]);
        // Match the column-major matrix multiplication in the reference shader.
        (x, y) = (1.6 * x - 1.2 * y, 1.2 * x + 1.6 * y);
        amplitude *= 0.45;
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_warp_preserves_coordinates_exactly() {
        for seed in [0, 58, 999] {
            let warp = Warp::new(0.0, 0.0, seed).unwrap();
            for position in [[0.0, 0.0], [-0.3, 0.2], [10.0, -4.0]] {
                assert_eq!(warp.apply(position), position);
            }
        }
    }

    #[test]
    fn swirl_preserves_radius_and_reverses_direction() {
        let clockwise = Warp::new(0.0, 0.5, 58).unwrap().apply([0.3, 0.0]);
        let counterclockwise = Warp::new(0.0, -0.5, 58).unwrap().apply([0.3, 0.0]);
        assert!((clockwise[0].hypot(clockwise[1]) - 0.3).abs() < 0.000001);
        assert!((clockwise[0] - counterclockwise[0]).abs() < 0.000001);
        assert!((clockwise[1] + counterclockwise[1]).abs() < 0.000001);
        assert!(clockwise[1] < 0.0 && counterclockwise[1] > 0.0);
        assert_eq!(
            Warp::new(0.0, 0.5, 58).unwrap().apply([0.0, 0.0]),
            [0.0, 0.0]
        );
    }

    #[test]
    fn distortion_is_repeatable_and_seeded() {
        let first = Warp::new(1.0, 0.0, 58).unwrap();
        let second = Warp::new(1.0, 0.0, 59).unwrap();
        let position = [-0.2, 0.1];
        let result = first.apply(position);
        assert_eq!(result, first.apply(position));
        assert_ne!(result, position);
        assert_ne!(result, second.apply(position));
        assert!(result.into_iter().all(f32::is_finite));
    }

    #[test]
    fn noise_is_continuous_across_negative_coordinates() {
        assert_eq!(fract(-0.25), 0.75);
        for x in [-2.0, -1.0, 0.0, 1.0] {
            let left = noise([x - 0.0001, -0.3]);
            let right = noise([x + 0.0001, -0.3]);
            assert!((left - right).abs() < 0.00001);
            assert!((0.0..=1.0).contains(&left));
        }
    }

    #[test]
    fn rejects_invalid_settings() {
        for distortion in [f32::NAN, f32::INFINITY, -0.1, 1.6] {
            assert!(Warp::new(distortion, 0.0, 58).is_err());
        }
        for swirl in [f32::NAN, f32::INFINITY, -1.1, 1.1] {
            assert!(Warp::new(0.0, swirl, 58).is_err());
        }
        assert!(Warp::new(0.0, 0.0, 1000).is_err());
    }

    #[test]
    fn animation_starts_at_the_static_warp_and_moves_continuously() {
        let warp = Warp::new(1.0, 0.45, 58).unwrap();
        let position = [-0.2, 0.1];
        let start = warp.apply(position);
        assert_eq!(start, warp.at_time(0.0).apply(position));
        let next = warp.at_time(0.001).apply(position);
        assert!(
            start
                .into_iter()
                .zip(next)
                .all(|(a, b)| (a - b).abs() < 0.002)
        );
        assert_ne!(start, warp.at_time(1.0).apply(position));
    }

    #[test]
    fn wrapping_time_keeps_the_warp_continuous() {
        let warp = Warp::new(1.0, 0.45, 58).unwrap();
        let position = [0.3, -0.1];
        let start = warp.at_time(0.0).apply(position);
        let end = warp.at_time(std::f32::consts::TAU).apply(position);
        assert!(
            start
                .into_iter()
                .zip(end)
                .all(|(a, b)| (a - b).abs() < 0.0001)
        );
    }
}
