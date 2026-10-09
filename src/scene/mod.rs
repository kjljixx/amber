//! Scenes: a palette of named roles plus a list of elements, compiled to a
//! [`Recipe`].
//!
//! Points are the only thing the renderer knows about. An element is a small
//! generator that emits points (a ridge of hills, a row of trunks, scattered
//! lights); a scene lists them over a palette of named color roles, so the same
//! scene reads differently in different colors. Compilation is deterministic:
//! the same scene always produces the same recipe, and `variation` changes the
//! random placement without changing the design.
mod color;
mod elements;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use color::ColorSpec;
#[cfg(feature = "generation")]
pub(crate) use color::{COLORS, family};
pub use elements::{Arrange, Bloom, Element, GridRule, LightRole};

use crate::{
    MotionStyle, Point, Recipe,
    color::{Oklab, oklab_to_rgb},
    warp::WarpLayer,
};
use color::Tone;

const MAX_ROLES: usize = 32;
const MAX_ELEMENTS: usize = 32;

/// Global look of a scene. Each value scores one feel, as the Jev decisions do.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Style {
    /// 0 very dark to 4 very light.
    pub lightness: f32,
    /// 0 almost neutral to 4 intensely saturated.
    pub saturation: f32,
    /// 0 very little to 3 dramatic. Scales how far shades move colors.
    pub contrast: f32,
    /// 0 still to 4 very energetic. Sets warp strength and motion unless
    /// the scene overrides them.
    pub energy: f32,
    /// 0 crisp to 3 very diffuse. Sets edge softness unless overridden.
    pub focus: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            lightness: 2.0,
            saturation: 2.0,
            contrast: 1.5,
            energy: 2.0,
            focus: 1.5,
        }
    }
}

/// Distant elements fade toward one palette role, like air between hills.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Haze {
    pub role: String,
    /// How much an element with `depth: 1` is pulled toward the haze (0 to 1).
    #[serde(default = "default_haze_strength")]
    pub strength: f32,
    /// Shade of the haze color; positive is lighter.
    #[serde(default = "default_haze_shade")]
    pub shade: f32,
}

fn default_haze_strength() -> f32 {
    0.75
}
fn default_haze_shade() -> f32 {
    0.3
}
fn default_variation() -> u32 {
    1
}
fn default_warp_seed() -> u16 {
    58
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    /// Named color roles that elements refer to.
    pub palette: BTreeMap<String, ColorSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub haze: Option<Haze>,
    #[serde(default)]
    pub style: Style,
    /// Emitted in order; later elements sit visually between or over earlier
    /// ones only through where their points land.
    pub elements: Vec<Element>,
    /// Seeds the random placement and color variation.
    #[serde(default = "default_variation")]
    pub variation: u32,
    /// Overrides for recipe settings that otherwise follow `style`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub softness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distortion: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swirl: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<f32>,
    /// Defaults to `wander`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_style: Option<MotionStyle>,
    #[serde(default = "default_warp_seed")]
    pub warp_seed: u16,
    /// Extra warp layers, as in a recipe's `warp`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warp: Vec<WarpLayer>,
}

/// State threaded through element emission.
pub(crate) struct Ctx<'a> {
    palette: &'a BTreeMap<String, ColorSpec>,
    tone: Tone,
    haze: Option<(Oklab, f32)>,
    rng: u32,
    points: Vec<Point>,
}

/// How a point's region is shaped; see [`Point`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shape {
    pub radius: f32,
    pub stretch: f32,
    pub angle: f32,
    pub strength: f32,
}

impl Default for Shape {
    fn default() -> Self {
        Self {
            radius: 1.0,
            stretch: 1.0,
            angle: 0.0,
            strength: 1.0,
        }
    }
}

impl Ctx<'_> {
    /// Uniform in [0, 1). A small linear generator keeps scenes reproducible
    /// everywhere without a dependency.
    pub fn random(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.rng >> 8) as f32 / 16777216.0
    }

    /// A role's color with a little random variation (two random draws).
    pub fn color(&mut self, role: &str, shade: f32) -> Oklab {
        let lch = self.palette[role].lch(&self.tone, shade);
        let l = (lch.l + (self.random() - 0.5) * 0.04).clamp(0.08, 0.97);
        let h = lch.h + (self.random() - 0.5) * 12.0;
        color::Lch { l, c: lch.c, h }.to_oklab()
    }

    fn hazed(&self, lab: Oklab, depth: f32) -> Oklab {
        match self.haze {
            Some((haze, strength)) if depth > 0.0 => {
                let amount = depth.min(1.0) * strength;
                [0, 1, 2].map(|channel| lab[channel] + (haze[channel] - lab[channel]) * amount)
            }
            _ => lab,
        }
    }

    /// Adds a point of an exact color (after haze). Positions are kept within
    /// the range the renderer accepts.
    pub fn emit_lab(&mut self, position: [f32; 2], lab: Oklab, depth: f32, shape: Shape) {
        let lab = self.hazed(lab, depth);
        self.points.push(Point {
            position: position.map(|value| value.clamp(-0.5, 1.5)),
            color: oklab_to_rgb(lab),
            radius: shape.radius,
            stretch: shape.stretch,
            strength: shape.strength,
            angle: shape.angle,
        });
    }

    pub fn emit(&mut self, position: [f32; 2], role: &str, shade: f32, depth: f32, shape: Shape) {
        let lab = self.color(role, shade);
        self.emit_lab(position, lab, depth, shape);
    }
}

fn check(what: &str, value: f32, low: f32, high: f32) -> Result<(), String> {
    if (low..=high).contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{what} must be finite and between {low} and {high}"
        ))
    }
}

impl Scene {
    /// Reads, parses, and validates a scene file.
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        let data = std::fs::read(path).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("could not read scene '{}': {error}", path.display()),
            )
        })?;
        let scene: Self = serde_json::from_slice(&data).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid JSON scene '{}': {error}", path.display()),
            )
        })?;
        scene.compile().map_err(|message| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid scene '{}': {message}", path.display()),
            )
        })?;
        Ok(scene)
    }

    fn validate(&self) -> Result<(), String> {
        if self.palette.is_empty() || self.palette.len() > MAX_ROLES {
            return Err(format!("a palette needs between 1 and {MAX_ROLES} roles"));
        }
        if self
            .palette
            .keys()
            .any(|name| name.is_empty() || name.chars().count() > 32)
        {
            return Err("palette role names must be 1 to 32 characters".into());
        }
        let style = &self.style;
        check("style lightness", style.lightness, 0.0, 4.0)?;
        check("style saturation", style.saturation, 0.0, 4.0)?;
        check("style contrast", style.contrast, 0.0, 3.0)?;
        check("style energy", style.energy, 0.0, 4.0)?;
        check("style focus", style.focus, 0.0, 3.0)?;
        if let Some(haze) = &self.haze {
            if !self.palette.contains_key(&haze.role) {
                return Err(format!("haze role '{}' is not in the palette", haze.role));
            }
            check("haze strength", haze.strength, 0.0, 1.0)?;
            check("haze shade", haze.shade, -1.0, 1.0)?;
        }
        if self.elements.is_empty() || self.elements.len() > MAX_ELEMENTS {
            return Err(format!(
                "a scene needs between 1 and {MAX_ELEMENTS} elements"
            ));
        }
        for (index, element) in self.elements.iter().enumerate() {
            element.validate(&self.palette).map_err(|message| {
                format!("element {} ({}): {message}", index + 1, element.kind())
            })?;
        }
        Ok(())
    }

    /// Builds the recipe. Deterministic for a given scene.
    pub fn compile(&self) -> Result<Recipe, String> {
        self.validate()?;
        let style = &self.style;
        let tone = Tone::new(style.lightness, style.saturation, style.contrast);
        let haze = self.haze.as_ref().map(|haze| {
            let lab = self.palette[&haze.role].lch(&tone, haze.shade).to_oklab();
            (lab, haze.strength)
        });
        let mut ctx = Ctx {
            palette: &self.palette,
            tone,
            haze,
            rng: self.variation,
            points: Vec::new(),
        };
        for element in &self.elements {
            element.emit(&mut ctx);
        }

        let energy = style.energy / 4.0;
        let swirl = match self.swirl {
            Some(swirl) => swirl,
            None if energy > 0.5 => (ctx.random() - 0.5) * 0.6 * (energy - 0.5) * 2.0,
            None => 0.0,
        };
        let recipe = Recipe {
            points: ctx.points,
            softness: self.softness.unwrap_or(0.36 + 0.22 * style.focus / 3.0),
            distortion: self.distortion.unwrap_or(0.8 + 0.5 * energy),
            swirl,
            seed: self.warp_seed,
            motion: self.motion.unwrap_or(0.3 + 0.6 * energy),
            motion_style: self.motion_style.unwrap_or(MotionStyle::Wander),
            warp: self.warp.clone(),
        };
        recipe.compile().map_err(str::to_string)?;
        Ok(recipe)
    }
}

/// Scenes bundled with the library, as JSON.
const TEMPLATES: &[(&str, &str)] = &[
    ("desert_hills", include_str!("templates/desert_hills.json")),
    (
        "redwood_forest",
        include_str!("templates/redwood_forest.json"),
    ),
    ("city_lights", include_str!("templates/city_lights.json")),
];

/// Names of the bundled scenes.
pub fn template_names() -> impl Iterator<Item = &'static str> {
    TEMPLATES.iter().map(|(name, _)| *name)
}

/// A bundled scene by name. Edit its palette or `variation` to make it yours.
pub fn template(name: &str) -> Option<Scene> {
    TEMPLATES
        .iter()
        .find(|(id, _)| *id == name)
        .map(|(_, json)| serde_json::from_str(json).expect("bundled scenes are valid JSON"))
}

#[cfg(test)]
mod tests;
