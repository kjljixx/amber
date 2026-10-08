use crate::color::{Oklab, Rgb, oklab_to_rgb, rgb_to_oklab};
use crate::warp::Warp;

const MAX_POINTS: usize = 24;

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
