//! Domain warping: change the sampling position, not the color points.
//!
//! A warp is an ordered stack of layers, each a small displacement of the
//! sampling position. A recipe's `distortion`, `swirl` and `seed` expand to the
//! classic stack (swirl, two fold layers, two-step noise); any `warp` layers in
//! the recipe run after it. Adding an effect means one [`WarpLayer`] variant,
//! one validation arm, one [`Step`] variant and its `apply`.
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

const MAX_LAYERS: usize = 16;
const MAX_WAVES: usize = 4;

/// How a layer moves. `Cycle` loops with the animation phase; `Drift` slides
/// without repeating; `Static` holds still.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerMotion {
    Static,
    Cycle,
    Drift,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WarpAxis {
    X,
    Y,
}

/// Per-layer animation. Layers that omit it follow the recipe's `motion_style`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayerAnimation {
    /// Defaults to the recipe's style: `loop` cycles; `flow` and `wander` drift.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<LayerMotion>,
    /// Drift direction; its length scales the distance travelled.
    #[serde(default = "default_direction")]
    pub direction: [f32; 2],
    /// Multiplies drift speed.
    #[serde(default = "one")]
    pub speed: f32,
}

impl Default for LayerAnimation {
    fn default() -> Self {
        Self {
            mode: None,
            direction: default_direction(),
            speed: 1.0,
        }
    }
}

impl LayerAnimation {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    fn validate(&self) -> Result<(), &'static str> {
        if !within(self.speed, 0.0..=10.0) {
            return Err("warp animation speed must be finite and between 0 and 10");
        }
        if !self.direction.iter().all(|&v| within(v, -5.0..=5.0)) {
            return Err("warp animation direction must be finite, each between -5 and 5");
        }
        Ok(())
    }
}

/// One sine wave of a `folds` layer.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FoldWave {
    pub amplitude: f32,
    pub frequency: f32,
    /// Phase advance per unit of animation phase when cycling; the sign sets
    /// the direction, so a negative speed runs the wave backwards.
    #[serde(default = "one")]
    pub speed: f32,
    /// Starting phase. Defaults to an offset derived from the layer's seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WarpLayer {
    /// Rotates the sampling position by an angle that is strongest at `center`.
    Swirl {
        strength: f32,
        #[serde(default = "default_falloff")]
        falloff: f32,
        #[serde(default = "default_center")]
        center: [f32; 2],
        /// Fraction of strength that pulses with the animation phase.
        #[serde(default = "default_pulse")]
        pulse: f32,
    },
    /// Sine waves of one coordinate displace the other, bending bands like
    /// draped cloth. `displace: "y"` makes horizontal bands wave up and down.
    Folds {
        displace: WarpAxis,
        amplitude: f32,
        waves: Vec<FoldWave>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed: Option<u16>,
        #[serde(default, skip_serializing_if = "LayerAnimation::is_default")]
        animation: LayerAnimation,
    },
    /// Two-step organic noise displacement.
    Noise {
        strength: f32,
        /// Feature frequency per axis; lower values stretch features along it.
        #[serde(default = "default_ones")]
        frequency: [f32; 2],
        /// Displacement per axis; `[1, 0.3]` keeps vertical movement small.
        #[serde(default = "default_ones")]
        weight: [f32; 2],
        /// Radius of the noise field's loop when cycling.
        #[serde(default = "default_orbit")]
        orbit: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed: Option<u16>,
        #[serde(default, skip_serializing_if = "LayerAnimation::is_default")]
        animation: LayerAnimation,
    },
    /// Rotates the whole composition about `center`, in degrees.
    Tilt {
        angle: f32,
        #[serde(default = "default_center")]
        center: [f32; 2],
    },
    /// Radial waves from `center`. `frequency` is in waves per unit radius.
    Ripple {
        amplitude: f32,
        frequency: f32,
        #[serde(default = "default_center")]
        center: [f32; 2],
        /// Exponential fade with distance from the center.
        #[serde(default)]
        decay: f32,
        #[serde(default, skip_serializing_if = "LayerAnimation::is_default")]
        animation: LayerAnimation,
    },
}

fn one() -> f32 {
    1.0
}
fn default_ones() -> [f32; 2] {
    [1.0, 1.0]
}
fn default_center() -> [f32; 2] {
    [0.5, 0.5]
}
fn default_falloff() -> f32 {
    1.4
}
fn default_pulse() -> f32 {
    0.2
}
fn default_orbit() -> f32 {
    0.12
}
fn default_direction() -> [f32; 2] {
    [-1.0, -0.35]
}

fn within(value: f32, range: RangeInclusive<f32>) -> bool {
    range.contains(&value)
}

fn valid_center(center: &[f32; 2]) -> bool {
    center.iter().all(|&v| within(v, -0.5..=1.5))
}

impl WarpLayer {
    fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Swirl {
                strength,
                falloff,
                center,
                pulse,
            } => {
                if !within(*strength, -1.0..=1.0) {
                    return Err("swirl must be finite and between -1 and 1");
                }
                if !within(*falloff, 0.05..=10.0) {
                    return Err("swirl falloff must be finite and between 0.05 and 10");
                }
                if !valid_center(center) {
                    return Err("warp center must be finite, each coordinate -0.5 to 1.5");
                }
                if !within(*pulse, 0.0..=1.0) {
                    return Err("swirl pulse must be finite and between 0 and 1");
                }
            }
            Self::Folds {
                amplitude,
                waves,
                seed,
                animation,
                ..
            } => {
                if !within(*amplitude, 0.0..=1.5) {
                    return Err("folds amplitude must be finite and between 0 and 1.5");
                }
                if !(1..=MAX_WAVES).contains(&waves.len()) {
                    return Err("folds need between 1 and 4 waves");
                }
                for wave in waves {
                    if !within(wave.amplitude, 0.0..=0.5) {
                        return Err("fold wave amplitude must be finite and between 0 and 0.5");
                    }
                    if !within(wave.frequency, 0.1..=20.0) {
                        return Err("fold wave frequency must be finite and between 0.1 and 20");
                    }
                    if !within(wave.speed, -4.0..=4.0) {
                        return Err("fold wave speed must be finite and between -4 and 4");
                    }
                    if wave
                        .phase
                        .is_some_and(|phase| !within(phase, -100.0..=100.0))
                    {
                        return Err("fold wave phase must be finite and between -100 and 100");
                    }
                }
                check_seed(*seed)?;
                animation.validate()?;
            }
            Self::Noise {
                strength,
                frequency,
                weight,
                orbit,
                seed,
                animation,
            } => {
                if !within(*strength, 0.0..=1.5) {
                    return Err("noise strength must be finite and between 0 and 1.5");
                }
                if !frequency.iter().all(|&v| within(v, 0.1..=10.0)) {
                    return Err("noise frequency must be finite, each between 0.1 and 10");
                }
                if !weight.iter().all(|&v| within(v, 0.0..=2.0)) {
                    return Err("noise weight must be finite, each between 0 and 2");
                }
                if !within(*orbit, 0.0..=1.0) {
                    return Err("noise orbit must be finite and between 0 and 1");
                }
                check_seed(*seed)?;
                animation.validate()?;
            }
            Self::Tilt { angle, center } => {
                if !within(*angle, -90.0..=90.0) {
                    return Err("tilt angle must be finite and between -90 and 90 degrees");
                }
                if !valid_center(center) {
                    return Err("warp center must be finite, each coordinate -0.5 to 1.5");
                }
            }
            Self::Ripple {
                amplitude,
                frequency,
                center,
                decay,
                animation,
            } => {
                if !within(*amplitude, 0.0..=0.5) {
                    return Err("ripple amplitude must be finite and between 0 and 0.5");
                }
                if !within(*frequency, 0.5..=60.0) {
                    return Err("ripple frequency must be finite and between 0.5 and 60");
                }
                if !valid_center(center) {
                    return Err("warp center must be finite, each coordinate -0.5 to 1.5");
                }
                if !within(*decay, 0.0..=20.0) {
                    return Err("ripple decay must be finite and between 0 and 20");
                }
                animation.validate()?;
            }
        }
        Ok(())
    }
}

fn check_seed(seed: Option<u16>) -> Result<(), &'static str> {
    if seed.is_some_and(|seed| seed > 999) {
        return Err("seed must be between 0 and 999");
    }
    Ok(())
}

fn seed_offsets(seed: u16) -> [f32; 2] {
    let seed = f32::from(seed);
    [fract(seed * 0.1371) * 40.0, fract(seed * 0.2113) * 40.0]
}

/// Time inputs for one frame.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    /// Wrapped animation phase, in radians.
    pub phase: f32,
    /// Unwrapped travel for drifting layers.
    pub travel: f64,
    /// Effective motion strength; zero stops drifting.
    pub motion: f32,
    /// Width over height of the preview, in pixels.
    pub aspect: f32,
    /// What layers without their own mode do.
    pub inherited: LayerMotion,
}

impl Clock {
    #[cfg(test)]
    pub fn rest(aspect: f32) -> Self {
        Self {
            phase: 0.0,
            travel: 0.0,
            motion: 0.0,
            aspect,
            inherited: LayerMotion::Cycle,
        }
    }
}

/// Validated layers, ready to be posed for a frame.
#[derive(Clone, Debug)]
pub struct WarpStack {
    seed: u16,
    layers: Vec<WarpLayer>,
}

impl WarpStack {
    /// `distortion`, `swirl` and `seed` expand to the classic stack; `extra`
    /// layers follow it.
    pub fn new(
        distortion: f32,
        swirl: f32,
        seed: u16,
        extra: &[WarpLayer],
    ) -> Result<Self, &'static str> {
        if !distortion.is_finite() || !(0.0..=1.5).contains(&distortion) {
            return Err("distortion must be finite and between 0 and 1.5");
        }
        if !swirl.is_finite() || !(-1.0..=1.0).contains(&swirl) {
            return Err("swirl must be finite and between -1 and 1");
        }
        if seed > 999 {
            return Err("seed must be between 0 and 999");
        }
        let mut layers = Vec::new();
        if swirl != 0.0 {
            layers.push(WarpLayer::Swirl {
                strength: swirl,
                falloff: default_falloff(),
                center: default_center(),
                pulse: default_pulse(),
            });
        }
        if distortion != 0.0 {
            let [sx, sy] = seed_offsets(seed);
            let wave = |amplitude, frequency, speed, phase| FoldWave {
                amplitude,
                frequency,
                speed,
                phase: Some(phase),
            };
            // Broad folds. The second layer carries the strength in its wave and
            // uses amplitude 1 so rounding matches the original fixed formula
            // `x += distortion * 0.06 * sin(..)` bit for bit.
            layers.push(WarpLayer::Folds {
                displace: WarpAxis::Y,
                amplitude: distortion,
                waves: vec![wave(0.13, 2.3, 1.0, sx), wave(0.05, 4.7, -1.0, sy)],
                seed: None,
                animation: LayerAnimation::default(),
            });
            layers.push(WarpLayer::Folds {
                displace: WarpAxis::X,
                amplitude: 1.0,
                waves: vec![wave(distortion * 0.06, 2.9, 1.0, sy)],
                seed: None,
                animation: LayerAnimation::default(),
            });
            layers.push(WarpLayer::Noise {
                strength: distortion,
                frequency: default_ones(),
                weight: default_ones(),
                orbit: default_orbit(),
                seed: None,
                animation: LayerAnimation::default(),
            });
        }
        layers.extend_from_slice(extra);
        if layers.len() > MAX_LAYERS {
            return Err("a recipe supports at most 16 warp layers including the classic ones");
        }
        for layer in &layers {
            layer.validate()?;
        }
        Ok(Self { seed, layers })
    }

    pub fn seed(&self) -> u16 {
        self.seed
    }

    /// The pose with all motion at its starting point.
    #[cfg(test)]
    pub fn at_rest(&self, aspect: f32) -> Warp {
        self.at(&Clock::rest(aspect))
    }

    pub fn at(&self, clock: &Clock) -> Warp {
        assert!(clock.aspect.is_finite() && clock.aspect > 0.0);
        let (sin, cos) = clock.phase.sin_cos();
        let x_scale = clock.aspect.sqrt();
        let space = |center: [f32; 2]| [(center[0] - 0.5) * x_scale, center[1] - 0.5];
        // Drift distance in the same units for every layer; `speed` scales it.
        let distance = |animation: &LayerAnimation| {
            (clock.travel * f64::from(clock.motion) * 0.08 * f64::from(animation.speed)) as f32
        };
        let steps = self
            .layers
            .iter()
            .map(|layer| match layer {
                WarpLayer::Swirl {
                    strength,
                    falloff,
                    center,
                    pulse,
                } => Step::Swirl {
                    strength: *strength,
                    falloff: *falloff,
                    center: space(*center),
                    pulse: 1.0 + pulse * sin,
                },
                WarpLayer::Folds {
                    displace,
                    amplitude,
                    waves,
                    seed,
                    animation,
                } => {
                    let [sx, sy] = seed_offsets(seed.unwrap_or(self.seed));
                    let mode = animation.mode.unwrap_or(clock.inherited);
                    let time = if mode == LayerMotion::Cycle {
                        clock.phase
                    } else {
                        0.0
                    };
                    let drift = drift_offset(mode, animation, distance(animation));
                    Step::Folds {
                        displace: *displace,
                        amplitude: *amplitude,
                        waves: waves
                            .iter()
                            .enumerate()
                            .map(|(index, wave)| Wave {
                                amplitude: wave.amplitude,
                                frequency: wave.frequency,
                                phase: wave.phase.unwrap_or(if index % 2 == 0 { sx } else { sy }),
                                shift: wave.speed * time,
                            })
                            .collect(),
                        // The driving coordinate's component of the drift.
                        offset: match displace {
                            WarpAxis::Y => drift[0],
                            WarpAxis::X => drift[1],
                        },
                    }
                }
                WarpLayer::Noise {
                    strength,
                    frequency,
                    weight,
                    orbit,
                    seed,
                    animation,
                } => {
                    let mode = animation.mode.unwrap_or(clock.inherited);
                    let offset = match mode {
                        LayerMotion::Cycle => [(cos - 1.0) * orbit, sin * orbit],
                        _ => drift_offset(mode, animation, distance(animation)),
                    };
                    Step::Noise {
                        seed_offset: seed_offsets(seed.unwrap_or(self.seed)),
                        offset,
                        near: [1.3 * frequency[0], 1.3 * frequency[1]],
                        far: [2.4 * frequency[0], 2.4 * frequency[1]],
                        strength: [strength * weight[0], strength * weight[1]],
                    }
                }
                WarpLayer::Tilt { angle, center } => {
                    let (sin, cos) = angle.to_radians().sin_cos();
                    Step::Tilt {
                        sin,
                        cos,
                        center: space(*center),
                    }
                }
                WarpLayer::Ripple {
                    amplitude,
                    frequency,
                    center,
                    decay,
                    animation,
                } => {
                    let mode = animation.mode.unwrap_or(clock.inherited);
                    Step::Ripple {
                        amplitude: *amplitude,
                        frequency: *frequency,
                        center: space(*center),
                        decay: *decay,
                        // Drifting waves travel outward at the drift distance.
                        shift: match mode {
                            LayerMotion::Static => 0.0,
                            LayerMotion::Cycle => clock.phase,
                            LayerMotion::Drift => frequency * distance(animation),
                        },
                    }
                }
            })
            .collect();
        Warp { steps }
    }
}

fn drift_offset(mode: LayerMotion, animation: &LayerAnimation, distance: f32) -> [f32; 2] {
    if mode == LayerMotion::Drift {
        [
            animation.direction[0] * distance,
            animation.direction[1] * distance,
        ]
    } else {
        [0.0, 0.0]
    }
}

#[derive(Clone, Debug)]
struct Wave {
    amplitude: f32,
    frequency: f32,
    phase: f32,
    shift: f32,
}

/// A layer posed for one frame, with every per-frame value precomputed.
#[derive(Clone, Debug)]
enum Step {
    Swirl {
        strength: f32,
        falloff: f32,
        center: [f32; 2],
        pulse: f32,
    },
    Folds {
        displace: WarpAxis,
        amplitude: f32,
        waves: Vec<Wave>,
        offset: f32,
    },
    Noise {
        seed_offset: [f32; 2],
        offset: [f32; 2],
        near: [f32; 2],
        far: [f32; 2],
        strength: [f32; 2],
    },
    Tilt {
        sin: f32,
        cos: f32,
        center: [f32; 2],
    },
    Ripple {
        amplitude: f32,
        frequency: f32,
        center: [f32; 2],
        decay: f32,
        shift: f32,
    },
}

/// Posed layers for one frame. The default does nothing.
#[derive(Clone, Debug, Default)]
pub struct Warp {
    steps: Vec<Step>,
}

impl Warp {
    pub fn apply(&self, position: [f32; 2]) -> [f32; 2] {
        self.steps
            .iter()
            .fold(position, |position, step| step.apply(position))
    }
}

impl Step {
    fn apply(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        match self {
            Self::Swirl {
                strength,
                falloff,
                center,
                pulse,
            } => {
                let (dx, dy) = (x - center[0], y - center[1]);
                // Strongest at the center, tapering toward the edges.
                let angle = strength * 3.2 * (-falloff * (dx * dx + dy * dy)).exp() * pulse;
                let (sin, cos) = angle.sin_cos();
                [
                    cos * dx + sin * dy + center[0],
                    -sin * dx + cos * dy + center[1],
                ]
            }
            Self::Folds {
                displace,
                amplitude,
                waves,
                offset,
            } => {
                let (driver, driven) = match displace {
                    WarpAxis::Y => (x, y),
                    WarpAxis::X => (y, x),
                };
                let mut sum = 0.0;
                for wave in waves {
                    sum += wave.amplitude
                        * ((driver + offset) * wave.frequency + wave.phase + wave.shift).sin();
                }
                let driven = driven + amplitude * sum;
                match displace {
                    WarpAxis::Y => [x, driven],
                    WarpAxis::X => [driven, y],
                }
            }
            Self::Noise {
                seed_offset: [sx, sy],
                offset: [ox, oy],
                near,
                far,
                strength,
            } => {
                let nx = fbm([x * near[0] + sx + ox, y * near[1] + sy + oy]) - 0.45;
                let ny = fbm([x * near[0] + sy + 7.3 - ox, y * near[1] + sx + 7.3 - oy]) - 0.45;
                let mx = fbm([
                    x * far[0] + nx * 1.5 + sy + 3.1,
                    y * far[1] + ny * 1.5 + sx + 3.1,
                ]) - 0.45;
                let my = fbm([
                    x * far[0] + nx * 1.5 + sx + 9.7,
                    y * far[1] + ny * 1.5 + sy + 9.7,
                ]) - 0.45;
                [
                    x + (nx * 0.35 + mx * 0.18) * strength[0],
                    y + (ny * 0.35 + my * 0.18) * strength[1],
                ]
            }
            Self::Tilt { sin, cos, center } => {
                let (dx, dy) = (x - center[0], y - center[1]);
                [
                    cos * dx - sin * dy + center[0],
                    sin * dx + cos * dy + center[1],
                ]
            }
            Self::Ripple {
                amplitude,
                frequency,
                center,
                decay,
                shift,
            } => {
                let (dx, dy) = (x - center[0], y - center[1]);
                let radius = dx.hypot(dy);
                if radius < 1e-6 {
                    return [x, y];
                }
                let push = amplitude * (-decay * radius).exp() * (frequency * radius - shift).sin();
                [x + dx / radius * push, y + dy / radius * push]
            }
        }
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

    // The original fixed-function warp, kept as a bit-exact reference for the
    // classic stack.
    #[derive(Clone, Copy)]
    struct Legacy {
        distortion: f32,
        swirl: f32,
        seed_offset: [f32; 2],
        time: f32,
        noise_offset: [f32; 2],
        fold_offset: [f32; 2],
        swirl_pulse: f32,
    }

    impl Legacy {
        fn new(distortion: f32, swirl: f32, seed: u16) -> Self {
            let seed = f32::from(seed);
            Self {
                distortion,
                swirl,
                seed_offset: [fract(seed * 0.1371) * 40.0, fract(seed * 0.2113) * 40.0],
                time: 0.0,
                noise_offset: [0.0, 0.0],
                fold_offset: [0.0, 0.0],
                swirl_pulse: 1.0,
            }
        }

        fn at_time(mut self, time: f32) -> Self {
            let (sin, cos) = time.sin_cos();
            self.time = time;
            self.fold_offset = [0.0, 0.0];
            // Anchor the orbit at zero so the first frame matches the static warp.
            self.noise_offset = [(cos - 1.0) * 0.12, sin * 0.12];
            self.swirl_pulse = 1.0 + 0.2 * sin;
            self
        }

        /// Advect the distortion field while retaining periodic swirl pulsing.
        fn at_flow(mut self, phase: f32, travel: f64, motion: f32) -> Self {
            self.time = 0.0;
            let distance = (travel * f64::from(motion) * 0.08) as f32;
            self.noise_offset = [-distance, -distance * 0.35];
            self.fold_offset = self.noise_offset;
            self.swirl_pulse = 1.0 + 0.2 * phase.sin();
            self
        }

        fn apply(&self, [mut x, mut y]: [f32; 2]) -> [f32; 2] {
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

    fn stack(distortion: f32, swirl: f32, seed: u16) -> WarpStack {
        WarpStack::new(distortion, swirl, seed, &[]).unwrap()
    }

    fn positions() -> Vec<[f32; 2]> {
        let mut out = Vec::new();
        for y in -3..=3 {
            for x in -4..=4 {
                out.push([x as f32 * 0.19 + 0.013, y as f32 * 0.17 - 0.021]);
            }
        }
        out
    }

    fn clock(phase: f32, travel: f64, motion: f32, inherited: LayerMotion) -> Clock {
        Clock {
            phase,
            travel,
            motion,
            aspect: 1.7,
            inherited,
        }
    }

    #[test]
    fn classic_stack_matches_the_original_warp_bit_for_bit() {
        for (distortion, swirl, seed) in [
            (1.0, 0.45, 58),
            (1.3, -0.6, 999),
            (0.8, 0.0, 0),
            (0.0, 0.45, 58),
            (0.0, 0.0, 58),
            (1.5, 1.0, 321),
        ] {
            let new = stack(distortion, swirl, seed);
            let old = Legacy::new(distortion, swirl, seed);
            for phase in [0.0, 0.9, 5.1] {
                let cycling = new.at(&clock(phase, 0.0, 0.0, LayerMotion::Cycle));
                let looped = old.at_time(phase);
                for (travel, motion) in [(0.0, 0.0), (3.7, 0.5), (91.25, 1.5)] {
                    let drifting = new.at(&clock(phase, travel, motion, LayerMotion::Drift));
                    let flowed = old.at_flow(phase, travel, motion);
                    for position in positions() {
                        assert_eq!(cycling.apply(position), looped.apply(position));
                        assert_eq!(drifting.apply(position), flowed.apply(position));
                    }
                }
            }
        }
    }

    #[test]
    fn swirl_preserves_radius_and_reverses_direction() {
        let at = |swirl| stack(0.0, swirl, 58).at_rest(1.0).apply([0.3, 0.0]);
        let (clockwise, counterclockwise) = (at(0.5), at(-0.5));
        assert!((clockwise[0].hypot(clockwise[1]) - 0.3).abs() < 0.000001);
        assert!((clockwise[0] - counterclockwise[0]).abs() < 0.000001);
        assert!((clockwise[1] + counterclockwise[1]).abs() < 0.000001);
        assert!(clockwise[1] < 0.0 && counterclockwise[1] > 0.0);
        assert_eq!(
            stack(0.0, 0.5, 58).at_rest(1.0).apply([0.0, 0.0]),
            [0.0, 0.0]
        );
    }

    #[test]
    fn distortion_is_repeatable_and_seeded() {
        let first = stack(1.0, 0.0, 58).at_rest(1.0);
        let second = stack(1.0, 0.0, 59).at_rest(1.0);
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
    fn rejects_invalid_classic_settings() {
        for distortion in [f32::NAN, f32::INFINITY, -0.1, 1.6] {
            assert!(WarpStack::new(distortion, 0.0, 58, &[]).is_err());
        }
        for swirl in [f32::NAN, f32::INFINITY, -1.1, 1.1] {
            assert!(WarpStack::new(0.0, swirl, 58, &[]).is_err());
        }
        assert!(WarpStack::new(0.0, 0.0, 1000, &[]).is_err());
    }

    #[test]
    fn animation_starts_at_the_static_warp_and_moves_continuously() {
        let warp = stack(1.0, 0.45, 58);
        let position = [-0.2, 0.1];
        let at = |phase| {
            warp.at(&clock(phase, 0.0, 0.0, LayerMotion::Cycle))
                .apply(position)
        };
        let start = warp.at_rest(1.7).apply(position);
        assert_eq!(start, at(0.0));
        let next = at(0.001);
        assert!(
            start
                .into_iter()
                .zip(next)
                .all(|(a, b)| (a - b).abs() < 0.002)
        );
        assert_ne!(start, at(1.0));
    }

    #[test]
    fn wrapping_time_keeps_the_warp_continuous() {
        let warp = stack(1.0, 0.45, 58);
        let position = [0.3, -0.1];
        let at = |phase| {
            warp.at(&clock(phase, 0.0, 0.0, LayerMotion::Cycle))
                .apply(position)
        };
        let (start, end) = (at(0.0), at(std::f32::consts::TAU));
        assert!(
            start
                .into_iter()
                .zip(end)
                .all(|(a, b)| (a - b).abs() < 0.0001)
        );
    }

    fn with(layer: WarpLayer) -> Warp {
        WarpStack::new(0.0, 0.0, 58, &[layer]).unwrap().at_rest(1.0)
    }

    fn json(text: &str) -> WarpLayer {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn tilt_rotates_about_its_center_and_preserves_distance() {
        let tilt = with(json(r#"{"type":"tilt","angle":30}"#));
        let moved = tilt.apply([0.4, 0.0]);
        assert!((moved[0] - 0.4 * 30f32.to_radians().cos()).abs() < 0.00001);
        assert!((moved[1] - 0.4 * 30f32.to_radians().sin()).abs() < 0.00001);
        assert_eq!(tilt.apply([0.0, 0.0]), [0.0, 0.0]);
        let about = with(json(r#"{"type":"tilt","angle":-50,"center":[0.8,0.2]}"#));
        // Aspect 1 here, so the normalized center maps to (0.3, -0.3).
        let fixed = about.apply([0.3, -0.3]);
        assert!((fixed[0] - 0.3).abs() < 0.00001 && (fixed[1] + 0.3).abs() < 0.00001);
    }

    #[test]
    fn folds_displace_only_the_chosen_axis() {
        let folds = |axis: &str| {
            with(json(&format!(
                r#"{{"type":"folds","displace":"{axis}","amplitude":1,"waves":[{{"amplitude":0.1,"frequency":3,"phase":0.4}}]}}"#
            )))
        };
        let [x, y] = folds("y").apply([0.3, 0.2]);
        assert_eq!(x, 0.3);
        assert_ne!(y, 0.2);
        let [x, y] = folds("x").apply([0.3, 0.2]);
        assert_eq!(y, 0.2);
        assert_ne!(x, 0.3);
    }

    #[test]
    fn noise_weight_can_freeze_an_axis() {
        let noise = with(json(r#"{"type":"noise","strength":1,"weight":[1,0]}"#));
        let [x, y] = noise.apply([0.3, -0.2]);
        assert_eq!(y, -0.2);
        assert_ne!(x, 0.3);
        let free = with(json(r#"{"type":"noise","strength":1}"#)).apply([0.3, -0.2]);
        assert_ne!(free[1], -0.2);
        let stretched = with(json(r#"{"type":"noise","strength":1,"frequency":[1,0.3]}"#));
        assert_ne!(stretched.apply([0.3, -0.2]), free);
    }

    #[test]
    fn ripple_pushes_radially_and_leaves_the_center_alone() {
        let ripple = with(json(r#"{"type":"ripple","amplitude":0.05,"frequency":12}"#));
        assert_eq!(ripple.apply([0.0, 0.0]), [0.0, 0.0]);
        let moved = ripple.apply([0.2, 0.0]);
        assert_eq!(moved[1], 0.0);
        assert_ne!(moved[0], 0.2);
        assert!((moved[0] - 0.2).abs() <= 0.05 + 1e-6);
        let faded = with(json(
            r#"{"type":"ripple","amplitude":0.05,"frequency":12,"decay":8}"#,
        ));
        assert!((faded.apply([0.2, 0.0])[0] - 0.2).abs() < (moved[0] - 0.2).abs());
    }

    #[test]
    fn swirl_center_is_the_fixed_point() {
        let swirl = with(json(
            r#"{"type":"swirl","strength":0.8,"center":[0.7,0.3]}"#,
        ));
        let center = [0.2, -0.2];
        let fixed = swirl.apply(center);
        assert!((fixed[0] - center[0]).abs() < 0.00001 && (fixed[1] - center[1]).abs() < 0.00001);
        assert_ne!(swirl.apply([0.0, 0.0]), [0.0, 0.0]);
    }

    #[test]
    fn layers_run_in_order_after_the_classic_stack() {
        let tilt = json(r#"{"type":"tilt","angle":20}"#);
        let ripple = json(r#"{"type":"ripple","amplitude":0.05,"frequency":10}"#);
        let position = [0.35, 0.1];
        let at = |layers: &[WarpLayer]| {
            WarpStack::new(0.0, 0.0, 58, layers)
                .unwrap()
                .at_rest(1.0)
                .apply(position)
        };
        assert_ne!(
            at(&[tilt.clone(), ripple.clone()]),
            at(&[ripple.clone(), tilt.clone()])
        );
        let classic = stack(1.0, 0.0, 58).at_rest(1.0).apply(position);
        let combined = WarpStack::new(1.0, 0.0, 58, std::slice::from_ref(&tilt))
            .unwrap()
            .at_rest(1.0);
        let tilted = with(tilt).apply(classic);
        assert_eq!(combined.apply(position), tilted);
    }

    #[test]
    fn layer_animation_overrides_the_recipe_style() {
        let layer = |mode: &str| {
            json(&format!(
                r#"{{"type":"ripple","amplitude":0.05,"frequency":12,"animation":{{"mode":"{mode}"}}}}"#
            ))
        };
        let position = [0.3, 0.1];
        let at = |mode: &str, inherited, phase, travel| {
            WarpStack::new(0.0, 0.0, 58, &[layer(mode)])
                .unwrap()
                .at(&clock(phase, travel, 1.0, inherited))
                .apply(position)
        };
        // A static layer ignores both phase and travel, whatever the recipe says.
        let rest = at("static", LayerMotion::Cycle, 0.0, 0.0);
        assert_eq!(rest, at("static", LayerMotion::Drift, 2.0, 40.0));
        // A cycling layer follows phase even when the recipe drifts, and loops.
        assert_ne!(rest, at("cycle", LayerMotion::Drift, 2.0, 0.0));
        let looped = at("cycle", LayerMotion::Drift, std::f32::consts::TAU, 0.0);
        assert!((looped[0] - rest[0]).abs() < 0.0001);
        // A drifting layer follows travel, and stops when motion is zero.
        assert_ne!(rest, at("drift", LayerMotion::Cycle, 0.0, 40.0));
        let halted = WarpStack::new(0.0, 0.0, 58, &[layer("drift")])
            .unwrap()
            .at(&clock(0.0, 40.0, 0.0, LayerMotion::Cycle))
            .apply(position);
        assert_eq!(rest, halted);
    }

    #[test]
    fn layers_without_a_mode_follow_the_recipe_style() {
        let ripple = json(r#"{"type":"ripple","amplitude":0.05,"frequency":12}"#);
        let stack = WarpStack::new(0.0, 0.0, 58, &[ripple]).unwrap();
        let position = [0.3, 0.1];
        let rest = stack.at_rest(1.7).apply(position);
        let cycling = stack
            .at(&clock(2.0, 40.0, 1.0, LayerMotion::Cycle))
            .apply(position);
        let drifting = stack
            .at(&clock(2.0, 40.0, 1.0, LayerMotion::Drift))
            .apply(position);
        assert_ne!(cycling, rest);
        assert_ne!(drifting, rest);
        assert_ne!(cycling, drifting);
    }

    #[test]
    fn layer_json_uses_defaults_and_rejects_mistakes() {
        let layer = json(r#"{"type":"noise","strength":0.5}"#);
        assert_eq!(
            layer,
            WarpLayer::Noise {
                strength: 0.5,
                frequency: [1.0, 1.0],
                weight: [1.0, 1.0],
                orbit: 0.12,
                seed: None,
                animation: LayerAnimation::default(),
            }
        );
        // Defaults are omitted so serialized recipes stay short.
        assert_eq!(
            serde_json::to_string(&layer).unwrap(),
            r#"{"type":"noise","strength":0.5,"frequency":[1.0,1.0],"weight":[1.0,1.0],"orbit":0.12}"#
        );
        assert_eq!(
            serde_json::from_str::<WarpLayer>(&serde_json::to_string(&layer).unwrap()).unwrap(),
            layer
        );
        for bad in [
            r#"{"type":"noise","strength":0.5,"strenth":1}"#,
            r#"{"type":"wobble","strength":0.5}"#,
            r#"{"strength":0.5}"#,
            r#"{"type":"tilt"}"#,
            r#"{"type":"ripple","amplitude":0.1,"frequency":9,"animation":{"mode":"spin"}}"#,
            r#"{"type":"folds","displace":"z","amplitude":1,"waves":[]}"#,
        ] {
            assert!(serde_json::from_str::<WarpLayer>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rejects_invalid_layers() {
        for bad in [
            r#"{"type":"swirl","strength":1.5}"#,
            r#"{"type":"swirl","strength":0.5,"falloff":0}"#,
            r#"{"type":"swirl","strength":0.5,"pulse":2}"#,
            r#"{"type":"swirl","strength":0.5,"center":[2,0.5]}"#,
            r#"{"type":"folds","displace":"y","amplitude":1,"waves":[]}"#,
            r#"{"type":"folds","displace":"y","amplitude":2,"waves":[{"amplitude":0.1,"frequency":2}]}"#,
            r#"{"type":"folds","displace":"y","amplitude":1,"waves":[{"amplitude":0.9,"frequency":2}]}"#,
            r#"{"type":"folds","displace":"y","amplitude":1,"waves":[{"amplitude":0.1,"frequency":99}]}"#,
            r#"{"type":"folds","displace":"y","amplitude":1,"waves":[{"amplitude":0.1,"frequency":2,"phase":500}]}"#,
            r#"{"type":"folds","displace":"y","amplitude":1,"seed":1000,"waves":[{"amplitude":0.1,"frequency":2}]}"#,
            r#"{"type":"folds","displace":"y","amplitude":1,"waves":[{"amplitude":0.1,"frequency":2},{"amplitude":0.1,"frequency":2},{"amplitude":0.1,"frequency":2},{"amplitude":0.1,"frequency":2},{"amplitude":0.1,"frequency":2}]}"#,
            r#"{"type":"noise","strength":2}"#,
            r#"{"type":"noise","strength":1,"frequency":[0,1]}"#,
            r#"{"type":"noise","strength":1,"weight":[3,1]}"#,
            r#"{"type":"noise","strength":1,"orbit":2}"#,
            r#"{"type":"tilt","angle":120}"#,
            r#"{"type":"ripple","amplitude":0.9,"frequency":9}"#,
            r#"{"type":"ripple","amplitude":0.1,"frequency":0.1}"#,
            r#"{"type":"ripple","amplitude":0.1,"frequency":9,"decay":-1}"#,
            r#"{"type":"ripple","amplitude":0.1,"frequency":9,"animation":{"speed":20}}"#,
            r#"{"type":"ripple","amplitude":0.1,"frequency":9,"animation":{"direction":[9,0]}}"#,
        ] {
            let layer = serde_json::from_str::<WarpLayer>(bad).unwrap();
            assert!(WarpStack::new(0.0, 0.0, 58, &[layer]).is_err(), "{bad}");
        }
        let tilt = json(r#"{"type":"tilt","angle":1}"#);
        // 16 layers fit with no classic ones; a 17th does not.
        assert!(WarpStack::new(0.0, 0.0, 58, &vec![tilt.clone(); MAX_LAYERS]).is_ok());
        assert!(WarpStack::new(0.0, 0.0, 58, &vec![tilt.clone(); MAX_LAYERS + 1]).is_err());
        // The classic stack counts toward the limit: distortion adds three layers.
        assert!(WarpStack::new(1.0, 0.0, 58, &vec![tilt.clone(); MAX_LAYERS - 3]).is_ok());
        assert!(WarpStack::new(1.0, 0.0, 58, &vec![tilt; MAX_LAYERS - 2]).is_err());
    }

    #[test]
    fn nonfinite_layer_values_are_rejected() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for layer in [
                WarpLayer::Swirl {
                    strength: value,
                    falloff: 1.4,
                    center: [0.5, 0.5],
                    pulse: 0.2,
                },
                WarpLayer::Tilt {
                    angle: value,
                    center: [0.5, 0.5],
                },
                WarpLayer::Ripple {
                    amplitude: 0.1,
                    frequency: value,
                    center: [0.5, 0.5],
                    decay: 0.0,
                    animation: LayerAnimation::default(),
                },
            ] {
                assert!(WarpStack::new(0.0, 0.0, 58, &[layer]).is_err());
            }
        }
    }
}
