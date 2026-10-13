//! One log file for each part of the program, in a directory for each run.
//!
//! The terminal belongs to the interface, so nothing is logged to it. Call
//! [`init`] once at the start of a binary. Modules then log with the ordinary
//! `tracing` macros and their own module path; [`Channel::of`] says which file
//! that goes to:
//!
//! | File | Parts |
//! |---|---|
//! | `gradient.log` | `ambient`, `phrases`, `generation`, `scene`, `recipe`, `view`, `jev`, `amber` |
//! | `tasks.log` | `tasks`, `picker` |
//! | `audio.log` | `audio` |
//! | `focus.log` | `focus`, `focus_overlay` |
//! | `timer.log` | `timer`, `alarm` |
//!
//! Files are made when something is first written to them. The level is `info`;
//! `RUST_LOG` changes it, for example `RUST_LOG=amber::focus=debug`. At `debug`
//! the focus checker also keeps each screenshot and model response, see
//! [`attach`].
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use chrono::Utc;
use tracing_subscriber::{
    EnvFilter, Layer,
    filter::{FilterExt, filter_fn},
    fmt,
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

const BASE_DIRECTORY: &str = "logs";
const DEFAULT_LEVEL: &str = "info";

static RUN_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Gradient,
    Tasks,
    Audio,
    Focus,
    Timer,
}

impl Channel {
    pub const ALL: [Channel; 5] = [
        Channel::Gradient,
        Channel::Tasks,
        Channel::Audio,
        Channel::Focus,
        Channel::Timer,
    ];

    pub fn file_name(self) -> &'static str {
        match self {
            Channel::Gradient => "gradient.log",
            Channel::Tasks => "tasks.log",
            Channel::Audio => "audio.log",
            Channel::Focus => "focus.log",
            Channel::Timer => "timer.log",
        }
    }

    /// The channel for a `tracing` target, which is a module path such as
    /// `amber::focus` (library) or `timer::alarm` (a module of the timer).
    /// Targets of other crates have none and are not written.
    pub fn of(target: &str) -> Option<Self> {
        let mut parts = target.split("::");
        match (parts.next(), parts.next()) {
            (Some("amber"), Some("tasks")) | (Some("timer"), Some("picker")) => {
                Some(Channel::Tasks)
            }
            (Some("amber"), Some("audio")) => Some(Channel::Audio),
            (Some("amber"), Some("focus")) | (Some("timer"), Some("focus_overlay")) => {
                Some(Channel::Focus)
            }
            (Some("timer"), _) => Some(Channel::Timer),
            (Some("amber" | "jev"), _) => Some(Channel::Gradient),
            _ => None,
        }
    }
}

/// Opens its file on the first write, so a run that never logs to a channel
/// leaves no file for it.
struct LazyFile {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

#[derive(Clone)]
struct Sink(Arc<LazyFile>);

impl Write for Sink {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut file = self
            .0
            .file
            .lock()
            .map_err(|_| io::Error::other("log lock"))?;
        if file.is_none() {
            if let Some(directory) = self.0.path.parent() {
                fs::create_dir_all(directory)?;
            }
            *file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.0.path)?,
            );
        }
        file.as_mut().map_or(Ok(0), |file| file.write(buffer))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The filter for a level setting: `info` everywhere, then what `RUST_LOG` says.
fn filter(setting: Option<&str>) -> Result<EnvFilter, String> {
    let directives = match setting.map(str::trim).filter(|setting| !setting.is_empty()) {
        Some(setting) => format!("{DEFAULT_LEVEL},{setting}"),
        None => DEFAULT_LEVEL.to_owned(),
    };
    EnvFilter::try_new(&directives).map_err(|error| format!("RUST_LOG is not valid: {error}"))
}

/// Start logging into a new directory under `logs/` and return it. Call once.
pub fn init() -> io::Result<PathBuf> {
    let setting = std::env::var("RUST_LOG").ok();
    let directory = Path::new(BASE_DIRECTORY).join(format!(
        "run-{}-{}",
        Utc::now().format("%Y%m%dT%H%M%S"),
        std::process::id()
    ));
    let layers = Channel::ALL
        .map(|channel| {
            let sink = Sink(Arc::new(LazyFile {
                path: directory.join(channel.file_name()),
                file: Mutex::new(None),
            }));
            let filter = filter(setting.as_deref())
                .map_err(io::Error::other)?
                .and(filter_fn(move |metadata| {
                    Channel::of(metadata.target()) == Some(channel)
                }));
            Ok(fmt::layer()
                .with_ansi(false)
                .with_thread_names(true)
                .with_writer(move || sink.clone())
                .with_filter(filter)
                .boxed())
        })
        .into_iter()
        .collect::<io::Result<Vec<_>>>()?;
    tracing_subscriber::registry()
        .with(layers)
        .try_init()
        .map_err(io::Error::other)?;
    let _ = RUN_DIRECTORY.set(directory.clone());
    Ok(directory)
}

/// Save a file that is too big for a log line, such as a screenshot, in
/// `subdirectory` of the run's directory, and return where it went. Callers
/// log the path.
pub fn attach(subdirectory: &str, name: &str, contents: &[u8]) -> io::Result<PathBuf> {
    let directory = RUN_DIRECTORY
        .get()
        .ok_or_else(|| io::Error::other("logging was not started"))?
        .join(subdirectory);
    fs::create_dir_all(&directory)?;
    let path = directory.join(name);
    fs::write(&path, contents)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_go_to_the_channel_of_their_part() {
        for (target, channel) in [
            ("amber::ambient", Channel::Gradient),
            ("amber::generation::client", Channel::Gradient),
            ("amber::view", Channel::Gradient),
            ("amber", Channel::Gradient),
            ("jev", Channel::Gradient),
            ("amber::tasks::todoist", Channel::Tasks),
            ("timer::picker", Channel::Tasks),
            ("amber::audio", Channel::Audio),
            ("amber::focus", Channel::Focus),
            ("timer::focus_overlay", Channel::Focus),
            ("timer", Channel::Timer),
            ("timer::alarm", Channel::Timer),
        ] {
            assert_eq!(Channel::of(target), Some(channel), "{target}");
        }
        assert_eq!(Channel::of("ureq::agent"), None);
    }

    #[test]
    fn the_level_is_info_unless_the_setting_says_more() {
        assert!(filter(None).is_ok());
        assert!(filter(Some("amber::focus=debug")).is_ok());
        assert!(filter(Some("  ")).is_ok());
        assert!(filter(Some("amber::focus=loud")).is_err());
    }
}
