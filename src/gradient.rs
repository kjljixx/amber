use crate::color::{Oklab, Rgb, oklab_to_rgb, rgb_to_oklab};
use crate::warp::Warp;

const MAX_POINTS: usize = 24;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub position: [f32; 2],
    pub color: Rgb,
}

struct LabPoint {
    position: [f32; 2],
    color: Oklab,
}

pub struct Gradient {
    points: Vec<LabPoint>,
    radius_squared: f32,
}

/// Positions are prepared for a particular preview aspect ratio.
pub struct PreparedGradient {
    points: Vec<LabPoint>,
    x_scale: f32,
    radius_squared: f32,
    warp: Warp,
}

impl Gradient {
    pub fn new(points: &[Point], softness: f32) -> Result<Self, &'static str> {
        if !(2..=MAX_POINTS).contains(&points.len()) {
            return Err("a gradient needs between 2 and 24 points");
        }
        if !softness.is_finite() || !(0.0..=1.0).contains(&softness) {
            return Err("softness must be finite and between 0 and 1");
        }
        if points.iter().any(|point| {
            point
                .position
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        }) {
            return Err("point coordinates must be finite and between 0 and 1");
        }

        let radius = 0.04 + 0.28 * softness;
        Ok(Self {
            points: points
                .iter()
                .map(|point| LabPoint {
                    position: point.position,
                    color: rgb_to_oklab(point.color),
                })
                .collect(),
            radius_squared: radius * radius,
        })
    }

    pub fn prepare(&self, aspect: f32) -> PreparedGradient {
        assert!(aspect.is_finite() && aspect > 0.0);
        let x_scale = aspect.sqrt();
        PreparedGradient {
            points: self
                .points
                .iter()
                .map(|point| LabPoint {
                    position: [(point.position[0] - 0.5) * x_scale, point.position[1] - 0.5],
                    color: point.color,
                })
                .collect(),
            x_scale,
            radius_squared: self.radius_squared,
            warp: Warp::default(),
        }
    }
}

impl PreparedGradient {
    /// Apply Mesh's looping point motion once per frame, before pixel sampling.
    pub fn with_drift(mut self, phase: f32, motion: f32) -> Self {
        assert!(phase.is_finite());
        assert!(motion.is_finite() && (0.0..=1.5).contains(&motion));
        if motion == 0.0 {
            return self;
        }
        let amplitude = motion * 0.06;
        for (index, point) in self.points.iter_mut().enumerate() {
            let index = index as f32;
            point.position[0] += amplitude * (phase + index * 1.7).sin() * self.x_scale;
            point.position[1] += amplitude * (phase + index * 2.3).cos();
        }
        self
    }

    /// Smooth seeded motion with no prescribed repeating cycle.
    /// Subtract the initial noise value so time zero preserves the layout.
    pub fn with_wander(mut self, travel: f64, motion: f32, seed: u16) -> Self {
        assert!(travel.is_finite() && travel >= 0.0);
        assert!(motion.is_finite() && (0.0..=1.5).contains(&motion));
        let time = travel * 0.18;
        let amplitude = motion * 0.06;
        for (index, point) in self.points.iter_mut().enumerate() {
            let key = u64::from(seed) * 131 + index as u64 * 2;
            point.position[0] +=
                amplitude * (wander_noise(time, key) - wander_noise(0.0, key)) * self.x_scale;
            point.position[1] +=
                amplitude * (wander_noise(time, key + 1) - wander_noise(0.0, key + 1));
        }
        self
    }

    pub fn with_warp(mut self, warp: Warp) -> Self {
        self.warp = warp;
        self
    }

    pub fn sample(&self, x: f32, y: f32) -> Rgb {
        let position = self.warp.apply([(x - 0.5) * self.x_scale, y - 0.5]);
        let mut distances = [0.0; MAX_POINTS];
        let mut nearest = f32::INFINITY;

        for (point, distance) in self.points.iter().zip(&mut distances) {
            *distance = (position[0] - point.position[0]).powi(2)
                + (position[1] - point.position[1]).powi(2);
            nearest = nearest.min(*distance);
        }

        let mut lab = [0.0; 3];
        let mut total_weight = 0.0;
        for (point, distance) in self.points.iter().zip(distances) {
            // The nearest point has weight 1, avoiding all weights underflowing.
            let weight = (-(distance - nearest) / self.radius_squared).exp();
            for (channel, color) in lab.iter_mut().zip(point.color) {
                *channel += color * weight;
            }
            total_weight += weight;
        }

        oklab_to_rgb(lab.map(|channel| channel / total_weight))
    }
}

// Smooth one-dimensional value noise. Integer hashing avoids the precision
// loss of floating-point sine hashes and makes trajectories deterministic.
fn wander_noise(time: f64, seed: u64) -> f32 {
    let cell = time.floor() as u64;
    let value = |index: u64| {
        let mut hash = index.wrapping_add(seed.wrapping_mul(0x9e3779b97f4a7c15));
        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d049bb133111eb);
        hash ^= hash >> 31;
        (hash >> 40) as f64 / ((1u64 << 24) - 1) as f64 * 2.0 - 1.0
    };
    let fraction = time - time.floor();
    let fade = fraction * fraction * fraction * (fraction * (fraction * 6.0 - 15.0) + 10.0);
    (value(cell) * (1.0 - fade) + value(cell.wrapping_add(1)) * fade) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(softness: f32) -> Gradient {
        Gradient::new(
            &[
                Point {
                    position: [0.0, 0.5],
                    color: [0, 0, 0],
                },
                Point {
                    position: [1.0, 0.5],
                    color: [255, 255, 255],
                },
            ],
            softness,
        )
        .unwrap()
    }

    #[test]
    fn wandering_is_bounded_seeded_and_smooth_across_noise_cells() {
        let gradient = pair(0.5);
        let fixed = gradient.prepare(2.0);
        for time in [0.0, 1.0, 10.0, 1000.0, 1000000.0] {
            let field = gradient.prepare(2.0).with_wander(time, 0.8, 58);
            for (moving, anchor) in field.points.iter().zip(&fixed.points) {
                assert!(
                    (moving.position[0] - anchor.position[0]).abs()
                        <= 0.8 * 0.12 * field.x_scale + 1e-6
                );
                assert!((moving.position[1] - anchor.position[1]).abs() <= 0.8 * 0.12 + 1e-6);
            }
        }
        let boundary = 1.0 / 0.18;
        let before = gradient.prepare(2.0).with_wander(boundary - 1e-5, 0.8, 58);
        let after = gradient.prepare(2.0).with_wander(boundary + 1e-5, 0.8, 58);
        for (a, b) in before.points.iter().zip(&after.points) {
            assert!((a.position[0] - b.position[0]).abs() < 1e-5);
            assert!((a.position[1] - b.position[1]).abs() < 1e-5);
        }
        assert_ne!(wander_noise(2.5, 58), wander_noise(2.5, 59));
    }

    #[test]
    fn drift_uses_point_phases_and_aspect_corrected_amplitude() {
        let field = pair(0.5).prepare(4.0).with_drift(0.7, 0.5);
        let base = pair(0.5).prepare(4.0);
        for (index, (moving, fixed)) in field.points.iter().zip(&base.points).enumerate() {
            let index = index as f32;
            assert!(
                (moving.position[0] - fixed.position[0] - 0.06 * (0.7 + index * 1.7).sin()).abs()
                    < 0.000001
            );
            assert!(
                (moving.position[1] - fixed.position[1] - 0.03 * (0.7 + index * 2.3).cos()).abs()
                    < 0.000001
            );
            assert_eq!(moving.color, fixed.color);
        }
    }

    #[test]
    fn drift_moves_the_field_and_loops_without_a_jump() {
        let gradient = pair(0.5);
        let start = gradient.prepare(1.0).with_drift(0.7, 0.5);
        let later = gradient.prepare(1.0).with_drift(2.0, 0.5);
        assert_ne!(start.sample(0.5, 0.5), later.sample(0.5, 0.5));
        let looped = gradient
            .prepare(1.0)
            .with_drift(0.7 + std::f32::consts::TAU, 0.5);
        for (a, b) in start.points.iter().zip(&looped.points) {
            for axis in 0..2 {
                assert!((a.position[axis] - b.position[axis]).abs() < 0.000001);
            }
        }
    }

    #[test]
    fn zero_drift_preserves_static_samples() {
        let gradient = pair(0.5);
        let base = gradient.prepare(1.0);
        let disabled = gradient.prepare(1.0).with_drift(1.2, 0.0);
        for y in 0..10 {
            for x in 0..10 {
                let (x, y) = (x as f32 / 9.0, y as f32 / 9.0);
                assert_eq!(base.sample(x, y), disabled.sample(x, y));
            }
        }
    }

    #[test]
    fn midpoint_is_a_perceptual_blend() {
        let field = pair(0.5).prepare(1.0);
        let midpoint = field.sample(0.5, 0.5);
        for channel in midpoint {
            assert!(channel.abs_diff(99) <= 1);
        }
        assert_eq!(midpoint, field.sample(0.5, 0.5));
    }

    #[test]
    fn softness_widens_the_transition() {
        let crisp = pair(0.0).prepare(1.0).sample(0.45, 0.5)[0];
        let soft = pair(1.0).prepare(1.0).sample(0.45, 0.5)[0];
        assert_eq!(crisp, 0);
        assert!(soft > 0 && soft < 99);
    }

    #[test]
    fn identical_colors_stay_constant_at_extreme_aspects() {
        let color = [45, 180, 210];
        let gradient = Gradient::new(
            &[
                Point {
                    position: [0.0, 0.0],
                    color,
                },
                Point {
                    position: [1.0, 1.0],
                    color,
                },
            ],
            0.0,
        )
        .unwrap();
        for aspect in [0.001, 1.0, 1000.0] {
            let field = gradient.prepare(aspect);
            for (x, y) in [(0.0, 0.0), (0.5, 0.5), (1.0, 0.0), (1.0, 1.0)] {
                assert_eq!(field.sample(x, y), color);
            }
        }
    }

    #[test]
    fn rejects_invalid_recipes() {
        assert!(Gradient::new(&[], 0.5).is_err());
        let points = [
            Point {
                position: [0.0, 0.0],
                color: [0, 0, 0],
            },
            Point {
                position: [1.0, 1.0],
                color: [255, 255, 255],
            },
        ];
        for softness in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(Gradient::new(&points, softness).is_err());
        }
        let invalid = [
            Point {
                position: [f32::NAN, 0.0],
                color: [0, 0, 0],
            },
            Point {
                position: [1.0, 1.0],
                color: [255, 255, 255],
            },
        ];
        assert!(Gradient::new(&invalid, 0.5).is_err());
    }

    #[test]
    fn warp_changes_the_field_and_can_be_disabled() {
        let gradient = pair(0.5);
        let plain = gradient.prepare(1.0);
        let disabled = gradient
            .prepare(1.0)
            .with_warp(Warp::new(0.0, 0.0, 58).unwrap());
        let warped = gradient
            .prepare(1.0)
            .with_warp(Warp::new(1.0, 0.45, 58).unwrap());
        let mut changed = 0;
        for row in 0..10 {
            for column in 0..10 {
                let x = (column as f32 + 0.5) / 10.0;
                let y = (row as f32 + 0.5) / 10.0;
                assert_eq!(plain.sample(x, y), disabled.sample(x, y));
                if plain.sample(x, y) != warped.sample(x, y) {
                    changed += 1;
                }
            }
        }
        assert!(changed > 10);
    }
}
