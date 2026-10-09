//! Jev makes bounded design decisions; local code builds a validated recipe.
//! No network requests are made by the renderer or by `GradientView`.
use std::{collections::BTreeMap, path::Path, time::Duration};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Point, Recipe, color::oklab_to_rgb};

const ENDPOINT: &str = "https://openrouter.ai/api/alpha/decisions";
pub const DEFAULT_MODEL: &str = "typesafe/jev-1.13";

// OKLCH families; local conversion keeps the model away from raw pixel math.
const COLORS: &[(&str, f32, f32, f32)] = &[
    ("red", 25.0, 0.20, 0.62),
    ("coral", 35.0, 0.15, 0.72),
    ("orange", 55.0, 0.18, 0.70),
    ("amber", 75.0, 0.15, 0.76),
    ("yellow", 100.0, 0.17, 0.90),
    ("lime", 125.0, 0.17, 0.84),
    ("green", 150.0, 0.14, 0.65),
    ("teal", 185.0, 0.11, 0.65),
    ("cyan", 205.0, 0.13, 0.80),
    ("sky", 235.0, 0.10, 0.80),
    ("blue", 260.0, 0.18, 0.58),
    ("indigo", 275.0, 0.15, 0.38),
    ("violet", 300.0, 0.16, 0.62),
    ("magenta", 335.0, 0.22, 0.65),
    ("pink", 355.0, 0.12, 0.80),
    ("brown", 55.0, 0.07, 0.42),
    ("cream", 85.0, 0.04, 0.92),
    ("white", 250.0, 0.01, 0.97),
    ("grey", 250.0, 0.015, 0.62),
    ("black", 270.0, 0.015, 0.14),
];

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    Sky,
    Horizon,
    Glow,
    Scatter,
    Superscatter,
    Wash,
}

#[derive(Clone, Debug)]
pub struct Design {
    pub hues: [String; 3],
    pub top: String,
    pub layout: Layout,
    pub lightness: f32,
    pub saturation: f32,
    pub contrast: f32,
    pub energy: f32,
    pub focus: f32,
}

#[derive(Clone, Debug)]
pub struct GeneratedGradient {
    pub recipe: Recipe,
    pub radians_per_second: f64,
    pub design: Design,
    pub cost_usd: Option<f64>,
}

/// Keeps credentials private; cloning the client reuses its HTTP connection pool.
#[derive(Clone)]
pub struct JevClient {
    key: String,
    model: String,
    agent: ureq::Agent,
}

impl JevClient {
    pub fn new(key: String, model: String) -> Result<Self, String> {
        if key.trim().is_empty() || key.chars().any(char::is_control) {
            return Err("Configure OPENROUTER_API_KEY in the environment or .env".into());
        }
        if model.trim().is_empty() {
            return Err("JEV_MODEL must not be empty".into());
        }
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .into();
        Ok(Self {
            key: key.trim().into(),
            model,
            agent,
        })
    }

    /// Environment variables take precedence over the optional local dotenv file.
    /// Does not mutate process-wide environment variables.
    pub fn from_env(path: &Path) -> Result<Self, String> {
        let file = match std::fs::read_to_string(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(_) => return Err("Could not read local .env configuration".into()),
        };
        let settings = dotenv_settings(&file);
        let get = |name: &str| {
            std::env::var(name)
                .ok()
                .or_else(|| settings.get(name).cloned())
        };
        Self::new(
            get("OPENROUTER_API_KEY").unwrap_or_default(),
            get("JEV_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into()),
        )
    }

    /// Blocking HTTP call; hosts should invoke it on a worker thread.
    pub fn generate(&self, prompt: &str) -> Result<GeneratedGradient, String> {
        self.generate_at(prompt, ENDPOINT)
    }

    fn generate_at(&self, prompt: &str, endpoint: &str) -> Result<GeneratedGradient, String> {
        let request = decision_request(prompt, &self.model)?;
        let mut response = self
            .agent
            .post(endpoint)
            .header("Authorization", format!("Bearer {}", self.key))
            .header("X-Title", "Amber Gradients")
            .send_json(&request)
            .map_err(|_| "Could not reach OpenRouter (connection or timeout)".to_string())?;
        match response.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err("OpenRouter rejected the API key or model access".into()),
            402 => return Err("OpenRouter account needs credits".into()),
            429 => return Err("OpenRouter rate limit reached; try again shortly".into()),
            code => return Err(format!("OpenRouter returned HTTP {code}")),
        }
        let envelope: Envelope = response
            .body_mut()
            .with_config()
            .limit(256 * 1024)
            .read_json()
            .map_err(|_| "OpenRouter returned an invalid decision response".to_string())?;
        let design = parse_design(&envelope.answers)?;
        let recipe = design.to_recipe(prompt)?;
        Ok(GeneratedGradient {
            recipe,
            radians_per_second: f64::from(0.2 + 0.9 * design.energy / 4.0) * 0.3,
            design,
            cost_usd: envelope
                .usage
                .and_then(|usage| usage.cost)
                .filter(|cost| cost.is_finite() && *cost >= 0.0),
        })
    }
}

fn dotenv_settings(file: &str) -> BTreeMap<String, String> {
    file.lines()
        .filter_map(|line| {
            let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
            let (name, value) = line.split_once('=')?;
            let name = name.trim();
            if !["OPENROUTER_API_KEY", "JEV_MODEL"].contains(&name) {
                return None;
            }
            let value = value.trim();
            let value = if value.starts_with('"') || value.starts_with('\'') {
                value.get(1..)?.split(value.chars().next()?).next()?
            } else {
                value.split('#').next()?.trim()
            };
            Some((name.to_string(), value.to_string()))
        })
        .collect()
}

pub fn decision_request(prompt: &str, model: &str) -> Result<Value, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Describe a gradient first".into());
    }
    if prompt.chars().count() > 1000 {
        return Err("Keep the description under 1,000 characters".into());
    }
    let palette: BTreeMap<_, _> = COLORS
        .iter()
        .map(|(name, _, _, _)| (*name, format!("The {name} color family")))
        .collect();
    let choice = |instructions: &str| json!({"type":"choice", "instructions":instructions, "criteria":palette});
    let score = |instructions: &str, criteria: &[&str]| json!({"type":"score", "instructions":instructions, "criteria":criteria});
    Ok(json!({
        "model": model,
        "state": {"description":prompt, "task":"Interpret this description as an abstract animated color gradient, not a literal picture."},
        "questions": {
            "hue_main": choice("Which color family best represents the dominant color of this description?"),
            "hue_second": choice("Which supporting color family best complements this description?"),
            "hue_accent": choice("Which accent color family best captures highlights or contrast in this description?"),
            "hue_top": choice("Which color family belongs toward the top of this composition?"),
            "layout": {"type":"choice", "instructions":"Which spatial composition best represents this description?", "criteria": {
                "sky":"Broad horizontal color regions changing from top to bottom",
                "horizon":"A bright seam separating upper and lower color regions",
                "glow":"A luminous central region with a darker outer region",
                "scatter":"Broad, softly connected colored regions distributed across the image",
                "superscatter":"Many smaller irregular color pockets interspersed throughout the image, like confetti, mosaic colors, or scattered lights",
                "wash":"Mostly one color with gentle, subtle variations"
            }},
            "lightness": score("How light should this gradient feel?", &["Very dark", "Dark", "Medium", "Light", "Very light"]),
            "saturation": score("How saturated should the colors feel?", &["Almost neutral", "Muted", "Moderate", "Vivid", "Intensely saturated"]),
            "contrast": score("How much contrast should the color regions have?", &["Very little", "Gentle", "Strong", "Dramatic"]),
            "energy": score("How energetic should the animated movement feel?", &["Still and calm", "Slow", "Moderate", "Lively", "Very energetic"]),
            "focus": score("How soft should the edges of the color regions be?", &["Crisp", "Slightly soft", "Soft", "Very diffuse"])
        }
    }))
}

#[derive(Deserialize)]
struct Envelope {
    answers: BTreeMap<String, Answer>,
    usage: Option<Usage>,
}
#[derive(Deserialize)]
struct Usage {
    cost: Option<f64>,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Answer {
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f32>,
    },
    Score {
        score: f32,
    },
}

fn family(name: &str) -> Option<(f32, f32, f32)> {
    COLORS
        .iter()
        .find(|(id, _, _, _)| *id == name)
        .map(|(_, hue, chroma, light)| (*hue, *chroma, *light))
}

fn parse_design(answers: &BTreeMap<String, Answer>) -> Result<Design, String> {
    let color = |name: &str, excluded: &[&str]| -> Result<String, String> {
        let Some(Answer::Choice {
            choice,
            probabilities,
        }) = answers.get(name)
        else {
            return Err(format!("Jev response is missing the {name} color decision"));
        };
        if family(choice).is_none()
            || probabilities
                .values()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err(format!("Jev returned an invalid {name} color decision"));
        }
        probabilities
            .iter()
            .filter(|(key, weight)| {
                family(key).is_some() && !excluded.contains(&key.as_str()) && **weight > 0.0
            })
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(key, _)| key.clone())
            .ok_or_else(|| format!("Jev returned no usable {name} colors"))
    };
    let main = color("hue_main", &[])?;
    let second = color("hue_second", &[&main])?;
    let accent = color("hue_accent", &[&main, &second])?;
    let top_candidate = color("hue_top", &[])?;
    let hues = [main, second, accent];
    let top = if hues.contains(&top_candidate) {
        top_candidate
    } else {
        hues[0].clone()
    };
    let Some(Answer::Choice { choice, .. }) = answers.get("layout") else {
        return Err("Jev response is missing the layout decision".into());
    };
    let layout: Layout = serde_json::from_value(json!(choice))
        .map_err(|_| "Jev returned an unknown layout".to_string())?;
    let score = |name: &str, max: f32| -> Result<f32, String> {
        match answers.get(name) {
            Some(Answer::Score { score }) if score.is_finite() && (0.0..=max).contains(score) => {
                Ok(*score)
            }
            _ => Err(format!(
                "Jev returned a missing or out-of-range {name} score"
            )),
        }
    };
    Ok(Design {
        hues,
        top,
        layout,
        lightness: score("lightness", 4.0)?,
        saturation: score("saturation", 4.0)?,
        contrast: score("contrast", 3.0)?,
        energy: score("energy", 4.0)?,
        focus: score("focus", 3.0)?,
    })
}

impl Design {
    /// A deterministic composition for the same prompt and decisions.
    pub fn to_recipe(&self, prompt: &str) -> Result<Recipe, String> {
        for (name, value, max) in [
            ("lightness", self.lightness, 4.0),
            ("saturation", self.saturation, 4.0),
            ("contrast", self.contrast, 3.0),
            ("energy", self.energy, 4.0),
            ("focus", self.focus, 3.0),
        ] {
            if !value.is_finite() || !(0.0..=max).contains(&value) {
                return Err(format!("Invalid design {name}"));
            }
        }
        let mut hues = self.hues.clone();
        if matches!(self.layout, Layout::Sky | Layout::Horizon) {
            if let Some(index) = hues.iter().position(|hue| hue == &self.top) {
                hues.swap(0, index);
            }
        }
        let families = hues
            .iter()
            .map(|hue| family(hue).ok_or_else(|| "Unknown design color family".to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut rng = prompt
            .trim()
            .to_lowercase()
            .bytes()
            .fold(2166136261u32, |hash, byte| {
                (hash ^ u32::from(byte)).wrapping_mul(16777619)
            });
        let mut random = || {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            (rng >> 8) as f32 / 16777216.0
        };
        let base = 0.28 + self.lightness * 0.15;
        let contrast = 0.10 + self.contrast / 3.0 * 0.24;
        let scatter_phase = random() * std::f32::consts::TAU;
        let mut points: Vec<Point> = Vec::new();
        let scattered_positions = if self.layout == Layout::Superscatter {
            superscatter_positions(&mut random)
        } else {
            Vec::new()
        };
        let mut assigned_families = Vec::new();
        // Eighteen color centers cover edges as well as the interior.
        for row in 0..6 {
            for column in 0..3 {
                let [x, y] = if self.layout == Layout::Superscatter {
                    scattered_positions[row * 3 + column]
                } else {
                    [
                        (column as f32 / 2.0 + (random() - 0.5) * 0.08).clamp(0.0, 1.0),
                        (row as f32 / 5.0 + (random() - 0.5) * 0.06).clamp(0.0, 1.0),
                    ]
                };
                let distance = (x - 0.5).hypot((y - 0.5) * 1.2);
                let (index, shade) = match self.layout {
                    Layout::Sky => (
                        if y < 0.45 {
                            0
                        } else if y < 0.75 {
                            1
                        } else {
                            2
                        },
                        (y - 0.5) * 2.0,
                    ),
                    Layout::Horizon => (
                        if (y - 0.58).abs() < 0.14 {
                            2
                        } else if y < 0.58 {
                            0
                        } else {
                            1
                        },
                        if (y - 0.58).abs() < 0.14 { 1.0 } else { -0.4 },
                    ),
                    Layout::Glow => (
                        if distance < 0.25 {
                            2
                        } else if distance < 0.45 {
                            1
                        } else {
                            0
                        },
                        1.0 - 3.2 * distance,
                    ),
                    Layout::Scatter => {
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
                    Layout::Superscatter => {
                        let mut counts = [0; 3];
                        let mut penalties = [0.0_f32; 3];
                        for (point, &family) in points.iter().zip(&assigned_families) {
                            counts[family] += 1;
                            let distance = (x - point.position[0]).hypot(y - point.position[1]);
                            penalties[family] += 1.0 / distance.max(0.03).powi(2);
                        }
                        // Primary / secondary / accent quotas retain palette hierarchy.
                        // Normalize neighbor penalties so the primary can recur more often.
                        let targets = [9, 6, 3];
                        for family in 0..3 {
                            penalties[family] /= targets[family] as f32;
                        }
                        let start = (random() * 3.0) as usize;
                        let index = (0..3)
                            .map(|offset| (start + offset) % 3)
                            .filter(|&family| counts[family] < targets[family])
                            .min_by(|&a, &b| penalties[a].total_cmp(&penalties[b]))
                            .unwrap();
                        (index, (random() - 0.5) * 1.2)
                    }
                    Layout::Wash => (
                        if row == 3 && column == 1 {
                            1
                        } else if row == 5 && column == 0 {
                            2
                        } else {
                            0
                        },
                        (random() - 0.5) * 0.4,
                    ),
                };
                assigned_families.push(index);
                let (hue, chroma, preferred_light) = families[index];
                let light = (base + shade * contrast) * 0.6 + preferred_light * 0.4;
                let light = (light + (random() - 0.5) * 0.04).clamp(0.08, 0.97);
                let chroma = chroma * (0.5 + 0.7 * self.saturation / 4.0);
                let angle = (hue + (random() - 0.5) * 12.0).to_radians();
                points.push(Point::new(
                    [x, y],
                    oklab_to_rgb([light, chroma * angle.cos(), chroma * angle.sin()]),
                ));
            }
        }
        let energy = self.energy / 4.0;
        let recipe = Recipe {
            points,
            softness: if self.layout == Layout::Superscatter {
                0.24 + 0.18 * self.focus / 3.0
            } else {
                0.36 + 0.22 * self.focus / 3.0
            },
            distortion: 0.8 + 0.5 * energy,
            swirl: if energy > 0.5 {
                (random() - 0.5) * 0.6 * (energy - 0.5) * 2.0
            } else {
                0.0
            },
            seed: 58,
            motion: 0.3 + 0.6 * energy,
            motion_style: match self.layout {
                Layout::Sky | Layout::Horizon => crate::MotionStyle::Flow,
                Layout::Glow | Layout::Scatter | Layout::Superscatter | Layout::Wash => {
                    crate::MotionStyle::Wander
                }
            },
            warp: Vec::new(),
        };
        recipe.compile().map_err(str::to_string)?;
        Ok(recipe)
    }
}

// Best-candidate sampling spreads points without imposing rows or columns.
fn superscatter_positions(random: &mut impl FnMut() -> f32) -> Vec<[f32; 2]> {
    let mut positions: Vec<[f32; 2]> = Vec::with_capacity(18);
    for _ in 0..18 {
        let mut best = [random(), random()];
        let mut best_distance = -1.0;
        for _ in 0..24 {
            let candidate = [random(), random()];
            let distance = positions
                .iter()
                .map(|point| (candidate[0] - point[0]).powi(2) + (candidate[1] - point[1]).powi(2))
                .fold(f32::INFINITY, f32::min);
            if distance > best_distance {
                best = candidate;
                best_distance = distance;
            }
        }
        positions
            .push(best.map(|coordinate| (coordinate + (random() - 0.5) * 0.02).clamp(0.0, 1.0)));
    }
    positions
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn fixture() -> Value {
        let mut answers = serde_json::Map::new();
        for (name, choice) in [
            ("hue_main", "blue"),
            ("hue_second", "pink"),
            ("hue_accent", "amber"),
            ("hue_top", "blue"),
        ] {
            let mut probabilities: BTreeMap<_, f32> =
                COLORS.iter().map(|(id, _, _, _)| (*id, 0.01)).collect();
            probabilities.insert(choice, 0.8);
            answers.insert(
                name.into(),
                json!({"type":"choice", "choice":choice, "probabilities":probabilities}),
            );
        }
        answers.insert(
            "layout".into(),
            json!({"type":"choice", "choice":"horizon", "probabilities":{"horizon":1.0}}),
        );
        for (name, score) in [
            ("lightness", 3.0),
            ("saturation", 2.0),
            ("contrast", 2.0),
            ("energy", 1.0),
            ("focus", 2.0),
        ] {
            answers.insert(name.into(), json!({"type":"score", "score":score}));
        }
        json!({"answers":answers, "usage":{"cost":0.0001}})
    }

    #[test]
    fn request_uses_bounded_decisions_and_checks_prompts() {
        let request = decision_request("sunset over the sea", DEFAULT_MODEL).unwrap();
        assert_eq!(request["questions"].as_object().unwrap().len(), 10);
        assert_eq!(request["model"], DEFAULT_MODEL);
        assert_eq!(
            request["questions"]["energy"]["criteria"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        assert!(decision_request("  ", DEFAULT_MODEL).is_err());
        assert!(decision_request(&"x".repeat(1001), DEFAULT_MODEL).is_err());
        assert!(request.get("messages").is_none());
    }

    #[test]
    fn every_layout_produces_a_valid_reproducible_recipe() {
        let envelope: Envelope = serde_json::from_value(fixture()).unwrap();
        let mut design = parse_design(&envelope.answers).unwrap();
        for layout in [
            Layout::Sky,
            Layout::Horizon,
            Layout::Glow,
            Layout::Scatter,
            Layout::Superscatter,
            Layout::Wash,
        ] {
            design.layout = layout;
            let a = design.to_recipe("a sunset over the sea").unwrap();
            let b = design.to_recipe("a sunset over the sea").unwrap();
            assert_eq!(a.points.len(), 18);
            assert_eq!(
                serde_json::to_value(a).unwrap(),
                serde_json::to_value(b).unwrap()
            );
        }
        for score in [0.0, 4.0] {
            design.lightness = score;
            design.saturation = score;
            design.energy = score;
            assert!(design.to_recipe("extreme").unwrap().compile().is_ok());
        }
    }

    #[test]
    fn superscatter_decision_keeps_eighteen_separated_points() {
        let mut response = fixture();
        response["answers"]["layout"]["choice"] = json!("superscatter");
        let envelope: Envelope = serde_json::from_value(response).unwrap();
        let design = parse_design(&envelope.answers).unwrap();
        assert_eq!(design.layout, Layout::Superscatter);
        assert!(
            decision_request("confetti", DEFAULT_MODEL).unwrap()["questions"]["layout"]["criteria"]
                .get("superscatter")
                .is_some()
        );
        for prompt in [
            "confetti",
            "scattered lights",
            "mosaic",
            "wildflowers",
            "rainbow pockets",
        ] {
            let recipe = design.to_recipe(prompt).unwrap();
            assert_eq!(recipe.points.len(), 18);
            assert_eq!(recipe.motion_style, crate::MotionStyle::Wander);
            for (index, point) in recipe.points.iter().enumerate() {
                for other in &recipe.points[..index] {
                    assert!(
                        (point.position[0] - other.position[0])
                            .hypot(point.position[1] - other.position[1])
                            > 0.1,
                        "points collide for {prompt}"
                    );
                }
            }
            let mut scatter = design.clone();
            scatter.layout = Layout::Scatter;
            assert!(recipe.softness < scatter.to_recipe(prompt).unwrap().softness);
        }
    }

    #[test]
    fn malformed_decisions_do_not_become_recipes() {
        let original = fixture();
        for value in [json!(-1.0), json!(5.0)] {
            let mut data = original.clone();
            data["answers"]["energy"]["score"] = value;
            let envelope: Envelope = serde_json::from_value(data).unwrap();
            assert!(parse_design(&envelope.answers).is_err());
        }
        let mut data = original.clone();
        data["answers"].as_object_mut().unwrap().remove("layout");
        assert!(parse_design(&serde_json::from_value::<Envelope>(data).unwrap().answers).is_err());
        let mut data = original;
        data["answers"]["hue_main"]["choice"] = json!("unlisted");
        assert!(parse_design(&serde_json::from_value::<Envelope>(data).unwrap().answers).is_err());
    }

    #[test]
    fn dotenv_quotes_and_comments_are_supported_without_exporting_secrets() {
        let config = dotenv_settings(
            "export OPENROUTER_API_KEY='test-key' # comment\nJEV_MODEL=typesafe/jev-1.13 # comment\nUNRELATED=ignored",
        );
        assert_eq!(config["OPENROUTER_API_KEY"], "test-key");
        assert_eq!(config["JEV_MODEL"], DEFAULT_MODEL);
        assert!(!config.contains_key("UNRELATED"));
        assert!(JevClient::new("test\nkey".into(), DEFAULT_MODEL.into()).is_err());
    }

    fn mock(status: u16, body: String) -> (String, std::thread::JoinHandle<Value>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/decisions", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let (header_end, length) = loop {
                let mut buffer = [0; 4096];
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..index]).to_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    break (index + 4, length);
                }
            };
            while bytes.len() < header_end + length {
                let mut buffer = [0; 4096];
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            let request: Value =
                serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
            write!(stream, "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            request
        });
        (url, worker)
    }

    #[test]
    fn http_request_roundtrips_through_local_mock_only() {
        let client = JevClient::new("dummy-test-key".into(), DEFAULT_MODEL.into()).unwrap();
        let (url, worker) = mock(200, fixture().to_string());
        let generated = client.generate_at("sunset", &url).unwrap();
        assert_eq!(generated.design.layout, Layout::Horizon);
        assert_eq!(generated.cost_usd, Some(0.0001));
        assert!(generated.radians_per_second < 0.34);
        let request = worker.join().unwrap();
        assert_eq!(request["state"]["description"], "sunset");
        assert_eq!(request["questions"]["layout"]["type"], "choice");
        assert!(generated.recipe.compile().is_ok());
    }

    #[test]
    fn server_errors_are_clear_and_do_not_echo_response_bodies() {
        let client = JevClient::new("dummy-test-key".into(), DEFAULT_MODEL.into()).unwrap();
        let (url, worker) = mock(401, "sensitive response body".into());
        let error = client.generate_at("sunset", &url).unwrap_err();
        assert!(error.contains("API key"));
        assert!(!error.contains("sensitive"));
        worker.join().unwrap();
    }
}
