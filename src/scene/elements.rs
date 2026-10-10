//! Elements: small generators that emit points.
//!
//! Adding an element means one [`Element`] variant, its arm in `validate`, its
//! arm in `emit`, and a name in `kind`.
use std::{collections::BTreeMap, f32::consts::TAU};

use serde::{Deserialize, Serialize};

use super::{ColorSpec, Ctx, Shape, check};

/// How the original eighteen-point layouts assign palette slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GridRule {
    Sky,
    Horizon,
    Glow,
    Scatter,
    Superscatter,
    Wash,
}

/// One entry of a `lights` element's color mix.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LightRole {
    pub role: String,
    /// Relative likelihood of this role.
    #[serde(default = "one")]
    pub weight: f32,
}

/// Light spilling into the space around each light: concentric points that
/// grow wider, weaker and closer to the haze color, so the glow fades smoothly
/// into whatever is behind it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bloom {
    /// Number of concentric points around each light.
    #[serde(default = "bloom_rings")]
    pub rings: u32,
    /// Radius growth per ring, as a multiple of the previous ring.
    #[serde(default = "bloom_spread")]
    pub spread: f32,
    /// Pull of the innermost ring. Keep it well below the backdrop's, or the
    /// glow takes over instead of tinting.
    #[serde(default = "bloom_strength")]
    pub strength: f32,
    /// Each ring's strength as a fraction of the one inside it.
    #[serde(default = "bloom_fade")]
    pub fade: f32,
}

/// How `strokes` are placed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Arrange {
    #[default]
    Row,
    Scatter,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Element {
    /// The classic eighteen jittered grid points, colored from the roles
    /// `main`, `second` and `accent` by one of the original layout rules.
    Grid { rule: GridRule },
    /// A single point, for anything the other elements don't cover.
    Point {
        position: [f32; 2],
        role: String,
        #[serde(default)]
        shade: f32,
        #[serde(default)]
        depth: f32,
        #[serde(default = "one")]
        radius: f32,
        #[serde(default = "one")]
        stretch: f32,
        #[serde(default)]
        angle: f32,
        #[serde(default = "one")]
        strength: f32,
    },
    /// Wide horizontal bands blending from one role at `y[0]` to another at
    /// `y[1]`: a sky, a backdrop, a floor.
    Gradient {
        from: String,
        to: String,
        #[serde(default = "full_range")]
        y: [f32; 2],
        #[serde(default = "eight_rows")]
        rows: u32,
        /// Shade at the first and last band.
        #[serde(default)]
        shade: [f32; 2],
        #[serde(default)]
        depth: [f32; 2],
        /// Band thickness and softness. Keep it no larger than the radius of
        /// the shapes in front, or the backdrop wins and they fade out.
        #[serde(default = "backdrop_radius")]
        radius: f32,
        /// How far each band reaches sideways, as a multiple of `radius`.
        #[serde(default = "six")]
        stretch: f32,
        /// Pull of each band. Below 1 keeps a backdrop from showing through
        /// the shapes in front of it.
        #[serde(default = "half")]
        strength: f32,
        #[serde(default = "half")]
        x: f32,
    },
    /// A wavy horizon: a row of wide points just under a crest line, each
    /// leaning with the slope. Stack several with growing `depth` for hills.
    Ridge {
        role: String,
        /// Height of the crest line, before waves.
        y: f32,
        #[serde(default = "ridge_amplitude")]
        amplitude: f32,
        /// Waves across the image.
        #[serde(default = "ridge_frequency")]
        frequency: f32,
        /// How much finer waves roughen the crest (0 to 1).
        #[serde(default = "ridge_roughness")]
        roughness: f32,
        #[serde(default = "ridge_extent")]
        x: [f32; 2],
        #[serde(default = "ridge_spacing")]
        spacing: f32,
        /// How far below the crest the points sit.
        #[serde(default = "ridge_inset")]
        inset: f32,
        #[serde(default = "ridge_radius")]
        radius: f32,
        #[serde(default = "three")]
        stretch: f32,
        #[serde(default)]
        shade: f32,
        #[serde(default = "tenth")]
        shade_jitter: f32,
        /// 0 is near and 1 is as faint as the scene's haze allows.
        #[serde(default)]
        depth: f32,
        /// Tilt each point to follow the crest.
        #[serde(default = "yes")]
        follow: bool,
    },
    /// Elongated soft marks: trunks, poles, buildings, beams of light. Each is
    /// a stretched point; `angle` leans it away from upright.
    Strokes {
        role: String,
        count: u32,
        /// `row` spaces them evenly along `x`, each at a random height in `y`;
        /// `scatter` puts them at random anywhere in the box.
        #[serde(default)]
        arrange: Arrange,
        #[serde(default = "unit_range")]
        x: [f32; 2],
        #[serde(default = "stroke_y")]
        y: [f32; 2],
        /// How far each reaches along its length, as a multiple of `width`.
        #[serde(default = "stroke_length")]
        length: [f32; 2],
        /// Thickness and edge softness of the nearest strokes.
        #[serde(default = "quarter")]
        width: f32,
        /// How much the farthest strokes narrow (0 to 1).
        #[serde(default = "half")]
        taper: f32,
        /// Each stroke draws its depth from this range.
        #[serde(default)]
        depth: [f32; 2],
        /// Lean from upright, in degrees clockwise: 0 is vertical, 90 is
        /// horizontal, and positive leans right.
        #[serde(default)]
        angle: f32,
        /// Largest random change to `angle`, in degrees.
        #[serde(default = "two")]
        angle_jitter: f32,
        /// Row position wobble, as a fraction of the spacing.
        #[serde(default = "stroke_jitter")]
        jitter: f32,
        #[serde(default)]
        shade: f32,
        #[serde(default = "tenth")]
        shade_jitter: f32,
        #[serde(default = "one")]
        strength: f32,
    },
    /// Soft glowing patches, spread apart: windows, stars, lanterns. Keep them
    /// few and soft; many small saturated dots read as confetti, not light.
    Lights {
        count: u32,
        roles: Vec<LightRole>,
        #[serde(default = "unit_range")]
        x: [f32; 2],
        #[serde(default = "lights_y")]
        y: [f32; 2],
        #[serde(default = "lights_radius")]
        radius: [f32; 2],
        #[serde(default = "lights_strength")]
        strength: f32,
        #[serde(default = "lights_shade")]
        shade: [f32; 2],
        #[serde(default)]
        depth: f32,
        /// A smaller, brighter point inside each light, as a multiple of its
        /// radius (0 for none), so it glows toward the center instead of being
        /// one flat color.
        #[serde(default)]
        core: f32,
        /// How much lighter the core is than its light.
        #[serde(default = "core_shade")]
        core_shade: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bloom: Option<Bloom>,
    },
}

fn one() -> f32 {
    1.0
}
fn two() -> f32 {
    2.0
}
fn three() -> f32 {
    3.0
}
fn six() -> f32 {
    6.0
}
fn half() -> f32 {
    0.5
}
fn quarter() -> f32 {
    0.25
}
fn tenth() -> f32 {
    0.1
}
fn yes() -> bool {
    true
}
fn eight_rows() -> u32 {
    8
}
fn backdrop_radius() -> f32 {
    0.4
}
fn full_range() -> [f32; 2] {
    [0.0, 1.0]
}
fn unit_range() -> [f32; 2] {
    [0.0, 1.0]
}
fn ridge_amplitude() -> f32 {
    0.06
}
fn ridge_frequency() -> f32 {
    1.5
}
fn ridge_roughness() -> f32 {
    0.4
}
fn ridge_extent() -> [f32; 2] {
    [-0.15, 1.15]
}
fn ridge_spacing() -> f32 {
    0.1
}
fn ridge_inset() -> f32 {
    0.06
}
fn ridge_radius() -> f32 {
    0.5
}
fn stroke_y() -> [f32; 2] {
    [0.6, 0.6]
}
fn stroke_length() -> [f32; 2] {
    [4.0, 6.0]
}
fn stroke_jitter() -> f32 {
    0.6
}
fn lights_y() -> [f32; 2] {
    [0.6, 1.0]
}
fn lights_radius() -> [f32; 2] {
    [0.08, 0.14]
}
fn lights_strength() -> f32 {
    2.0
}
fn lights_shade() -> [f32; 2] {
    [0.2, 0.8]
}
fn core_shade() -> f32 {
    0.3
}
fn bloom_rings() -> u32 {
    3
}
fn bloom_spread() -> f32 {
    1.9
}
fn bloom_strength() -> f32 {
    0.3
}
fn bloom_fade() -> f32 {
    0.5
}

fn role(palette: &BTreeMap<String, ColorSpec>, name: &str) -> Result<(), String> {
    if palette.contains_key(name) {
        Ok(())
    } else {
        Err(format!("role '{name}' is not in the palette"))
    }
}

fn ordered(what: &str, range: [f32; 2], low: f32, high: f32) -> Result<(), String> {
    check(what, range[0], low, high)?;
    check(what, range[1], low, high)?;
    if range[0] > range[1] {
        return Err(format!("{what} must list the smaller value first"));
    }
    Ok(())
}

/// Shared checks for the attributes every point-like element exposes.
fn shape(radius: f32, stretch: f32, strength: f32) -> Result<(), String> {
    check("radius", radius, 0.05, 10.0)?;
    check("stretch", stretch, 0.1, 10.0)?;
    check("strength", strength, 0.05, 20.0)
}

impl Element {
    pub(super) fn kind(&self) -> &'static str {
        match self {
            Self::Grid { .. } => "grid",
            Self::Point { .. } => "point",
            Self::Gradient { .. } => "gradient",
            Self::Ridge { .. } => "ridge",
            Self::Strokes { .. } => "strokes",
            Self::Lights { .. } => "lights",
        }
    }

    pub(super) fn validate(&self, palette: &BTreeMap<String, ColorSpec>) -> Result<(), String> {
        match self {
            Self::Grid { .. } => {
                for name in ["main", "second", "accent"] {
                    role(palette, name).map_err(|_| {
                        "a grid needs the palette roles main, second and accent".to_string()
                    })?;
                }
            }
            Self::Point {
                position,
                role: name,
                shade,
                depth,
                radius,
                stretch,
                angle,
                strength,
            } => {
                role(palette, name)?;
                for value in position {
                    check("position", *value, -0.5, 1.5)?;
                }
                check("shade", *shade, -2.0, 2.0)?;
                check("depth", *depth, 0.0, 1.0)?;
                shape(*radius, *stretch, *strength)?;
                check("angle", *angle, -180.0, 180.0)?;
            }
            Self::Gradient {
                from,
                to,
                y,
                rows,
                shade,
                depth,
                radius,
                stretch,
                strength,
                x,
            } => {
                role(palette, from)?;
                role(palette, to)?;
                for value in y {
                    check("y", *value, -0.5, 1.5)?;
                }
                if !(2..=32).contains(rows) {
                    return Err("rows must be between 2 and 32".into());
                }
                for value in shade {
                    check("shade", *value, -2.0, 2.0)?;
                }
                for value in depth {
                    check("depth", *value, 0.0, 1.0)?;
                }
                shape(*radius, *stretch, *strength)?;
                check("x", *x, -0.5, 1.5)?;
            }
            Self::Ridge {
                role: name,
                y,
                amplitude,
                frequency,
                roughness,
                x,
                spacing,
                inset,
                radius,
                stretch,
                shade,
                shade_jitter,
                depth,
                ..
            } => {
                role(palette, name)?;
                check("y", *y, -0.5, 1.5)?;
                check("amplitude", *amplitude, 0.0, 0.5)?;
                check("frequency", *frequency, 0.2, 8.0)?;
                check("roughness", *roughness, 0.0, 1.0)?;
                ordered("x", *x, -0.5, 1.5)?;
                check("spacing", *spacing, 0.02, 0.5)?;
                check("inset", *inset, 0.0, 0.5)?;
                shape(*radius, *stretch, 1.0)?;
                check("shade", *shade, -2.0, 2.0)?;
                check("shade_jitter", *shade_jitter, 0.0, 1.0)?;
                check("depth", *depth, 0.0, 1.0)?;
            }
            Self::Strokes {
                role: name,
                count,
                x,
                y,
                length,
                width,
                taper,
                depth,
                angle,
                angle_jitter,
                jitter,
                shade,
                shade_jitter,
                strength,
                ..
            } => {
                role(palette, name)?;
                if !(1..=64).contains(count) {
                    return Err("count must be between 1 and 64".into());
                }
                ordered("x", *x, -0.5, 1.5)?;
                ordered("y", *y, -0.5, 1.5)?;
                ordered("length", *length, 1.0, 10.0)?;
                check("width", *width, 0.05, 2.0)?;
                check("taper", *taper, 0.0, 1.0)?;
                ordered("depth", *depth, 0.0, 1.0)?;
                check("angle", *angle, -90.0, 90.0)?;
                check("angle_jitter", *angle_jitter, 0.0, 45.0)?;
                check("jitter", *jitter, 0.0, 1.0)?;
                check("shade", *shade, -2.0, 2.0)?;
                check("shade_jitter", *shade_jitter, 0.0, 1.0)?;
                check("strength", *strength, 0.05, 20.0)?;
            }
            Self::Lights {
                count,
                roles,
                x,
                y,
                radius,
                strength,
                shade,
                depth,
                core,
                core_shade,
                bloom,
            } => {
                if !(1..=200).contains(count) {
                    return Err("count must be between 1 and 200".into());
                }
                if !(1..=8).contains(&roles.len()) {
                    return Err("lights need between 1 and 8 roles".into());
                }
                for light in roles {
                    role(palette, &light.role)?;
                    check("weight", light.weight, 0.01, 100.0)?;
                }
                ordered("x", *x, -0.5, 1.5)?;
                ordered("y", *y, -0.5, 1.5)?;
                ordered("radius", *radius, 0.05, 2.0)?;
                check("strength", *strength, 0.05, 20.0)?;
                ordered("shade", *shade, -2.0, 2.0)?;
                check("depth", *depth, 0.0, 1.0)?;
                if *core != 0.0 {
                    check("core", *core, 0.2, 0.9)?;
                }
                check("core_shade", *core_shade, -2.0, 2.0)?;
                if let Some(bloom) = bloom {
                    if !(1..=4).contains(&bloom.rings) {
                        return Err("bloom rings must be between 1 and 4".into());
                    }
                    check("bloom spread", bloom.spread, 1.2, 3.0)?;
                    check("bloom strength", bloom.strength, 0.02, 2.0)?;
                    check("bloom fade", bloom.fade, 0.1, 1.0)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn emit(&self, ctx: &mut Ctx) {
        match self {
            Self::Grid { rule } => emit_grid(*rule, ctx),
            Self::Point {
                position,
                role,
                shade,
                depth,
                radius,
                stretch,
                angle,
                strength,
            } => ctx.emit(
                *position,
                role,
                *shade,
                *depth,
                Shape {
                    radius: *radius,
                    stretch: *stretch,
                    angle: *angle,
                    strength: *strength,
                },
            ),
            Self::Gradient {
                from,
                to,
                y,
                rows,
                shade,
                depth,
                radius,
                stretch,
                strength,
                x,
            } => {
                let rows = *rows as usize;
                let lerp = |pair: [f32; 2], t: f32| pair[0] + (pair[1] - pair[0]) * t;
                for row in 0..rows {
                    let t = row as f32 / (rows - 1) as f32;
                    let (a, b) = (
                        ctx.color(from, lerp(*shade, t)),
                        ctx.color(to, lerp(*shade, t)),
                    );
                    let lab = [0, 1, 2].map(|channel| a[channel] + (b[channel] - a[channel]) * t);
                    ctx.emit_lab(
                        [*x, lerp(*y, t)],
                        lab,
                        lerp(*depth, t),
                        Shape {
                            radius: *radius,
                            stretch: *stretch,
                            strength: *strength,
                            ..Shape::default()
                        },
                    );
                }
            }
            Self::Ridge {
                role,
                y,
                amplitude,
                frequency,
                roughness,
                x,
                spacing,
                inset,
                radius,
                stretch,
                shade,
                shade_jitter,
                depth,
                follow,
            } => {
                let phases = [ctx.random() * TAU, ctx.random() * TAU, ctx.random() * TAU];
                // Three sines at unrelated frequencies make a crest that does
                // not repeat visibly; roughness sets how loud the fine ones are.
                let crest = |at: f32| {
                    let wave = (TAU * frequency * at + phases[0]).sin()
                        + 0.5 * roughness * (TAU * frequency * 2.3 * at + phases[1]).sin()
                        + 0.25 * roughness * (TAU * frequency * 5.1 * at + phases[2]).sin();
                    y + amplitude * wave / (1.0 + 0.75 * roughness)
                };
                for index in 0..ridge_count(*x, *spacing) {
                    let at = x[0] + index as f32 * spacing + (ctx.random() - 0.5) * 0.3 * spacing;
                    let angle = if *follow {
                        let slope = (crest(at + 0.01) - crest(at - 0.01)) / 0.02;
                        slope.atan().to_degrees().clamp(-60.0, 60.0)
                    } else {
                        0.0
                    };
                    let shade = shade + (ctx.random() - 0.5) * 2.0 * shade_jitter;
                    ctx.emit(
                        [at, crest(at) + inset],
                        role,
                        shade,
                        *depth,
                        Shape {
                            radius: *radius,
                            stretch: *stretch,
                            angle,
                            strength: 1.0,
                        },
                    );
                }
            }
            Self::Strokes {
                role,
                count,
                arrange,
                x,
                y,
                length,
                width,
                taper,
                depth,
                angle,
                angle_jitter,
                jitter,
                shade,
                shade_jitter,
                strength,
            } => {
                let slot = (x[1] - x[0]) / *count as f32;
                for index in 0..*count {
                    let (at, top) = match arrange {
                        Arrange::Row => (
                            x[0] + (index as f32 + 0.5) * slot
                                + (ctx.random() - 0.5) * jitter * slot,
                            y[0] + (y[1] - y[0]) * ctx.random(),
                        ),
                        Arrange::Scatter => (
                            x[0] + (x[1] - x[0]) * ctx.random(),
                            y[0] + (y[1] - y[0]) * ctx.random(),
                        ),
                    };
                    // 0 for the nearest stroke and 1 for the farthest.
                    let far = ctx.random();
                    let reach = length[0] + (length[1] - length[0]) * ctx.random();
                    let lean = angle + (ctx.random() - 0.5) * 2.0 * angle_jitter;
                    let shade = shade + (ctx.random() - 0.5) * 2.0 * shade_jitter;
                    ctx.emit(
                        [at, top],
                        role,
                        shade,
                        depth[0] + (depth[1] - depth[0]) * far,
                        Shape {
                            // Taper must not push a stroke below the smallest point radius.
                            radius: (width * (1.0 - taper * far)).max(0.05),
                            // A tall region, leaned clockwise by `lean`.
                            stretch: 1.0 / reach,
                            angle: lean,
                            strength: *strength,
                        },
                    );
                }
            }
            Self::Lights {
                count,
                roles,
                x,
                y,
                radius,
                strength,
                shade,
                depth,
                core,
                core_shade,
                bloom,
            } => {
                let positions = spread_points(*count as usize, *x, *y, 0.01, &mut || ctx.random());
                let total: f32 = roles.iter().map(|light| light.weight).sum();
                for position in positions {
                    let mut pick = ctx.random() * total;
                    let light = roles
                        .iter()
                        .find(|light| {
                            pick -= light.weight;
                            pick < 0.0
                        })
                        .unwrap_or(&roles[roles.len() - 1]);
                    let size = radius[0] + (radius[1] - radius[0]) * ctx.random();
                    let tone = shade[0] + (shade[1] - shade[0]) * ctx.random();
                    ctx.emit(
                        position,
                        &light.role,
                        tone,
                        *depth,
                        Shape {
                            radius: size,
                            strength: *strength,
                            ..Shape::default()
                        },
                    );
                    if *core > 0.0 {
                        ctx.emit(
                            position,
                            &light.role,
                            tone + core_shade,
                            *depth,
                            Shape {
                                radius: (size * core).max(0.05),
                                strength: *strength,
                                ..Shape::default()
                            },
                        );
                    }
                    if let Some(bloom) = bloom {
                        for ring in 1..=bloom.rings {
                            let outward = ring as f32 / bloom.rings as f32;
                            ctx.emit(
                                position,
                                &light.role,
                                tone - 0.1 * ring as f32,
                                // Outer rings blend into the haze, so the glow
                                // dissolves into the surroundings.
                                depth + (1.0 - depth) * outward,
                                Shape {
                                    radius: (size * bloom.spread.powi(ring as i32)).min(10.0),
                                    strength: bloom.strength * bloom.fade.powi(ring as i32 - 1),
                                    ..Shape::default()
                                },
                            );
                        }
                    }
                }
            }
        }
    }
}

fn ridge_count(x: [f32; 2], spacing: f32) -> usize {
    ((x[1] - x[0]) / spacing).ceil() as usize + 1
}

/// Best-candidate sampling spreads points without imposing rows or columns:
/// each point is the farthest of 24 random candidates from those already
/// placed. `jitter` nudges the result so placements don't look computed.
pub(super) fn spread_points(
    count: usize,
    x: [f32; 2],
    y: [f32; 2],
    jitter: f32,
    random: &mut impl FnMut() -> f32,
) -> Vec<[f32; 2]> {
    let mut positions: Vec<[f32; 2]> = Vec::with_capacity(count);
    let candidate = |random: &mut dyn FnMut() -> f32| {
        [
            x[0] + random() * (x[1] - x[0]),
            y[0] + random() * (y[1] - y[0]),
        ]
    };
    for _ in 0..count {
        let mut best = candidate(random);
        let mut best_distance = -1.0;
        for _ in 0..24 {
            let next = candidate(random);
            let distance = positions
                .iter()
                .map(|point| (next[0] - point[0]).powi(2) + (next[1] - point[1]).powi(2))
                .fold(f32::INFINITY, f32::min);
            if distance > best_distance {
                best = next;
                best_distance = distance;
            }
        }
        positions.push([
            (best[0] + (random() - 0.5) * jitter).clamp(x[0], x[1]),
            (best[1] + (random() - 0.5) * jitter).clamp(y[0], y[1]),
        ]);
    }
    positions
}

/// The original layouts. The order of random draws is part of their output, so
/// it is kept exactly: phase, then positions, then per point a jitter pair,
/// any rule draws, and the color's three draws.
fn emit_grid(rule: GridRule, ctx: &mut Ctx) {
    const SLOTS: [&str; 3] = ["main", "second", "accent"];
    let scatter_phase = ctx.random() * TAU;
    let scattered = if rule == GridRule::Superscatter {
        spread_points(18, [0.0, 1.0], [0.0, 1.0], 0.02, &mut || ctx.random())
    } else {
        Vec::new()
    };
    let mut placed: Vec<([f32; 2], usize)> = Vec::new();
    // Eighteen centers cover edges as well as the interior.
    for row in 0..6 {
        for column in 0..3 {
            let [x, y] = if rule == GridRule::Superscatter {
                scattered[row * 3 + column]
            } else {
                [
                    (column as f32 / 2.0 + (ctx.random() - 0.5) * 0.08).clamp(0.0, 1.0),
                    (row as f32 / 5.0 + (ctx.random() - 0.5) * 0.06).clamp(0.0, 1.0),
                ]
            };
            let distance = (x - 0.5).hypot((y - 0.5) * 1.2);
            let (index, shade) = match rule {
                GridRule::Sky => (
                    if y < 0.45 {
                        0
                    } else if y < 0.75 {
                        1
                    } else {
                        2
                    },
                    (y - 0.5) * 2.0,
                ),
                GridRule::Horizon => (
                    if (y - 0.58).abs() < 0.14 {
                        2
                    } else if y < 0.58 {
                        0
                    } else {
                        1
                    },
                    if (y - 0.58).abs() < 0.14 { 1.0 } else { -0.4 },
                ),
                GridRule::Glow => (
                    if distance < 0.25 {
                        2
                    } else if distance < 0.45 {
                        1
                    } else {
                        0
                    },
                    1.0 - 3.2 * distance,
                ),
                GridRule::Scatter => {
                    let value = 0.5
                        + 0.25 * (5.3 * x + scatter_phase).sin()
                        + 0.25 * (4.1 * y + 1.7 * x).sin();
                    (
                        if value < 0.5 {
                            0
                        } else if value < 0.82 {
                            1
                        } else {
                            2
                        },
                        (value - 0.5) * 1.6,
                    )
                }
                GridRule::Superscatter => {
                    let mut counts = [0; 3];
                    let mut penalties = [0.0_f32; 3];
                    for &([px, py], family) in &placed {
                        counts[family] += 1;
                        let distance = (x - px).hypot(y - py);
                        penalties[family] += 1.0 / distance.max(0.03).powi(2);
                    }
                    // Primary / secondary / accent quotas retain palette
                    // hierarchy. Normalize neighbor penalties so the primary
                    // can recur more often.
                    let targets = [9, 6, 3];
                    for family in 0..3 {
                        penalties[family] /= targets[family] as f32;
                    }
                    let start = (ctx.random() * 3.0) as usize;
                    let index = (0..3)
                        .map(|offset| (start + offset) % 3)
                        .filter(|&family| counts[family] < targets[family])
                        .min_by(|&a, &b| penalties[a].total_cmp(&penalties[b]))
                        .unwrap();
                    (index, (ctx.random() - 0.5) * 1.2)
                }
                GridRule::Wash => (
                    if row == 3 && column == 1 {
                        1
                    } else if row == 5 && column == 0 {
                        2
                    } else {
                        0
                    },
                    (ctx.random() - 0.5) * 0.4,
                ),
            };
            placed.push(([x, y], index));
            ctx.emit([x, y], SLOTS[index], shade, 0.0, Shape::default());
        }
    }
}
