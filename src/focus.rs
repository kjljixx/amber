//! Periodic screenshot checks against the task the user selected in the timer.
//! Capture and OpenRouter work happen away from the terminal event loop.

use std::{
    collections::BTreeMap,
    path::Path,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, imageops::FilterType};
use serde::Deserialize;
use serde_json::json;

const ENDPOINT: &str = "https://openrouter.ai/api/alpha/decisions";
const DEFAULT_MODEL: &str = "openai/gpt-6-luna-decisions";
const INITIAL_DELAY: Duration = Duration::from_secs(20);
const INTERVAL: Duration = Duration::from_secs(20);
const OFF_TASK_THRESHOLD: f64 = 0.8;

#[derive(Debug)]
pub enum Event {
    /// The off-task streak reached two or more.
    OffTask,
    /// A permission, capture, or API request needs attention.
    Error(String),
}

type Pending = (String, Receiver<Result<bool, String>>);

/// Checks one explicitly selected timer task at 20-second intervals.
pub struct FocusChecker {
    client: Client,
    task: Option<String>,
    next_check: Option<Instant>,
    pending: Option<Pending>,
    off_task_count: u8,
}

impl FocusChecker {
    pub fn from_env(path: &Path) -> Result<Self, String> {
        Ok(Self {
            client: Client::from_env(path)?,
            task: None,
            next_check: None,
            pending: None,
            off_task_count: 0,
        })
    }

    /// Begin a fresh sequence when the selected task changes. The first image
    /// is captured twenty seconds later, then once every twenty seconds while
    /// that task stays selected.
    pub fn set_task(&mut self, task: Option<String>, now: Instant) {
        let task = task
            .map(|task| task.trim().to_owned())
            .filter(|task| !task.is_empty());
        if self.task == task {
            return;
        }
        self.task = task;
        self.next_check = self.task.as_ref().map(|_| now + INITIAL_DELAY);
        self.off_task_count = 0;
    }

    pub fn time_until_next(&self, now: Instant) -> Option<Duration> {
        if self.pending.is_some() {
            Some(Duration::from_millis(25))
        } else {
            self.next_check.map(|at| at.saturating_duration_since(now))
        }
    }

    /// Start a due check and return any completed decision or failure.
    pub fn poll(&mut self, now: Instant) -> Option<Event> {
        if let Some((task, result)) = self
            .pending
            .as_ref()
            .map(|(task, receiver)| (task.clone(), receiver.try_recv()))
        {
            match result {
                Ok(result) => {
                    self.pending = None;
                    if self.task.as_deref() == Some(task.as_str()) {
                        match result {
                            Ok(off_task) => {
                                if off_task {
                                    self.off_task_count = self.off_task_count.saturating_add(1);
                                    if self.off_task_count >= 2 {
                                        return Some(Event::OffTask);
                                    }
                                } else {
                                    self.off_task_count = 0;
                                }
                            }
                            Err(message) => return Some(Event::Error(message)),
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.pending = None;
                    if self.task.as_deref() == Some(task.as_str()) {
                        return Some(Event::Error("Focus check stopped unexpectedly".into()));
                    }
                }
                Err(TryRecvError::Empty) => {}
            }
        }

        if self.pending.is_none()
            && self.task.is_some()
            && self.next_check.is_some_and(|at| now >= at)
        {
            let task = self.task.clone().unwrap_or_default();
            let client = self.client.clone();
            let (send, receive) = mpsc::channel();
            self.next_check = Some(now + INTERVAL);
            self.pending = Some((task.clone(), receive));
            let _ = std::thread::Builder::new()
                .name("focus-check".into())
                .spawn(move || {
                    let result = capture_jpeg().and_then(|image| client.is_off_task(&task, &image));
                    let _ = send.send(result);
                });
        }
        None
    }
}

#[derive(Clone)]
struct Client {
    key: String,
    model: String,
    agent: ureq::Agent,
}

impl Client {
    fn from_env(path: &Path) -> Result<Self, String> {
        let file = match std::fs::read_to_string(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(_) => return Err("Could not read local .env configuration".into()),
        };
        let settings =
            crate::generation::dotenv_settings(&file, &["OPENROUTER_API_KEY", "FOCUS_MODEL"]);
        let get = |name: &str| {
            std::env::var(name)
                .ok()
                .or_else(|| settings.get(name).cloned())
        };
        let key = get("OPENROUTER_API_KEY").unwrap_or_default();
        if key.trim().is_empty() || key.chars().any(char::is_control) {
            return Err("Configure OPENROUTER_API_KEY in the environment or .env".into());
        }
        let model = get("FOCUS_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into());
        if model.trim().is_empty() {
            return Err("FOCUS_MODEL must not be empty".into());
        }
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
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

    fn is_off_task(&self, task: &str, jpeg: &[u8]) -> Result<bool, String> {
        let data_url = format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg));
        let request = json!({
            "model": self.model,
            "state": [
                format!("The user's current task is: {task}. Decide whether the visible computer activity is unrelated to making progress on this task. Count ordinary task work, research, communication required by the task, and brief transitions as on task. If the screenshot is ambiguous, judge it on task."),
                {"type": "image_url", "image_url": {"url": data_url}}
            ],
            "questions": {
                "off_task": {
                    "type": "noul",
                    "instructions": "Is the visible activity off task?",
                    "criteria": {
                        "true": "The visible activity is clearly unrelated to progress on the stated task.",
                        "false": "The visible activity is related, ambiguous, or not enough to conclude it is off task."
                    }
                }
            }
        });
        let mut response = self
            .agent
            .post(ENDPOINT)
            .header("Authorization", format!("Bearer {}", self.key))
            .header("Content-Type", "application/json")
            .header("X-Title", "Amber Focus Check")
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
            .limit(128 * 1024)
            .read_json()
            .map_err(|_| "OpenRouter returned an invalid focus decision".to_string())?;
        let probability = envelope
            .answers
            .get("off_task")
            .filter(|answer| answer.is_noul())
            .map(|answer| answer.noul)
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
            .ok_or_else(|| "OpenRouter omitted the off-task probability".to_string())?;
        Ok(probability >= OFF_TASK_THRESHOLD)
    }
}

#[derive(Deserialize)]
struct Envelope {
    answers: BTreeMap<String, Answer>,
}

#[derive(Deserialize)]
struct Answer {
    #[serde(rename = "type")]
    answer_type: String,
    noul: f64,
}

fn capture_jpeg() -> Result<Vec<u8>, String> {
    use xcap::Monitor;

    let monitor = Monitor::all()
        .map_err(|_| "Could not enumerate screens for capture".to_string())?
        .into_iter()
        .find(|monitor| monitor.is_primary().unwrap_or(false))
        .ok_or_else(|| "Could not find the primary screen".to_string())?;
    let image = monitor
        .capture_image()
        .map_err(|_| "Screen capture failed; check screen recording permission".to_string())?;
    let width = image.width().min(960);
    let height = ((image.height() as u64 * width as u64) / image.width().max(1) as u64) as u32;
    let reduced = image::imageops::resize(&image, width, height.max(1), FilterType::Triangle);
    let mut output = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(reduced)
        .write_to(&mut output, ImageFormat::Jpeg)
        .map_err(|_| "Could not encode screen capture".to_string())?;
    Ok(output.into_inner())
}

// Keep this import visible to the compiler: validating the returned primitive
// prevents accidental parsing of a non-noul answer as a probability.
impl Answer {
    fn is_noul(&self) -> bool {
        self.answer_type == "noul"
    }
}
