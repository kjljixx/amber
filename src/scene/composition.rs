//! Compositions: a scene described by what is in it, not by elements.
//!
//! A composition answers a fixed set of questions about generic parts of a
//! picture (a backdrop, land, upright features, rays, lights, a glow, haze),
//! each as a color or a small bounded number. That is exactly what Jev can
//! answer, so no scene is named or special-cased: a forest, a city and a
//! lighthouse are different answers to the same questions. [`Composition::to_scene`]
//! turns the answers into elements; everything below it is unchanged.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::elements::{Arrange, Bloom};
use super::{ColorSpec, Element, GridRule, Haze, LightRole, Scene, Style, check};
use crate::MotionStyle;

/// The five colors a composition uses. Parts take darker or lighter shades of
/// these, so they only need to name a hue.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    /// The top of the sky.
    pub sky: ColorSpec,
    /// The sky at the horizon, and the haze that distant things fade into.
    pub horizon: ColorSpec,
    /// Land, water or floor.
    pub ground: ColorSpec,
    /// Upright features such as trunks, towers or poles.
    pub feature: ColorSpec,
    /// Anything that gives off light: lights, rays, the glow.
    pub light: ColorSpec,
}

/// Where lights sit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LightsAt {
    /// In the sky, like stars.
    Sky,
    /// Along the horizon, like a skyline or a shore.
    #[default]
    Horizon,
    /// Below the horizon, like reflections or lanterns on the ground.
    Ground,
    /// Anywhere.
    Everywhere,
}

/// Every level is a number from 0 to its maximum; in between values blend the
/// neighboring settings, so answers stay smooth. Below 0.5, a part is absent.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    pub colors: Colors,
    /// 0 high in the picture to 4 very low. Splits sky from ground.
    #[serde(default = "two")]
    pub horizon_height: f32,
    /// 0 none, 1 flat (water, plains), 2 gentle hills, 3 rolling hills,
    /// 4 jagged mountains.
    #[serde(default)]
    pub land: f32,
    /// How many upright features: 0 none, 1 a few, 2 several, 3 many, 4 a
    /// dense crowd.
    #[serde(default)]
    pub features: f32,
    /// 0 hair-thin to 3 massive.
    #[serde(default = "one")]
    pub feature_width: f32,
    /// Slanted beams of light: 0 none, 1 a few, 2 several, 3 many.
    #[serde(default)]
    pub rays: f32,
    /// How many small lights: 0 none, 1 a few, 2 some, 3 many, 4 a great many.
    #[serde(default)]
    pub lights: f32,
    /// 0 pinpoints to 3 large orbs.
    #[serde(default = "one")]
    pub light_size: f32,
    /// How far each light spills into its surroundings: 0 not at all to 3
    /// radiant.
    #[serde(default)]
    pub light_glow: f32,
    #[serde(default)]
    pub lights_at: LightsAt,
    /// One big source of light, like a sun or moon: 0 none, 1 faint, 2 bright,
    /// 3 blazing.
    #[serde(default)]
    pub glow: f32,
    /// 0 sitting on the horizon to 3 high overhead.
    #[serde(default)]
    pub glow_height: f32,
    /// Wisps of cloud or mist across the sky: 0 none, 1 a few, 2 several,
    /// 3 a sky full of them.
    #[serde(default)]
    pub clouds: f32,
    /// Distance haze: 0 crystal clear, 1 slight, 2 hazy, 3 thick fog.
    #[serde(default)]
    pub atmosphere: f32,
    /// How much this is a place you could see rather than a feeling or idea:
    /// 0 purely a feeling, 1 mostly a feeling, 2 partly a place, 3 a concrete
    /// place or scene. Below 1.5 the picture is not a scene at all but broad
    /// soft regions of color arranged by `layout`; above it, scenery fades in
    /// with the score.
    #[serde(default = "three")]
    pub concrete: f32,
    /// How the colors of a feeling are arranged when `concrete` is low: the
    /// original layouts `sky`, `horizon`, `glow`, `scatter`, `superscatter` or
    /// `wash`. The sky, horizon and ground colors become their main, second
    /// and accent colors.
    #[serde(default = "default_layout")]
    pub layout: GridRule,
    #[serde(default)]
    pub style: Style,
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
fn default_layout() -> GridRule {
    GridRule::Scatter
}

/// Piecewise-linear lookup: a score of 2.5 is halfway between `table[2]` and
/// `table[3]`.
fn blend(table: &[f32], score: f32) -> f32 {
    let score = score.clamp(0.0, (table.len() - 1) as f32);
    let index = (score.floor() as usize).min(table.len() - 2);
    table[index] + (table[index + 1] - table[index]) * (score - index as f32)
}

/// Below this `concrete` score a description is a feeling, not a scene.
const ABSTRACT_BELOW: f32 = 1.5;
const PRESENT: f32 = 0.5;
/// A lone slanted streak reads as a mistake, so beams need a clear yes.
const RAYS_PRESENT: f32 = 1.5;
/// Jev scores "no lights" prompts up to about 1.2, so lights need a clear yes.
const LIGHTS_PRESENT: f32 = 1.5;

/// Largest value of each level, which is also how many settings Jev scores
/// it against (see `generation`).
impl Composition {
    pub const HORIZON_HEIGHT_MAX: f32 = 4.0;
    pub const LAND_MAX: f32 = 4.0;
    pub const FEATURES_MAX: f32 = 4.0;
    pub const FEATURE_WIDTH_MAX: f32 = 3.0;
    pub const RAYS_MAX: f32 = 3.0;
    pub const LIGHTS_MAX: f32 = 4.0;
    pub const LIGHT_SIZE_MAX: f32 = 3.0;
    pub const LIGHT_GLOW_MAX: f32 = 3.0;
    pub const GLOW_MAX: f32 = 3.0;
    pub const GLOW_HEIGHT_MAX: f32 = 3.0;
    pub const CLOUDS_MAX: f32 = 3.0;
    pub const ATMOSPHERE_MAX: f32 = 3.0;
    pub const CONCRETE_MAX: f32 = 3.0;

    fn validate(&self) -> Result<(), String> {
        check(
            "horizon_height",
            self.horizon_height,
            0.0,
            Self::HORIZON_HEIGHT_MAX,
        )?;
        check("land", self.land, 0.0, Self::LAND_MAX)?;
        check("features", self.features, 0.0, Self::FEATURES_MAX)?;
        check(
            "feature_width",
            self.feature_width,
            0.0,
            Self::FEATURE_WIDTH_MAX,
        )?;
        check("rays", self.rays, 0.0, Self::RAYS_MAX)?;
        check("lights", self.lights, 0.0, Self::LIGHTS_MAX)?;
        check("light_size", self.light_size, 0.0, Self::LIGHT_SIZE_MAX)?;
        check("light_glow", self.light_glow, 0.0, Self::LIGHT_GLOW_MAX)?;
        check("glow", self.glow, 0.0, Self::GLOW_MAX)?;
        check("glow_height", self.glow_height, 0.0, Self::GLOW_HEIGHT_MAX)?;
        check("clouds", self.clouds, 0.0, Self::CLOUDS_MAX)?;
        check("atmosphere", self.atmosphere, 0.0, Self::ATMOSPHERE_MAX)?;
        check("concrete", self.concrete, 0.0, Self::CONCRETE_MAX)
    }

    /// Reads, parses, and validates a composition file.
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        let data = std::fs::read(path).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("could not read composition '{}': {error}", path.display()),
            )
        })?;
        let composition: Self = serde_json::from_slice(&data).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid JSON composition '{}': {error}", path.display()),
            )
        })?;
        composition.to_scene(1).map_err(|message| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid composition '{}': {message}", path.display()),
            )
        })?;
        Ok(composition)
    }

    /// The scene these answers describe. `variation` changes the random
    /// placement without changing the design.
    pub fn to_scene(&self, variation: u32) -> Result<Scene, String> {
        self.validate()?;
        if self.concrete < ABSTRACT_BELOW {
            return Ok(self.color_field(variation));
        }
        // Scenery fades in with how concrete the description is.
        let scenery = blend(&[0.0, 0.3, 0.85, 1.0], self.concrete);
        Self {
            land: self.land * scenery,
            features: self.features * scenery,
            rays: self.rays * scenery,
            lights: self.lights * scenery,
            glow: self.glow * scenery,
            ..self.clone()
        }
        .build(variation)
    }

    /// A feeling, not a place: the original broad soft regions of color.
    fn color_field(&self, variation: u32) -> Scene {
        let colors = &self.colors;
        let palette = [
            ("main", &colors.sky),
            ("second", &colors.horizon),
            ("accent", &colors.ground),
        ]
        .into_iter()
        .map(|(role, color)| (role.to_string(), color.clone()))
        .collect();
        Scene {
            palette,
            haze: None,
            style: self.style,
            elements: vec![Element::Grid { rule: self.layout }],
            variation,
            // Smaller pockets need crisper edges to stay visible.
            softness: (self.layout == GridRule::Superscatter)
                .then(|| 0.24 + 0.18 * self.style.focus / 3.0),
            distortion: None,
            swirl: None,
            motion: None,
            motion_style: Some(match self.layout {
                GridRule::Sky | GridRule::Horizon => MotionStyle::Flow,
                _ => MotionStyle::Wander,
            }),
            warp_seed: 58,
            warp: Vec::new(),
        }
    }

    fn build(&self, variation: u32) -> Result<Scene, String> {
        let horizon = 0.25 + 0.125 * self.horizon_height;
        let mut elements = Vec::new();
        self.backdrop(horizon, &mut elements);
        if self.glow >= PRESENT {
            elements.push(self.glow_point(horizon, variation));
        }
        if self.land >= 2.0 - PRESENT {
            self.ridges(horizon, &mut elements);
        } else if self.land >= PRESENT {
            elements.push(self.swells(horizon));
        }
        if self.clouds >= PRESENT {
            elements.push(self.cloud_wisps(horizon));
        }
        if self.features >= PRESENT {
            self.feature_rows(horizon, &mut elements);
        }
        if self.rays >= RAYS_PRESENT {
            elements.push(self.ray_strokes(horizon));
        }
        if self.lights >= LIGHTS_PRESENT {
            self.light_field(horizon, &mut elements);
        }

        // Layered hills need some air between them to read as distance.
        let hills_haze = if self.land >= 2.0 - PRESENT { 2.0 } else { 0.0 };
        let colors = &self.colors;
        let palette: BTreeMap<String, ColorSpec> = [
            ("sky", &colors.sky),
            ("horizon", &colors.horizon),
            ("ground", &colors.ground),
            ("feature", &colors.feature),
            ("light", &colors.light),
        ]
        .into_iter()
        .map(|(role, color)| (role.to_string(), color.clone()))
        .collect();

        // Shapes with structure stay recognizable only under a gentle warp.
        let structured =
            self.land >= 1.5 || self.features >= PRESENT || self.lights >= LIGHTS_PRESENT;
        let energy = self.style.energy / 4.0;
        Ok(Scene {
            palette,
            haze: Some(Haze {
                role: "horizon".into(),
                strength: blend(&[0.0, 0.35, 0.65, 0.9], self.atmosphere.max(hills_haze)),
                shade: 0.3,
            }),
            style: Style {
                // Layers read as layers only through steps in value.
                contrast: if structured {
                    self.style.contrast.max(2.5)
                } else {
                    self.style.contrast
                },
                ..self.style
            },
            elements,
            variation,
            // Outlines need crisper edges than a pure wash does.
            softness: structured.then_some((0.36 + 0.22 * self.style.focus / 3.0).min(0.4)),
            distortion: structured.then_some((0.8 + 0.5 * energy) * 0.15),
            swirl: structured.then_some(0.0),
            motion: None,
            motion_style: Some(if structured {
                MotionStyle::Flow
            } else {
                MotionStyle::Wander
            }),
            warp_seed: 58,
            warp: Vec::new(),
        })
    }

    /// Sky from top to horizon, and ground below it when there is any.
    fn backdrop(&self, horizon: f32, elements: &mut Vec<Element>) {
        let gradient =
            |from: &str, to: &str, y: [f32; 2], rows: u32, shade: [f32; 2]| Element::Gradient {
                from: from.into(),
                to: to.into(),
                y,
                rows,
                shade,
                depth: [0.0, 0.0],
                radius: 0.4,
                stretch: 6.0,
                strength: 0.4,
                x: 0.5,
            };
        if self.land < PRESENT {
            elements.push(gradient("sky", "horizon", [0.0, 1.0], 10, [-0.4, 0.3]));
        } else {
            elements.push(gradient("sky", "horizon", [0.0, horizon], 8, [-0.4, 0.3]));
            // Hills and mountains cover the ground themselves; a floor of the
            // same color behind them would hide their outlines.
            if self.land < 2.0 - PRESENT {
                elements.push(gradient(
                    "horizon",
                    "ground",
                    [horizon + 0.06, 1.0],
                    6,
                    [0.3, -0.8],
                ));
            }
        }
    }

    /// One big soft point: a sun, a moon, a lamp.
    fn glow_point(&self, horizon: f32, variation: u32) -> Element {
        let x = 0.3 + 0.4 * ((variation.wrapping_mul(2654435761) >> 8) as f32 / 16777216.0);
        let top = 0.12;
        let y = horizon - 0.02 - (horizon - 0.02 - top) * self.glow_height / Self::GLOW_HEIGHT_MAX;
        Element::Point {
            position: [x, y],
            role: "light".into(),
            shade: 0.5,
            depth: 0.1,
            radius: 0.3,
            stretch: 1.0,
            angle: 0.0,
            strength: blend(&[0.0, 0.8, 1.8, 3.5], self.glow),
        }
    }

    /// Soft horizontal streaks of sky light on flat water or ground.
    fn swells(&self, horizon: f32) -> Element {
        Element::Strokes {
            role: "horizon".into(),
            count: 6,
            arrange: Arrange::Scatter,
            x: [0.0, 1.0],
            y: [horizon + 0.08, 0.95],
            length: [6.0, 10.0],
            width: 0.08,
            taper: 0.0,
            depth: [0.1, 0.5],
            angle: 90.0,
            angle_jitter: 3.0,
            jitter: 0.6,
            shade: 0.3,
            shade_jitter: 0.1,
            strength: 0.5,
        }
    }

    /// Long soft streaks across the sky.
    fn cloud_wisps(&self, horizon: f32) -> Element {
        Element::Strokes {
            role: "horizon".into(),
            count: blend(&[0.0, 2.0, 4.0, 7.0], self.clouds).round() as u32,
            arrange: Arrange::Scatter,
            x: [0.0, 1.0],
            y: [0.08, (horizon - 0.12).max(0.15)],
            length: [6.0, 9.0],
            width: 0.12,
            taper: 0.0,
            depth: [0.2, 0.5],
            angle: 90.0,
            angle_jitter: 4.0,
            jitter: 0.6,
            shade: 0.5,
            shade_jitter: 0.1,
            strength: 0.5,
        }
    }

    /// Hills or mountains: ridges from the far horizon down to the nearest.
    fn ridges(&self, horizon: f32, elements: &mut Vec<Element>) {
        let layers = (self.land + 0.5).floor().clamp(2.0, 4.0) as usize;
        let amplitude = blend(&[0.0, 0.0, 0.04, 0.09, 0.14], self.land);
        let frequency = blend(&[1.0, 1.0, 1.0, 1.4, 2.8], self.land);
        let roughness = blend(&[0.0, 0.0, 0.2, 0.4, 0.8], self.land);
        let far = horizon - 0.06;
        for layer in 0..layers {
            let near = layer as f32 / (layers - 1) as f32;
            // Distant hills are darker versions of the horizon's color, as
            // in real atmospheric perspective; only the nearest takes the
            // ground's own color.
            let nearest = layer == layers - 1;
            elements.push(Element::Ridge {
                role: if nearest { "ground" } else { "horizon" }.into(),
                y: far + (0.92 - far) * near,
                amplitude: amplitude * (0.7 + 0.6 * near),
                frequency: frequency * (1.4 - 0.8 * near),
                roughness,
                x: [-0.15, 1.15],
                spacing: 0.1,
                inset: 0.05,
                radius: 0.45,
                stretch: 3.0,
                shade: if nearest { -0.9 } else { -0.15 - 0.5 * near },
                shade_jitter: 0.1,
                depth: 0.9 * (1.0 - near),
                follow: true,
            });
        }
    }

    /// Upright features in up to three depth layers: many thin hazy ones far
    /// away, a few wide dark ones close up.
    fn feature_rows(&self, horizon: f32, elements: &mut Vec<Element>) {
        let total = blend(&[0.0, 4.0, 8.0, 14.0, 22.0], self.features).round();
        let width = blend(&[0.06, 0.1, 0.18, 0.3], self.feature_width);
        // (share of the count, width factor, depth, shade, taper)
        for (share, factor, depth, shade, taper) in [
            (0.6, 0.6, [0.6, 0.9], -0.3, 0.3),
            (0.3, 1.0, [0.3, 0.5], -0.4, 0.3),
            (0.1, 1.8, [0.0, 0.1], -0.5, 0.2),
        ] {
            let count = (total * share).round() as u32;
            if count == 0 {
                continue;
            }
            let width = (width * factor).max(0.05);
            let reach = 1.1;
            elements.push(Element::Strokes {
                role: "feature".into(),
                count,
                arrange: Arrange::Row,
                x: [-0.05, 1.05],
                y: [horizon - 0.03, horizon + 0.05],
                length: [
                    (0.8 * reach / width).clamp(1.0, 10.0),
                    (1.1 * reach / width).clamp(1.0, 10.0),
                ],
                width,
                taper,
                depth,
                angle: 0.0,
                angle_jitter: 1.5,
                jitter: 0.6,
                shade,
                shade_jitter: 0.1,
                strength: 1.2,
            });
        }
    }

    /// Slanted beams of light across the picture.
    fn ray_strokes(&self, horizon: f32) -> Element {
        Element::Strokes {
            role: "light".into(),
            count: blend(&[0.0, 0.0, 3.0, 7.0], self.rays).round() as u32,
            arrange: Arrange::Scatter,
            x: [0.1, 0.9],
            y: [0.15, horizon + 0.05],
            length: [8.0, 8.0],
            width: 0.08,
            taper: 0.0,
            depth: [0.3, 0.3],
            angle: 28.0,
            angle_jitter: 4.0,
            jitter: 0.6,
            shade: 0.4,
            shade_jitter: 0.1,
            strength: 1.2,
        }
    }

    /// Small lights, with a brighter core and a spill of bloom if they glow.
    fn light_field(&self, horizon: f32, elements: &mut Vec<Element>) {
        let count = blend(&[0.0, 0.0, 8.0, 18.0, 28.0], self.lights).round() as u32;
        let size = blend(&[0.05, 0.08, 0.12, 0.2], self.light_size);
        let y = match self.lights_at {
            LightsAt::Sky => [0.05, (horizon - 0.1).max(0.1)],
            LightsAt::Horizon => [(horizon - 0.2).max(0.05), horizon + 0.02],
            LightsAt::Ground => [horizon + 0.05, 0.95],
            LightsAt::Everywhere => [0.05, 0.95],
        };
        let glowing = self.light_glow >= PRESENT;
        let roles = vec![LightRole {
            role: "light".into(),
            weight: 1.0,
        }];
        if glowing {
            // A few big faint orbs behind the lights, so light seems to fill
            // the air around them.
            elements.push(Element::Lights {
                count: (count / 4).max(1),
                roles: roles.clone(),
                x: [0.1, 0.9],
                y,
                radius: [size * 1.8, size * 2.2],
                strength: 1.2,
                shade: [0.3, 0.6],
                depth: 0.5,
                core: 0.0,
                core_shade: 0.0,
                bloom: None,
            });
        }
        elements.push(Element::Lights {
            count,
            roles,
            x: [0.05, 0.95],
            y,
            radius: [(size * 0.8).max(0.05), size * 1.25],
            strength: 3.0 - 0.5 * self.light_size,
            shade: [0.3, 0.8],
            depth: 0.05,
            core: if glowing { 0.5 } else { 0.0 },
            core_shade: 0.4,
            bloom: glowing.then(|| Bloom {
                rings: (1.0 + self.light_glow).round().clamp(2.0, 3.0) as u32,
                spread: 1.9,
                strength: 0.15 + 0.1 * self.light_glow,
                fade: 0.5,
            }),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn composition(extra: serde_json::Value) -> Composition {
        let mut value = serde_json::json!({"colors": {
            "sky": "blue", "horizon": "coral", "ground": "brown",
            "feature": "#251b5c", "light": "yellow"
        }});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    fn kinds(scene: &Scene) -> Vec<&'static str> {
        scene.elements.iter().map(Element::kind).collect()
    }

    #[test]
    fn a_bare_composition_is_just_a_sky() {
        let scene = composition(serde_json::json!({})).to_scene(1).unwrap();
        assert_eq!(kinds(&scene), ["gradient"]);
        assert_eq!(scene.motion_style, Some(MotionStyle::Wander));
        assert!(scene.compile().is_ok());
    }

    #[test]
    fn each_part_appears_only_when_asked_for() {
        for (extra, expected) in [
            (
                serde_json::json!({"land": 1}),
                vec!["gradient", "gradient", "strokes"],
            ),
            (
                serde_json::json!({"land": 3}),
                vec!["gradient", "ridge", "ridge", "ridge"],
            ),
            (serde_json::json!({"glow": 2}), vec!["gradient", "point"]),
            (serde_json::json!({"rays": 2}), vec!["gradient", "strokes"]),
            (serde_json::json!({"lights": 2}), vec!["gradient", "lights"]),
            (
                serde_json::json!({"lights": 2, "light_glow": 2}),
                vec!["gradient", "lights", "lights"],
            ),
            // Below the halfway mark a part is absent.
            (
                serde_json::json!({"glow": 0.4, "lights": 0.4, "features": 0.4}),
                vec!["gradient"],
            ),
        ] {
            let scene = composition(extra.clone()).to_scene(1).unwrap();
            assert_eq!(kinds(&scene), expected, "{extra}");
            assert!(scene.compile().is_ok(), "{extra}");
        }
        // Features split into depth layers once there are enough of them.
        let few = composition(serde_json::json!({"features": 1}))
            .to_scene(1)
            .unwrap();
        let many = composition(serde_json::json!({"features": 4}))
            .to_scene(1)
            .unwrap();
        let strokes = |scene: &Scene| {
            scene
                .elements
                .iter()
                .filter(|e| e.kind() == "strokes")
                .count()
        };
        assert!(strokes(&few) < strokes(&many));
        assert_eq!(strokes(&many), 3);
    }

    #[test]
    fn a_feeling_is_a_field_of_color_not_a_scene() {
        let scenic = serde_json::json!({
            "land": 3, "features": 3, "rays": 3, "lights": 3, "glow": 2
        });
        let with = |concrete: f32, layout: &str| {
            let mut value = scenic.clone();
            value["concrete"] = concrete.into();
            value["layout"] = layout.into();
            composition(value).to_scene(1).unwrap()
        };
        // Fully concrete is the default and ignores the layout.
        assert_eq!(
            kinds(&composition(scenic.clone()).to_scene(1).unwrap()),
            kinds(&with(3.0, "wash"))
        );
        assert!(kinds(&with(3.0, "wash")).contains(&"lights"));
        // A feeling is just the layout, in the three colors, and nothing else.
        for layout in ["sky", "horizon", "glow", "scatter", "superscatter", "wash"] {
            let scene = with(0.0, layout);
            assert_eq!(kinds(&scene), ["grid"], "{layout}");
            assert_eq!(scene.palette.len(), 3);
            assert_eq!(scene.compile().unwrap().points.len(), 18, "{layout}");
        }
        let sky = with(0.0, "sky");
        assert_eq!(sky.motion_style, Some(MotionStyle::Flow));
        assert_eq!(sky.palette["main"], ColorSpec::Family("blue".into()));
        assert_eq!(with(0.0, "scatter").motion_style, Some(MotionStyle::Wander));
        // Small pockets are crisper than the same layout elsewhere.
        assert!(
            with(0.0, "superscatter").compile().unwrap().softness
                < with(0.0, "scatter").compile().unwrap().softness
        );
        // Just past the boundary a scene returns, with its objects partly there.
        assert!(kinds(&with(1.5, "wash")).contains(&"lights"));
        assert!(kinds(&with(1.4, "wash")) == ["grid"]);
    }

    #[test]
    fn clouds_and_swells_are_horizontal_streaks_of_sky_light() {
        let streaks = |value: serde_json::Value| -> Vec<(u32, f32)> {
            composition(value)
                .to_scene(1)
                .unwrap()
                .elements
                .iter()
                .filter_map(|element| match element {
                    Element::Strokes {
                        count, angle, role, ..
                    } if role == "horizon" => Some((*count, *angle)),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(streaks(serde_json::json!({})), []);
        // Clouds sit in the sky and lie flat.
        assert_eq!(streaks(serde_json::json!({"clouds": 2})), [(4, 90.0)]);
        assert_eq!(streaks(serde_json::json!({"clouds": 0.4})), []);
        // Flat water gets swells; hills do not.
        assert_eq!(streaks(serde_json::json!({"land": 1})), [(6, 90.0)]);
        assert_eq!(streaks(serde_json::json!({"land": 3})), []);
    }

    #[test]
    fn answers_between_settings_blend_them() {
        assert_eq!(blend(&[0.0, 10.0, 20.0], 0.0), 0.0);
        assert_eq!(blend(&[0.0, 10.0, 20.0], 1.5), 15.0);
        assert_eq!(blend(&[0.0, 10.0, 20.0], 2.0), 20.0);
        assert_eq!(blend(&[0.0, 10.0, 20.0], 9.0), 20.0);
        // More of something never means less of it.
        let count = |lights: f32| match &composition(serde_json::json!({"lights": lights}))
            .to_scene(1)
            .unwrap()
            .elements[1]
        {
            Element::Lights { count, .. } => *count,
            other => panic!("{}", other.kind()),
        };
        let counts: Vec<_> = (3..=8).map(|step| count(step as f32 / 2.0)).collect();
        assert!(
            counts.windows(2).all(|pair| pair[0] <= pair[1]),
            "{counts:?}"
        );
    }

    #[test]
    fn variation_moves_things_without_changing_the_design() {
        let design = composition(serde_json::json!({"lights": 3, "features": 3, "glow": 2}));
        let a = design.to_scene(1).unwrap().compile().unwrap();
        let b = design.to_scene(2).unwrap().compile().unwrap();
        assert_eq!(a.points.len(), b.points.len());
        assert_ne!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&b).unwrap()
        );
        assert_eq!(
            serde_json::to_value(design.to_scene(1).unwrap().compile().unwrap()).unwrap(),
            serde_json::to_value(&a).unwrap()
        );
    }

    #[test]
    fn out_of_range_levels_are_rejected() {
        for (field, value) in [
            ("horizon_height", 4.5),
            ("land", -1.0),
            ("features", 5.0),
            ("feature_width", 3.5),
            ("rays", 3.1),
            ("lights", 4.1),
            ("light_size", 4.0),
            ("light_glow", 3.1),
            ("glow", 3.1),
            ("glow_height", 3.1),
            ("clouds", 3.1),
            ("atmosphere", 3.1),
            ("concrete", 3.1),
        ] {
            let result = composition(serde_json::json!({ field: value })).to_scene(1);
            assert!(result.unwrap_err().contains(field), "{field}");
        }
        let mut nan = composition(serde_json::json!({}));
        nan.lights = f32::NAN;
        assert!(nan.to_scene(1).is_err());
        let unknown: Result<Composition, _> = serde_json::from_value(serde_json::json!({
            "colors": {"sky":"blue","horizon":"blue","ground":"blue","feature":"blue","light":"blue"},
            "mountains": 3
        }));
        assert!(unknown.is_err());
    }

    #[test]
    fn the_most_crowded_composition_stays_within_the_point_limit() {
        for lights_at in ["sky", "horizon", "ground", "everywhere"] {
            let scene = composition(serde_json::json!({
                "land": 4, "features": 4, "feature_width": 3, "rays": 3, "lights": 4, "clouds": 3,
                "light_size": 3, "light_glow": 3, "glow": 3, "glow_height": 3,
                "atmosphere": 3, "lights_at": lights_at, "horizon_height": 4
            }))
            .to_scene(9)
            .unwrap();
            let recipe = scene.compile().unwrap();
            assert!(recipe.points.len() <= 256, "{}", recipe.points.len());
        }
    }

    #[test]
    fn composition_files_load_wherever_scenes_do() {
        let dir =
            std::env::temp_dir().join(format!("amber-composition-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.json");
        std::fs::write(
            &path,
            r##"{"colors":{"sky":"blue","horizon":"coral","ground":"brown","feature":"#251b5c","light":"yellow"},"land":3}"##,
        )
        .unwrap();
        let scene = Scene::load(&path).unwrap();
        assert_eq!(
            kinds(&scene)
                .iter()
                .filter(|kind| **kind == "ridge")
                .count(),
            3
        );
        std::fs::write(&path, r#"{"colors":{"sky":"blue"}}"#).unwrap();
        let error = Scene::load(&path).unwrap_err().to_string();
        assert!(error.contains("invalid JSON composition"), "{error}");
        for name in ["forest", "hills", "city", "lighthouse"] {
            let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/compositions")
                .join(format!("{name}.json"));
            assert!(Scene::load(&example).is_ok(), "{name}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
