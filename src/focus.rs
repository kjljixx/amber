//! Periodic screenshot checks against the task the user selected in the timer.
//! Capture and OpenRouter work happen away from the terminal event loop.

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, imageops::FilterType};
use serde::Deserialize;
use serde_json::json;
use tracing::{Level, debug, enabled, info, warn};

const ENDPOINT: &str = "https://openrouter.ai/api/alpha/decisions";
const DEFAULT_MODEL: &str = "openai/gpt-6-luna-decisions";
const INITIAL_DELAY: Duration = Duration::from_secs(20);
const INTERVAL: Duration = Duration::from_secs(20);
const OVERLAY_INTERVAL: Duration = Duration::from_secs(5);
const OFF_TASK_THRESHOLD: f64 = 0.5;

pub const OFF_TASK_TITLE: &str = "Let's get back on track";
pub const OFF_TASK_BODY: &str = "Don't get distracted. Stay focused on your task";

#[derive(Debug)]
pub enum Event {
    /// The off-task streak reached two or more.
    OffTask,
    /// The latest completed decision says the user is back on task.
    OnTask,
    /// A permission, capture, or API request needs attention.
    Error(String),
}

#[derive(Clone, Copy)]
struct Decision {
    probability: f64,
    off_task: bool,
}

type Pending = (
    u64,
    String,
    Receiver<Result<Decision, String>>,
    Arc<AtomicBool>,
);

/// Checks one explicitly selected timer task, more frequently after an
/// off-task decision until an on-task decision arrives.
pub struct FocusChecker {
    client: Client,
    task: Option<String>,
    next_check: Option<Instant>,
    pending: Option<Pending>,
    off_task_count: u8,
    interval: Duration,
    monitor_active: bool,
    next_check_id: u64,
}

impl FocusChecker {
    pub fn from_env(path: &Path) -> Result<Self, String> {
        let client = Client::from_env(path)?;
        info!(
            model = client.model,
            initial_delay_secs = INITIAL_DELAY.as_secs(),
            normal_interval_secs = INTERVAL.as_secs(),
            off_task_interval_secs = OVERLAY_INTERVAL.as_secs(),
            threshold = OFF_TASK_THRESHOLD,
            "focus checker configured"
        );
        Ok(Self {
            client,
            task: None,
            next_check: None,
            pending: None,
            off_task_count: 0,
            interval: INTERVAL,
            monitor_active: true,
            next_check_id: 1,
        })
    }

    pub fn task(&self) -> Option<&str> {
        self.task.as_deref()
    }

    /// Begin a fresh sequence when the selected task changes. The first image
    /// follows the current cadence, then checks repeat at that interval.
    pub fn set_task(&mut self, task: Option<String>, now: Instant) {
        let task = task
            .map(|task| task.trim().to_owned())
            .filter(|task| !task.is_empty());
        if self.task == task {
            return;
        }
        self.discard_pending("task_changed");
        info!(task = ?task, "task selected");
        self.task = task;
        self.interval = INTERVAL;
        self.next_check =
            (self.monitor_active && self.task.is_some()).then_some(now + INITIAL_DELAY);
        self.off_task_count = 0;
        self.log_state("task_changed");
    }

    /// Suspend captures during breaks. A pending result belongs to the previous
    /// work period, so discard it and start fresh when work resumes.
    pub fn set_work_active(&mut self, active: bool, now: Instant) {
        if self.monitor_active == active {
            return;
        }
        self.monitor_active = active;
        self.discard_pending(if active {
            "work_resumed"
        } else {
            "break_started"
        });
        info!(active, "monitoring changed");
        self.off_task_count = 0;
        self.interval = INTERVAL;
        self.next_check = (active && self.task.is_some()).then_some(now + INITIAL_DELAY);
        self.log_state("work_active_changed");
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
        if !self.monitor_active {
            return None;
        }

        if let Some((check_id, task, result)) = self
            .pending
            .as_ref()
            .map(|(check_id, task, receiver, _)| (*check_id, task.clone(), receiver.try_recv()))
        {
            match result {
                Ok(result) => {
                    self.pending = None;
                    if self.task.as_deref() == Some(task.as_str()) {
                        match result {
                            Ok(decision) => {
                                info!(
                                    check_id,
                                    task = %task,
                                    probability_off_task = decision.probability,
                                    off_task = decision.off_task,
                                    "decision received"
                                );
                                if decision.off_task {
                                    self.off_task_count = self.off_task_count.saturating_add(1);
                                    self.interval = OVERLAY_INTERVAL;
                                    self.next_check = Some(now + self.interval);
                                    self.log_state("off_task");
                                    if self.off_task_count >= 2 {
                                        return Some(Event::OffTask);
                                    }
                                } else {
                                    self.off_task_count = 0;
                                    self.interval = INTERVAL;
                                    self.next_check = Some(now + self.interval);
                                    self.log_state("on_task");
                                    return Some(Event::OnTask);
                                }
                            }
                            Err(message) => {
                                warn!(check_id, task = %task, error = %message, "check failed");
                                self.log_state("check_error");
                                return Some(Event::Error(message));
                            }
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.pending = None;
                    if self.task.as_deref() == Some(task.as_str()) {
                        warn!(check_id, task = %task, "check worker disconnected");
                        self.log_state("worker_disconnected");
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
            let check_id = self.next_check_id;
            self.next_check_id = self.next_check_id.saturating_add(1);
            let (send, receive) = mpsc::channel();
            let cancelled = Arc::new(AtomicBool::new(false));
            let worker_cancelled = Arc::clone(&cancelled);
            self.next_check = Some(now + self.interval);
            self.pending = Some((check_id, task.clone(), receive, cancelled));
            info!(
                check_id,
                task = %task,
                streak = self.off_task_count,
                interval_secs = self.interval.as_secs(),
                "check started"
            );
            let spawn = std::thread::Builder::new()
                .name("focus-check".into())
                .spawn(move || {
                    if worker_cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    let capture_started = Instant::now();
                    let result = match capture_jpeg() {
                        Ok(image) => {
                            info!(
                                check_id,
                                bytes = image.len(),
                                elapsed_ms = capture_started.elapsed().as_millis() as u64,
                                "screenshot captured"
                            );
                            keep(check_id, &format!("check-{check_id:04}.jpg"), &image);
                            if worker_cancelled.load(Ordering::Relaxed) {
                                info!(check_id, "check cancelled before the model request");
                                return;
                            }
                            client.is_off_task(&task, &image, check_id)
                        }
                        Err(message) => {
                            warn!(
                                check_id,
                                error = %message,
                                elapsed_ms = capture_started.elapsed().as_millis() as u64,
                                "screenshot failed"
                            );
                            Err(message)
                        }
                    };
                    if !worker_cancelled.load(Ordering::Relaxed) {
                        let _ = send.send(result);
                    }
                });
            if let Err(error) = spawn {
                self.pending = None;
                warn!(check_id, %error, "check worker could not start");
                return Some(Event::Error("Could not start focus check".into()));
            }
        }
        None
    }

    fn discard_pending(&mut self, reason: &str) {
        if let Some((check_id, _, _, cancelled)) = self.pending.take() {
            cancelled.store(true, Ordering::Relaxed);
            info!(check_id, reason, "check cancelled");
        }
    }

    fn log_state(&self, reason: &str) {
        debug!(
            reason,
            task = ?self.task,
            monitor_active = self.monitor_active,
            streak = self.off_task_count,
            interval_secs = self.interval.as_secs(),
            next_check_id = self.next_check_id,
            "focus state"
        );
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

    fn is_off_task(&self, task: &str, jpeg: &[u8], check_id: u64) -> Result<Decision, String> {
        let data_url = format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg));
        let prompt = format!(
            "The user's current task is: {task}. Decide whether the visible computer activity is unrelated to making progress on this task. Count ordinary task work, research, communication required by the task, and brief transitions as on task. If an 'Let's get back on track' overlay is visible, ignore it and judge the underlying window instead. If the screenshot is ambiguous, judge it on task."
        );
        let request = json!({
            "model": self.model,
            "state": [
                prompt,
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
        let model_started = Instant::now();
        info!(check_id, model = self.model, "model request started");
        debug!(check_id, prompt = %prompt, "model prompt");
        let response = self
            .agent
            .post(ENDPOINT)
            .header("Authorization", format!("Bearer {}", self.key))
            .header("Content-Type", "application/json")
            .header("X-Title", "Amber Focus Check")
            .send_json(&request);
        let mut response = match response {
            Ok(response) => response,
            Err(_) => {
                warn!(
                    check_id,
                    elapsed_ms = model_started.elapsed().as_millis() as u64,
                    "model request failed: connection or timeout"
                );
                return Err("Could not reach OpenRouter (connection or timeout)".into());
            }
        };
        let status = response.status().as_u16();
        let raw = match response
            .body_mut()
            .with_config()
            .limit(128 * 1024)
            .read_to_string()
        {
            Ok(raw) => raw,
            Err(_) => {
                warn!(
                    check_id,
                    status,
                    elapsed_ms = model_started.elapsed().as_millis() as u64,
                    "model response could not be read"
                );
                return Err("OpenRouter returned an invalid focus decision".into());
            }
        };
        info!(
            check_id,
            status,
            bytes = raw.len(),
            elapsed_ms = model_started.elapsed().as_millis() as u64,
            "model response received"
        );
        keep(
            check_id,
            &format!("check-{check_id:04}-response.json"),
            raw.as_bytes(),
        );
        match status {
            200..=299 => {}
            401 | 403 => return Err("OpenRouter rejected the API key or model access".into()),
            402 => return Err("OpenRouter account needs credits".into()),
            429 => return Err("OpenRouter rate limit reached; try again shortly".into()),
            code => return Err(format!("OpenRouter returned HTTP {code}")),
        }
        let envelope: Envelope = serde_json::from_str(&raw)
            .map_err(|_| "OpenRouter returned an invalid focus decision".to_string())?;
        let probability = envelope
            .answers
            .get("off_task")
            .filter(|answer| answer.is_noul())
            .map(|answer| answer.noul)
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
            .ok_or_else(|| "OpenRouter omitted the off-task probability".to_string())?;
        let off_task = probability >= OFF_TASK_THRESHOLD;
        debug!(
            check_id,
            probability_off_task = probability,
            threshold = OFF_TASK_THRESHOLD,
            off_task,
            "model output parsed"
        );
        Ok(Decision {
            probability,
            off_task,
        })
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

/// At `debug`, keep a screenshot or model response next to the logs.
fn keep(check_id: u64, name: &str, contents: &[u8]) {
    if !enabled!(Level::DEBUG) {
        return;
    }
    match crate::log::attach("focus", name, contents) {
        Ok(path) => {
            debug!(check_id, path = %path.display(), bytes = contents.len(), "attachment saved")
        }
        Err(error) => warn!(check_id, %error, "attachment not saved"),
    }
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
