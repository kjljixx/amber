use crate::{
    gradient::{Gradient, Point, PreparedGradient},
    warp::{Clock, LayerMotion, Warp, WarpLayer, WarpStack},
};

/// Motion behavior for a recipe's color regions. Older JSON recipes keep looping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionStyle {
    #[default]
    Loop,
    Flow,
    Wander,
    /// The glow center drifts and expands or contracts.
    Glow,
    /// Color regions wander far enough to cross one another.
    Scatter,
    Superscatter,
    /// Broad color patches drift gently.
    Wash,
}

/// Plain gradient settings, independent of terminal size and playback controls.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub points: Vec<Point>,
    pub softness: f32,
    pub distortion: f32,
    pub swirl: f32,
    pub seed: u16,
    pub motion: f32,
    #[serde(default)]
    pub motion_style: MotionStyle,
    /// Extra warp layers, applied after the classic `distortion` and `swirl`
    /// layers. Set both of those to 0 to use only these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warp: Vec<WarpLayer>,
}

/// Validated settings with color conversions and seeded warp setup cached.
pub struct CompiledRecipe {
    gradient: Gradient,
    warp: WarpStack,
    motion: f32,
    motion_style: MotionStyle,
}

impl Recipe {
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        let data = std::fs::read(path).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("could not read recipe '{}': {error}", path.display()),
            )
        })?;
        let recipe: Self = serde_json::from_slice(&data).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid JSON recipe '{}': {error}", path.display()),
            )
        })?;
        // Validate before entering the terminal UI.
        recipe.compile().map_err(|message| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid recipe '{}': {message}", path.display()),
            )
        })?;
        Ok(recipe)
    }

    pub fn compile(&self) -> Result<CompiledRecipe, &'static str> {
        if !self.motion.is_finite() || !(0.0..=1.5).contains(&self.motion) {
            return Err("motion must be finite and between 0 and 1.5");
        }
        Ok(CompiledRecipe {
            gradient: Gradient::new(&self.points, self.softness)?,
            warp: WarpStack::new(self.distortion, self.swirl, self.seed, &self.warp)?,
            motion: self.motion,
            motion_style: self.motion_style,
        })
    }
}

impl CompiledRecipe {
    pub(crate) fn seed(&self) -> u16 {
        self.warp.seed()
    }

    pub fn prepare(
        &self,
        aspect: f32,
        phase: f32,
        warped: bool,
        drifting: bool,
    ) -> PreparedGradient {
        self.prepare_at(aspect, phase, f64::from(phase), warped, drifting)
    }

    /// `travel` advances without wrapping; `phase` controls periodic effects.
    pub fn prepare_at(
        &self,
        aspect: f32,
        phase: f32,
        travel: f64,
        warped: bool,
        drifting: bool,
    ) -> PreparedGradient {
        self.prepare_intensified(aspect, phase, travel, warped, drifting, 1.0)
    }

    /// Like `prepare_at`, with the warp displacements multiplied by `intensity`
    /// (1 leaves the recipe as written).
    pub fn prepare_intensified(
        &self,
        aspect: f32,
        phase: f32,
        travel: f64,
        warped: bool,
        drifting: bool,
        intensity: f32,
    ) -> PreparedGradient {
        let motion = if drifting { self.motion } else { 0.0 };
        let field = self.gradient.prepare(aspect);
        let field = match self.motion_style {
            MotionStyle::Loop => field.with_drift(phase, motion),
            MotionStyle::Flow => field,
            MotionStyle::Wander => field.with_wander(travel, motion, self.warp.seed()),
            MotionStyle::Glow => field.with_roaming_glow(travel, motion, self.warp.seed()),
            MotionStyle::Scatter => {
                field.with_scatter_wander(travel, motion, self.warp.seed(), 0.12)
            }
            MotionStyle::Superscatter => {
                field.with_scatter_wander(travel, motion, self.warp.seed(), 0.15)
            }
            MotionStyle::Wash => field.with_soft_wash(travel, motion, self.warp.seed()),
        };
        field.with_warp(if warped {
            self.warp
                .at(&Clock {
                    phase,
                    travel,
                    motion,
                    aspect,
                    inherited: if self.motion_style == MotionStyle::Loop {
                        LayerMotion::Cycle
                    } else if self.motion_style == MotionStyle::Flow {
                        LayerMotion::Drift
                    } else {
                        LayerMotion::Wander
                    },
                })
                .intensified(intensity)
        } else {
            Warp::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../examples/complex.json");

    #[test]
    fn continuous_motion_preserves_initial_layout_and_does_not_repeat_at_tau() {
        let mut recipe = crate::presets::presets()[1].recipe.clone();
        let static_field = recipe.compile().unwrap().prepare(1.0, 0.0, true, false);
        for style in [MotionStyle::Flow, MotionStyle::Wander] {
            recipe.motion_style = style;
            let compiled = recipe.compile().unwrap();
            let initial = compiled.prepare_at(1.0, 0.0, 0.0, true, true);
            let next_cycle = compiled.prepare_at(1.0, 0.0, std::f64::consts::TAU, true, true);
            let samples = |field: &PreparedGradient| {
                (0..100)
                    .map(|i| field.sample((i % 10) as f32 / 9.0, (i / 10) as f32 / 9.0))
                    .collect::<Vec<_>>()
            };
            assert_eq!(samples(&initial), samples(&static_field));
            assert_ne!(samples(&initial), samples(&next_cycle));
            let disabled = compiled.prepare_at(1.0, 0.0, 100.0, true, false);
            assert_eq!(samples(&disabled), samples(&static_field));
        }
        assert_eq!(
            serde_json::from_str::<Recipe>(EXAMPLE)
                .unwrap()
                .motion_style,
            MotionStyle::Loop
        );
    }

    #[test]
    fn example_json_matches_complex_preset() {
        let loaded: Recipe = serde_json::from_str(EXAMPLE).unwrap();
        let [_, complex] = crate::presets::presets();
        let expected = complex.recipe;
        assert_eq!(loaded.points.len(), expected.points.len());
        for (a, b) in loaded.points.iter().zip(&expected.points) {
            assert_eq!(a.position, b.position);
            assert_eq!(a.color, b.color);
        }
        assert_eq!(loaded.softness, expected.softness);
        assert_eq!(loaded.distortion, expected.distortion);
        assert_eq!(loaded.swirl, expected.swirl);
        assert_eq!(loaded.seed, expected.seed);
        assert_eq!(loaded.motion, expected.motion);
        assert!(loaded.compile().is_ok());
    }

    #[test]
    fn json_rejects_missing_unknown_and_malformed_fields() {
        let original: serde_json::Value = serde_json::from_str(EXAMPLE).unwrap();
        let mut cases = Vec::new();
        let mut value = original.clone();
        value.as_object_mut().unwrap().remove("motion");
        cases.push(value);
        let mut value = original.clone();
        value["softnes"] = serde_json::json!(0.5);
        cases.push(value);
        let mut value = original.clone();
        value["points"][0]["color"] = serde_json::json!([256, 0, 0]);
        cases.push(value);
        let mut value = original.clone();
        value["points"][0]["position"] = serde_json::json!([0.5]);
        cases.push(value);
        let mut value = original;
        value["points"][0]["extra"] = serde_json::json!(true);
        cases.push(value);
        for value in cases {
            assert!(serde_json::from_value::<Recipe>(value).is_err());
        }
        assert!(serde_json::from_str::<Recipe>("{").is_err());
    }

    #[test]
    fn json_settings_must_pass_renderer_validation() {
        let original: serde_json::Value = serde_json::from_str(EXAMPLE).unwrap();
        for (field, invalid) in [
            ("motion", 1.6),
            ("softness", 1.1),
            ("distortion", -0.1),
            ("swirl", 1.1),
            ("seed", 1000.0),
        ] {
            let mut value = original.clone();
            value[field] = if field == "seed" {
                serde_json::json!(1000)
            } else {
                serde_json::json!(invalid)
            };
            assert!(
                serde_json::from_value::<Recipe>(value)
                    .unwrap()
                    .compile()
                    .is_err()
            );
        }
        let mut value = original.clone();
        value["points"][0]["position"] = serde_json::json!([-0.6, 0.5]);
        assert!(
            serde_json::from_value::<Recipe>(value)
                .unwrap()
                .compile()
                .is_err()
        );
        let mut value = original;
        value["points"] = serde_json::json!([]);
        assert!(
            serde_json::from_value::<Recipe>(value)
                .unwrap()
                .compile()
                .is_err()
        );
    }

    #[test]
    fn recipes_preserve_preset_output_with_every_effect_combination() {
        for preset in crate::presets::presets() {
            let recipe = preset.recipe;
            let compiled = recipe.compile().unwrap();
            let gradient = Gradient::new(&recipe.points, recipe.softness).unwrap();
            let warp = WarpStack::new(recipe.distortion, recipe.swirl, recipe.seed, &[]).unwrap();
            for aspect in [0.5, 1.0, 2.0] {
                for phase in [0.0, 1.2] {
                    for warped in [false, true] {
                        for drifting in [false, true] {
                            let actual = compiled.prepare(aspect, phase, warped, drifting);
                            let previous = gradient
                                .prepare(aspect)
                                .with_drift(phase, if drifting { 0.5 } else { 0.0 })
                                .with_warp(if warped {
                                    warp.at(&Clock {
                                        phase,
                                        travel: 0.0,
                                        motion: 0.0,
                                        aspect,
                                        inherited: LayerMotion::Cycle,
                                    })
                                } else {
                                    Warp::default()
                                });
                            for y in 0..8 {
                                for x in 0..8 {
                                    let (x, y) = (x as f32 / 7.0, y as f32 / 7.0);
                                    assert_eq!(actual.sample(x, y), previous.sample(x, y));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn warp_layers_load_from_json_and_change_the_render() {
        let mut value: serde_json::Value = serde_json::from_str(EXAMPLE).unwrap();
        assert!(value.get("warp").is_none());
        let plain: Recipe = serde_json::from_value(value.clone()).unwrap();
        assert!(plain.warp.is_empty());
        // Unset layers stay out of serialized recipes.
        assert!(serde_json::to_value(&plain).unwrap().get("warp").is_none());

        value["warp"] = serde_json::json!([
            {"type": "tilt", "angle": 15},
            {"type": "ripple", "amplitude": 0.04, "frequency": 14, "decay": 3,
             "animation": {"mode": "drift", "speed": 2}}
        ]);
        let layered: Recipe = serde_json::from_value(value).unwrap();
        assert_eq!(layered.warp.len(), 2);
        let round_trip: Recipe =
            serde_json::from_value(serde_json::to_value(&layered).unwrap()).unwrap();
        assert_eq!(round_trip.warp, layered.warp);

        let sample = |recipe: &Recipe| {
            let field = recipe
                .compile()
                .unwrap()
                .prepare_at(1.0, 0.0, 0.0, true, false);
            (0..64)
                .map(|i| field.sample((i % 8) as f32 / 7.0, (i / 8) as f32 / 7.0))
                .collect::<Vec<_>>()
        };
        assert_ne!(sample(&plain), sample(&layered));
        // Turning the warp off disables the layers along with the classic stack.
        let off = |recipe: &Recipe| {
            let field = recipe
                .compile()
                .unwrap()
                .prepare_at(1.0, 0.0, 0.0, false, false);
            field.sample(0.3, 0.6)
        };
        assert_eq!(off(&plain), off(&layered));
    }

    #[test]
    fn bundled_example_recipes_load() {
        let mut found = 0;
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/examples")).unwrap() {
            let path = entry.unwrap().path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                Recipe::load(&path).unwrap();
                found += 1;
            }
        }
        assert!(found >= 3, "expected the bundled examples, found {found}");
    }

    #[test]
    fn invalid_warp_layers_are_rejected_before_rendering() {
        let mut value: serde_json::Value = serde_json::from_str(EXAMPLE).unwrap();
        for bad in [
            serde_json::json!([{"type": "tilt", "angle": 400}]),
            serde_json::json!([{"type": "noise", "strength": 3}]),
        ] {
            value["warp"] = bad;
            let recipe: Recipe = serde_json::from_value(value.clone()).unwrap();
            assert!(recipe.compile().is_err());
        }
        for malformed in [
            serde_json::json!([{"type": "tilt"}]),
            serde_json::json!([{"type": "spin", "angle": 3}]),
            serde_json::json!({"type": "tilt", "angle": 3}),
        ] {
            value["warp"] = malformed;
            assert!(serde_json::from_value::<Recipe>(value.clone()).is_err());
        }
    }

    #[test]
    fn invalid_motion_is_rejected_before_rendering() {
        let [preset, _] = crate::presets::presets();
        let mut recipe = preset.recipe;
        for motion in [f32::NAN, f32::INFINITY, -0.1, 1.6] {
            recipe.motion = motion;
            assert!(recipe.compile().is_err());
        }
    }

    #[test]
    fn recipe_motion_controls_drift_amplitude() {
        let [preset, _] = crate::presets::presets();
        let mut recipe = preset.recipe;
        recipe.motion = 0.0;
        let compiled = recipe.compile().unwrap();
        for x in 0..10 {
            let x = x as f32 / 9.0;
            assert_eq!(
                compiled.prepare(1.0, 1.2, false, true).sample(x, 0.5),
                compiled.prepare(1.0, 1.2, false, false).sample(x, 0.5)
            );
        }
    }
}
