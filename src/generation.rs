//! Jev makes bounded design decisions; local code builds a validated recipe.
//! No network requests are made by the renderer or by `GradientView`.
use std::{collections::BTreeMap, path::Path, time::Duration};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    Recipe,
    scene::{COLORS, ColorSpec, Colors, Composition, GridRule, LightsAt, Style, family},
};

const ENDPOINT: &str = "https://openrouter.ai/api/alpha/decisions";
pub const DEFAULT_MODEL: &str = "typesafe/jev-1.13";

#[derive(Clone, Debug)]
pub struct GeneratedGradient {
    pub recipe: Recipe,
    pub radians_per_second: f64,
    /// What Jev decided is in the picture; it can be saved and edited.
    pub composition: Composition,
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
        let composition = parse_composition(&envelope.answers)?;
        let recipe = composition_recipe(&composition, prompt)?;
        Ok(GeneratedGradient {
            recipe,
            radians_per_second: f64::from(0.2 + 0.9 * composition.style.energy / 4.0) * 0.3,
            composition,
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

/// The bounded questions about what is in the picture. Each is scored against
/// its labels, so its largest value is one less than the number of labels; the
/// matching `Composition::*_MAX` constants must agree (a test checks this).
const LEVELS: &[(&str, &str, &[&str])] = &[
    (
        "horizon_height",
        "Where does the sky meet the ground or water?",
        &[
            "Very high in the picture",
            "High",
            "Middle",
            "Low",
            "Very low in the picture",
        ],
    ),
    (
        "land",
        "What is the ground like?",
        &[
            "No ground at all",
            "Flat ground or water",
            "Gentle hills",
            "Rolling hills",
            "Jagged mountains",
        ],
    ),
    (
        "features",
        "How many tall upright things (trees, towers, poles, buildings) stand in the scene?",
        &["None", "A few", "Several", "Many", "A dense crowd"],
    ),
    (
        "feature_width",
        "How thick are those upright things?",
        &["Very thin", "Thin", "Wide", "Massive"],
    ),
    (
        "rays",
        "Are there slanted beams or shafts of light?",
        &["None", "A few", "Several", "Many"],
    ),
    (
        "lights",
        "How many small lights (windows, stars, lanterns, sparks) are there?",
        &["None", "A few", "Some", "Many", "A great many"],
    ),
    (
        "light_size",
        "How big are those small lights?",
        &["Pinpoints", "Small", "Medium", "Large orbs"],
    ),
    (
        "light_glow",
        "How much does each small light spill its glow into the space around it?",
        &["Not at all", "A little", "Noticeably", "Radiant"],
    ),
    (
        "glow",
        "Is there one big source of light, like a sun, moon, or lamp?",
        &["None", "Faint", "Bright", "Blazing"],
    ),
    (
        "glow_height",
        "How high is that big source of light?",
        &["On the horizon", "Low", "High", "Overhead"],
    ),
    (
        "clouds",
        "Are there clouds or streaks of mist across the sky?",
        &["None", "A few wisps", "Several", "A sky full of clouds"],
    ),
    (
        "atmosphere",
        "How hazy is the air?",
        &["Crystal clear", "Slight haze", "Hazy", "Thick fog"],
    ),
    (
        "concrete",
        "Is this description a physical place or scene you could see, or a feeling or idea?",
        &[
            "Purely a feeling or idea",
            "Mostly a feeling",
            "Partly a place",
            "A concrete place or scene",
        ],
    ),
    (
        "lightness",
        "How light should this gradient feel?",
        &["Very dark", "Dark", "Medium", "Light", "Very light"],
    ),
    (
        "saturation",
        "How saturated should the colors feel?",
        &[
            "Almost neutral",
            "Muted",
            "Moderate",
            "Vivid",
            "Intensely saturated",
        ],
    ),
    (
        "contrast",
        "How much contrast should the color regions have?",
        &["Very little", "Gentle", "Strong", "Dramatic"],
    ),
    (
        "energy",
        "How energetic should the animated movement feel?",
        &[
            "Still and calm",
            "Slow",
            "Moderate",
            "Lively",
            "Very energetic",
        ],
    ),
    (
        "focus",
        "How soft should the edges of the color regions be?",
        &["Crisp", "Slightly soft", "Soft", "Very diffuse"],
    ),
];

/// The five colors, as (decision name, what it asks).
const COLOR_QUESTIONS: &[(&str, &str)] = &[
    (
        "color_sky",
        "Which color family best represents the dominant color of this description, as seen in the sky or upper part of the scene?",
    ),
    (
        "color_horizon",
        "Which supporting color family best complements this description, for the glow at the horizon and the haze in the distance?",
    ),
    (
        "color_ground",
        "Which supporting color family best complements this description, for the ground, water, or lower part of the scene?",
    ),
    (
        "color_feature",
        "Which color family best represents tall upright things such as trees, towers, or buildings?",
    ),
    (
        "color_light",
        "Which color family best represents light itself: lamps, windows, sun, or glow?",
    ),
];

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
    let mut questions = serde_json::Map::new();
    for (name, instructions) in COLOR_QUESTIONS {
        questions.insert(
            (*name).into(),
            json!({"type":"choice", "instructions":instructions, "criteria":palette}),
        );
    }
    questions.insert(
        "layout".into(),
        json!({"type":"choice", "instructions":"If this description is mostly a feeling or idea rather than a place, how should its colors be arranged?", "criteria": {
            "sky":"Broad horizontal color regions changing from top to bottom",
            "horizon":"A bright seam separating upper and lower color regions",
            "glow":"A luminous central region with a darker outer region",
            "scatter":"Broad, softly connected colored regions distributed across the image",
            "superscatter":"Many smaller irregular color pockets interspersed throughout the image, like confetti, mosaic colors, or scattered lights",
            "wash":"Mostly one color with gentle, subtle variations"
        }}),
    );
    questions.insert(
        "lights_at".into(),
        json!({"type":"choice", "instructions":"Where are the small lights?", "criteria": {
            "sky":"Up in the sky, like stars",
            "horizon":"Along the horizon, like a skyline, shoreline, or village",
            "ground":"Low in the picture, on the ground or reflected in water",
            "everywhere":"Scattered all over the picture"
        }}),
    );
    for (name, instructions, labels) in LEVELS {
        questions.insert(
            (*name).into(),
            json!({"type":"score", "instructions":instructions, "criteria":labels}),
        );
    }
    Ok(json!({
        "model": model,
        "state": {"description":prompt, "task":"Imagine this description as a scene of soft, blurry shapes and glowing colors, like an out-of-focus photograph. Describe what is in it."},
        "questions": questions
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

/// The color family Jev is most confident about for one color question,
/// skipping any in `excluded`.
fn color(
    answers: &BTreeMap<String, Answer>,
    name: &str,
    excluded: &[&ColorSpec],
) -> Result<ColorSpec, String> {
    let Some(Answer::Choice {
        choice,
        probabilities,
    }) = answers.get(name)
    else {
        return Err(format!("Jev response is missing the {name} decision"));
    };
    if family(choice).is_none()
        || probabilities
            .values()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(format!("Jev returned an invalid {name} decision"));
    }
    probabilities
        .iter()
        .filter(|(key, weight)| {
            family(key).is_some()
                && **weight > 0.0
                && !excluded
                    .iter()
                    .any(|taken| matches!(taken, ColorSpec::Family(name) if name == *key))
        })
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(key, _)| ColorSpec::Family(key.clone()))
        .ok_or_else(|| format!("Jev returned no usable {name} colors"))
}

fn parse_composition(answers: &BTreeMap<String, Answer>) -> Result<Composition, String> {
    let level = |name: &str| -> Result<f32, String> {
        let (_, _, labels) = LEVELS
            .iter()
            .find(|(id, ..)| *id == name)
            .expect("every level is asked");
        let max = (labels.len() - 1) as f32;
        match answers.get(name) {
            Some(Answer::Score { score }) if score.is_finite() && (0.0..=max).contains(score) => {
                Ok(*score)
            }
            _ => Err(format!(
                "Jev returned a missing or out-of-range {name} score"
            )),
        }
    };
    let Some(Answer::Choice { choice, .. }) = answers.get("lights_at") else {
        return Err("Jev response is missing the lights_at decision".into());
    };
    let lights_at: LightsAt = serde_json::from_value(json!(choice))
        .map_err(|_| "Jev returned an unknown lights_at decision".to_string())?;
    let Some(Answer::Choice { choice, .. }) = answers.get("layout") else {
        return Err("Jev response is missing the layout decision".into());
    };
    let layout: GridRule = serde_json::from_value(json!(choice))
        .map_err(|_| "Jev returned an unknown layout".to_string())?;
    // The three scene colors must differ, or a gradient has nothing to blend;
    // each takes its best family that the earlier ones did not. Features and
    // light are free to match anything (white light over a white wall is fine).
    let sky = color(answers, "color_sky", &[])?;
    let horizon = color(answers, "color_horizon", &[&sky])?;
    let ground = color(answers, "color_ground", &[&sky, &horizon])?;
    Ok(Composition {
        colors: Colors {
            feature: color(answers, "color_feature", &[])?,
            light: color(answers, "color_light", &[])?,
            sky,
            horizon,
            ground,
        },
        horizon_height: level("horizon_height")?,
        land: level("land")?,
        features: level("features")?,
        feature_width: level("feature_width")?,
        rays: level("rays")?,
        lights: level("lights")?,
        light_size: level("light_size")?,
        light_glow: level("light_glow")?,
        lights_at,
        glow: level("glow")?,
        glow_height: level("glow_height")?,
        clouds: level("clouds")?,
        atmosphere: level("atmosphere")?,
        concrete: level("concrete")?,
        layout,
        style: Style {
            lightness: level("lightness")?,
            saturation: level("saturation")?,
            contrast: level("contrast")?,
            energy: level("energy")?,
            focus: level("focus")?,
        },
    })
}

/// A deterministic recipe for the same prompt and decisions: the prompt seeds
/// the random placement.
pub fn composition_recipe(composition: &Composition, prompt: &str) -> Result<Recipe, String> {
    let variation = prompt
        .trim()
        .to_lowercase()
        .bytes()
        .fold(2166136261u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16777619)
        });
    composition.to_scene(variation)?.compile()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn fixture() -> Value {
        let mut answers = serde_json::Map::new();
        for (name, choice) in [
            ("color_sky", "blue"),
            ("color_horizon", "pink"),
            ("color_ground", "brown"),
            ("color_feature", "indigo"),
            ("color_light", "amber"),
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
            json!({"type":"choice", "choice":"scatter", "probabilities":{"scatter":1.0}}),
        );
        answers.insert(
            "lights_at".into(),
            json!({"type":"choice", "choice":"horizon", "probabilities":{"horizon":1.0}}),
        );
        for (name, _, labels) in LEVELS {
            // Roughly the middle of each scale.
            let score = (labels.len() - 1) as f32 / 2.0;
            answers.insert((*name).into(), json!({"type":"score", "score":score}));
        }
        json!({"answers":answers, "usage":{"cost":0.0001}})
    }

    fn composition_from(value: Value) -> Result<Composition, String> {
        parse_composition(&serde_json::from_value::<Envelope>(value).unwrap().answers)
    }

    #[test]
    fn request_asks_only_bounded_questions_that_match_the_composition() {
        let request = decision_request("sunset over the sea", DEFAULT_MODEL).unwrap();
        let questions = request["questions"].as_object().unwrap();
        assert_eq!(questions.len(), COLOR_QUESTIONS.len() + 2 + LEVELS.len());
        assert_eq!(request["model"], DEFAULT_MODEL);
        for (name, ..) in LEVELS {
            assert_eq!(questions[*name]["type"], "score");
        }
        assert_eq!(questions["energy"]["criteria"].as_array().unwrap().len(), 5);
        assert!(decision_request("  ", DEFAULT_MODEL).is_err());
        assert!(decision_request(&"x".repeat(1001), DEFAULT_MODEL).is_err());
        assert!(request.get("messages").is_none());

        // Each question's top label is the composition's largest accepted value.
        let max =
            |name: &str| LEVELS.iter().find(|(id, ..)| *id == name).unwrap().2.len() as f32 - 1.0;
        assert_eq!(max("horizon_height"), Composition::HORIZON_HEIGHT_MAX);
        assert_eq!(max("land"), Composition::LAND_MAX);
        assert_eq!(max("features"), Composition::FEATURES_MAX);
        assert_eq!(max("feature_width"), Composition::FEATURE_WIDTH_MAX);
        assert_eq!(max("rays"), Composition::RAYS_MAX);
        assert_eq!(max("lights"), Composition::LIGHTS_MAX);
        assert_eq!(max("light_size"), Composition::LIGHT_SIZE_MAX);
        assert_eq!(max("light_glow"), Composition::LIGHT_GLOW_MAX);
        assert_eq!(max("glow"), Composition::GLOW_MAX);
        assert_eq!(max("glow_height"), Composition::GLOW_HEIGHT_MAX);
        assert_eq!(max("clouds"), Composition::CLOUDS_MAX);
        assert_eq!(max("atmosphere"), Composition::ATMOSPHERE_MAX);
        assert_eq!(max("concrete"), Composition::CONCRETE_MAX);
    }

    #[test]
    fn composition_recipes_are_valid_and_reproducible() {
        let composition = composition_from(fixture()).unwrap();
        let a = composition_recipe(&composition, "a sunset over the sea").unwrap();
        let b = composition_recipe(&composition, "A sunset over the sea ").unwrap();
        let c = composition_recipe(&composition, "something else").unwrap();
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&b).unwrap()
        );
        assert_ne!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&c).unwrap()
        );
        assert!(a.compile().is_ok());
    }

    #[test]
    fn every_extreme_answer_yields_a_valid_recipe() {
        // Every question at its lowest and highest score, together.
        for high in [false, true] {
            let mut data = fixture();
            for (name, _, labels) in LEVELS {
                let score = if high { (labels.len() - 1) as f32 } else { 0.0 };
                data["answers"][*name]["score"] = json!(score);
            }
            for lights_at in ["sky", "horizon", "ground", "everywhere"] {
                data["answers"]["lights_at"]["choice"] = json!(lights_at);
                let composition = composition_from(data.clone()).unwrap();
                let recipe = composition_recipe(&composition, "extreme").unwrap();
                assert!(recipe.compile().is_ok());
                assert!(recipe.points.len() <= 256);
            }
        }
    }

    #[test]
    fn sky_horizon_and_ground_take_different_families() {
        // Jev's single favorite for every question is cream; the later
        // questions fall back to their next best.
        let mut data = fixture();
        for (name, runner_up) in [
            ("color_sky", "cream"),
            ("color_horizon", "yellow"),
            ("color_ground", "amber"),
            ("color_feature", "brown"),
            ("color_light", "white"),
        ] {
            let mut probabilities: BTreeMap<_, f32> =
                COLORS.iter().map(|(id, _, _, _)| (*id, 0.0)).collect();
            probabilities.insert("cream", 0.6);
            probabilities.insert(runner_up, 0.3);
            data["answers"][name] =
                json!({"type":"choice", "choice":"cream", "probabilities":probabilities});
        }
        let colors = composition_from(data).unwrap().colors;
        let family = |color: &ColorSpec| String::from(color.clone());
        assert_eq!(family(&colors.sky), "cream");
        assert_eq!(family(&colors.horizon), "yellow");
        // Ground: cream and yellow are taken, so its own runner-up wins.
        assert_eq!(family(&colors.ground), "amber");
        // Features and light may repeat.
        assert_eq!(family(&colors.feature), "cream");
        assert_eq!(family(&colors.light), "cream");
    }

    #[test]
    fn malformed_decisions_do_not_become_compositions() {
        let original = fixture();
        for value in [json!(-1.0), json!(5.0)] {
            let mut data = original.clone();
            data["answers"]["energy"]["score"] = value;
            assert!(composition_from(data).is_err());
        }
        let mut data = original.clone();
        data["answers"]["glow"]["score"] = json!(4.0);
        assert!(composition_from(data).is_err());
        for missing in ["lights_at", "layout", "color_sky", "land"] {
            let mut data = original.clone();
            data["answers"].as_object_mut().unwrap().remove(missing);
            assert!(composition_from(data).is_err(), "{missing}");
        }
        let mut data = original.clone();
        data["answers"]["color_light"]["choice"] = json!("unlisted");
        assert!(composition_from(data).is_err());
        let mut data = original;
        data["answers"]["lights_at"]["choice"] = json!("nowhere");
        assert!(composition_from(data).is_err());
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
        assert_eq!(generated.composition.lights_at, LightsAt::Horizon);
        assert_eq!(generated.cost_usd, Some(0.0001));
        assert!(generated.radians_per_second < 0.34);
        let request = worker.join().unwrap();
        assert_eq!(request["state"]["description"], "sunset");
        assert_eq!(request["questions"]["lights_at"]["type"], "choice");
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
