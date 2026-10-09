use crate::color::{Oklab, Rgb, oklab_to_rgb, rgb_to_oklab};
use crate::warp::Warp;
#[cfg(test)]
use crate::warp::WarpStack;

const MAX_POINTS: usize = 256;
const RADIUS_RANGE: std::ops::RangeInclusive<f32> = 0.05..=10.0;
const STRETCH_RANGE: std::ops::RangeInclusive<f32> = 0.1..=10.0;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub position: [f32; 2],
    pub color: Rgb,
    /// Width of this point's region along its short axis, and the softness of
    /// its edges. Below 1 gives a small, crisp spot; above 1 a broad glow.
    /// Relative to the recipe's softness.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub radius: f32,
    /// How far the region reaches along its long axis, as a multiple of
    /// `radius`. Below 1 is tall (reach is `radius / stretch` vertically) and
    /// above 1 is wide (reach is `radius * stretch` horizontally). The other
    /// axis keeps `radius`. Measured in aspect-corrected space.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub stretch: f32,
}

fn one() -> f32 {
    1.0
}

fn is_one(value: &f32) -> bool {
    *value == 1.0
}

impl Point {
    pub fn new(position: [f32; 2], color: Rgb) -> Self {
        Self {
            position,
            color,
            radius: 1.0,
            stretch: 1.0,
        }
    }

    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    pub fn with_stretch(mut self, stretch: f32) -> Self {
        self.stretch = stretch;
        self
    }
}

struct LabPoint {
    position: [f32; 2],
    color: Oklab,
    /// Per-axis multipliers turning squared offsets into region-relative distance.
    metric: [f32; 2],
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
            return Err("a gradient needs between 2 and 256 points");
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
        if points
            .iter()
            .any(|point| !RADIUS_RANGE.contains(&point.radius))
        {
            return Err("point radius must be finite and between 0.05 and 10");
        }
        if points
            .iter()
            .any(|point| !STRETCH_RANGE.contains(&point.stretch))
        {
            return Err("point stretch must be finite and between 0.1 and 10");
        }

        let radius = 0.04 + 0.28 * softness;
        Ok(Self {
            points: points
                .iter()
                .map(|point| LabPoint {
                    position: point.position,
                    color: rgb_to_oklab(point.color),
                    metric: Self::metric(point.radius, point.stretch),
                })
                .collect(),
            radius_squared: radius * radius,
        })
    }

    // Stretch lengthens one axis and leaves the other at `radius`. An
    // area-preserving stretch (x by s, y by 1/s) looks tidier but also narrows
    // the short axis, so a wide band gets hard top and bottom edges and a tall
    // trunk gets a thin, hard-edged width. Making bands soft then needs
    // `radius` raised to about sqrt(stretch) on every point, which is easy to
    // forget and hard to reason about. Keeping the short axis at `radius` means
    // `radius` always sets the width and softness and `stretch` only elongates.
    fn metric(radius: f32, stretch: f32) -> [f32; 2] {
        let wide = stretch.max(1.0);
        let tall = stretch.min(1.0);
        let radius_squared = radius * radius;
        [
            1.0 / (radius_squared * wide * wide),
            tall * tall / radius_squared,
        ]
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
                    metric: point.metric,
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
                amplitude * (wander_path(time, key) - wander_path(0.0, key)) * self.x_scale;
            point.position[1] +=
                amplitude * (wander_path(time, key + 1) - wander_path(0.0, key + 1));
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
            *distance = (position[0] - point.position[0]).powi(2) * point.metric[0]
                + (position[1] - point.position[1]).powi(2) * point.metric[1];
            nearest = nearest.min(*distance);
        }

        let mut lab = [0.0; 3];
        let mut total_weight = 0.0;
        for (point, distance) in self.points.iter().zip(distances) {
            // Distances are relative to each point's own region. The nearest has
            // weight 1, avoiding all weights underflowing.
            let weight = (-(distance - nearest) / self.radius_squared).exp();
            for (channel, color) in lab.iter_mut().zip(point.color) {
                *channel += color * weight;
            }
            total_weight += weight;
        }

        oklab_to_rgb(lab.map(|channel| channel / total_weight))
    }
}

// Independent phases and rates prevent all coordinates from stopping at the
// same noise-cell boundaries. Layering paths keeps motion alive when one slows.
// Positive weights sum to one, preserving the existing displacement bounds.
fn wander_path(time: f64, seed: u64) -> f32 {
    let phase = (seed.wrapping_mul(73) % 997) as f64 / 997.0 * 11.0;
    let rate = 0.85 + (seed.wrapping_mul(37) % 101) as f64 / 101.0 * 0.3;
    let time = time * rate;
    0.6 * wander_noise(time + phase, seed)
        + 0.28 * wander_noise(time * 1.73 + phase * 2.31 + 0.37, seed.wrapping_add(1009))
        + 0.12 * wander_noise(time * 2.91 + phase * 0.71 + 0.83, seed.wrapping_add(2017))
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
                Point::new([0.0, 0.5], [0, 0, 0]),
                Point::new([1.0, 0.5], [255, 255, 255]),
            ],
            softness,
        )
        .unwrap()
    }

    #[test]
    fn wandering_coordinates_do_not_share_stops_at_integer_times() {
        for time in [0.0, 1.0, 2.0, 3.0, 4.0, 10.0] {
            let moving = (0..36)
                .filter(|index| {
                    let seed = 58 * 131 + index;
                    let velocity =
                        (wander_path(time + 0.001, seed) - wander_path(time, seed)).abs();
                    velocity > 0.00001
                })
                .count();
            assert!(
                moving > 24,
                "shared motion pause at {time}: {moving} moving coordinates"
            );
        }
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
            &[Point::new([0.0, 0.0], color), Point::new([1.0, 1.0], color)],
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
    fn default_radius_and_stretch_match_plain_distance_blending() {
        let points = [
            Point::new([0.1, 0.2], [200, 40, 40]),
            Point::new([0.8, 0.3], [30, 90, 220]),
            Point::new([0.4, 0.9], [240, 220, 90]),
        ];
        let softness = 0.4;
        let field = Gradient::new(&points, softness).unwrap().prepare(1.6);
        let scale = 1.6_f32.sqrt();
        let radius = 0.04 + 0.28 * softness;
        for (x, y) in [(0.0, 0.0), (0.3, 0.5), (0.6, 0.6), (1.0, 1.0)] {
            let distances = points.each_ref().map(|point| {
                ((x - point.position[0]) * scale).powi(2) + (y - point.position[1]).powi(2)
            });
            let nearest = distances.into_iter().fold(f32::INFINITY, f32::min);
            let mut lab = [0.0; 3];
            let mut total = 0.0;
            for (point, distance) in points.iter().zip(distances) {
                let weight = (-(distance - nearest) / (radius * radius)).exp();
                for (channel, color) in lab.iter_mut().zip(rgb_to_oklab(point.color)) {
                    *channel += color * weight;
                }
                total += weight;
            }
            assert_eq!(field.sample(x, y), oklab_to_rgb(lab.map(|c| c / total)));
        }
    }

    // A bright point among four dark ones, for comparing how far it reaches.
    fn lit(light: Point) -> PreparedGradient {
        let dark = |x, y| Point::new([x, y], [0, 0, 0]);
        Gradient::new(
            &[
                light,
                dark(0.2, 0.5),
                dark(0.8, 0.5),
                dark(0.5, 0.2),
                dark(0.5, 0.8),
            ],
            0.3,
        )
        .unwrap()
        .prepare(1.0)
    }

    #[test]
    fn smaller_radius_shrinks_a_points_region() {
        let white = Point::new([0.5, 0.5], [255, 255, 255]);
        let normal = lit(white.clone()).sample(0.6, 0.5)[0];
        let small = lit(white.with_radius(0.3)).sample(0.6, 0.5)[0];
        assert!(normal > 120, "{normal}");
        assert!(small < 30, "{small}");
    }

    #[test]
    fn small_radius_gives_crisper_edges() {
        let width = |radius: f32| {
            let field = lit(Point::new([0.5, 0.5], [255, 255, 255]).with_radius(radius));
            // Distance over which the white falls from 90% to 10%.
            let at = |level: f32| {
                (0..2000)
                    .map(|i| 0.5 + i as f32 * 0.0001)
                    .find(|&x| f32::from(field.sample(x, 0.5)[0]) < level * 255.0)
                    .unwrap()
            };
            at(0.1) - at(0.9)
        };
        assert!(width(0.4) < width(1.0) * 0.6);
    }

    #[test]
    fn stretch_reaches_farther_along_the_long_axis() {
        let tall = lit(Point::new([0.5, 0.5], [255, 255, 255]).with_stretch(0.25));
        let (along, across) = (tall.sample(0.5, 0.65)[0], tall.sample(0.65, 0.5)[0]);
        assert!(along > across + 60, "{along} vs {across}");
        // The short axis keeps the point's own radius, so stretching never
        // narrows or sharpens it.
        let plain = lit(Point::new([0.5, 0.5], [255, 255, 255]));
        assert_eq!(across, plain.sample(0.65, 0.5)[0]);
        let wide = lit(Point::new([0.5, 0.5], [255, 255, 255]).with_stretch(4.0));
        let (along, across) = (wide.sample(0.65, 0.5)[0], wide.sample(0.5, 0.65)[0]);
        assert!(along > across + 60, "{along} vs {across}");
        assert_eq!(across, plain.sample(0.5, 0.65)[0]);
    }

    #[test]
    fn point_attributes_are_validated_and_capped() {
        let base = [
            Point::new([0.0, 0.0], [0, 0, 0]),
            Point::new([1.0, 1.0], [255, 255, 255]),
        ];
        for radius in [0.0, 0.04, 10.1, f32::NAN, f32::INFINITY, -1.0] {
            let mut points = base.clone();
            points[0].radius = radius;
            assert!(Gradient::new(&points, 0.5).is_err(), "radius {radius}");
        }
        for stretch in [0.0, 0.09, 10.1, f32::NAN, f32::INFINITY, -1.0] {
            let mut points = base.clone();
            points[1].stretch = stretch;
            assert!(Gradient::new(&points, 0.5).is_err(), "stretch {stretch}");
        }
        let mut points = base.to_vec();
        points[0].radius = 0.05;
        points[1].stretch = 10.0;
        assert!(Gradient::new(&points, 0.5).is_ok());

        let many = |count: usize| vec![base[0].clone(); count];
        assert!(Gradient::new(&many(MAX_POINTS), 0.5).is_ok());
        assert!(Gradient::new(&many(MAX_POINTS + 1), 0.5).is_err());
    }

    #[test]
    fn json_points_default_missing_attributes_and_omit_defaults() {
        let plain: Point =
            serde_json::from_str(r#"{"position":[0.1,0.2],"color":[1,2,3]}"#).unwrap();
        assert_eq!((plain.radius, plain.stretch), (1.0, 1.0));
        assert_eq!(
            serde_json::to_string(&plain).unwrap(),
            r#"{"position":[0.1,0.2],"color":[1,2,3]}"#
        );
        let shaped: Point = serde_json::from_str(
            r#"{"position":[0.1,0.2],"color":[1,2,3],"radius":0.5,"stretch":3.0}"#,
        )
        .unwrap();
        assert_eq!((shaped.radius, shaped.stretch), (0.5, 3.0));
        assert!(
            serde_json::to_string(&shaped)
                .unwrap()
                .contains("\"stretch\":3.0")
        );
        assert!(
            serde_json::from_str::<Point>(r#"{"position":[0.1,0.2],"color":[1,2,3],"radus":0.5}"#)
                .is_err()
        );
    }

    #[test]
    fn rejects_invalid_recipes() {
        assert!(Gradient::new(&[], 0.5).is_err());
        let points = [
            Point::new([0.0, 0.0], [0, 0, 0]),
            Point::new([1.0, 1.0], [255, 255, 255]),
        ];
        for softness in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(Gradient::new(&points, softness).is_err());
        }
        let invalid = [
            Point::new([f32::NAN, 0.0], [0, 0, 0]),
            Point::new([1.0, 1.0], [255, 255, 255]),
        ];
        assert!(Gradient::new(&invalid, 0.5).is_err());
    }

    #[test]
    fn warp_changes_the_field_and_can_be_disabled() {
        let gradient = pair(0.5);
        let plain = gradient.prepare(1.0);
        let disabled = gradient
            .prepare(1.0)
            .with_warp(WarpStack::new(0.0, 0.0, 58, &[]).unwrap().at_rest(1.0));
        let warped = gradient
            .prepare(1.0)
            .with_warp(WarpStack::new(1.0, 0.45, 58, &[]).unwrap().at_rest(1.0));
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
