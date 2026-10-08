use crate::{
    gradient::{Gradient, Point, PreparedGradient},
    warp::Warp,
};

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
}

/// Validated settings with color conversions and seeded warp setup cached.
pub struct CompiledRecipe {
    gradient: Gradient,
    warp: Warp,
    motion: f32,
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
            warp: Warp::new(self.distortion, self.swirl, self.seed)?,
            motion: self.motion,
        })
    }
}

impl CompiledRecipe {
    pub fn prepare(
        &self,
        aspect: f32,
        phase: f32,
        warped: bool,
        drifting: bool,
    ) -> PreparedGradient {
        self.gradient
            .prepare(aspect)
            .with_drift(phase, if drifting { self.motion } else { 0.0 })
            .with_warp(if warped {
                self.warp.at_time(phase)
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
        value["points"][0]["position"] = serde_json::json!([-0.1, 0.5]);
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
            let warp = Warp::new(recipe.distortion, recipe.swirl, recipe.seed).unwrap();
            for aspect in [0.5, 1.0, 2.0] {
                for phase in [0.0, 1.2] {
                    for warped in [false, true] {
                        for drifting in [false, true] {
                            let actual = compiled.prepare(aspect, phase, warped, drifting);
                            let previous = gradient
                                .prepare(aspect)
                                .with_drift(phase, if drifting { 0.5 } else { 0.0 })
                                .with_warp(if warped {
                                    warp.at_time(phase)
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
