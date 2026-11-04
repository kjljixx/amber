//! The settings file, `amber.json`, and the schedule it holds.
//!
//! ```json
//! { "scheduled_work": ["09:00", "14:30"] }
//! ```
//!
//! Each time is `HH:MM` on a 24 hour clock, in local time. A [`Schedule`] says
//! when one of them has just passed; it knows nothing about what to do then.
use std::{fs, io::ErrorKind, path::Path};

use chrono::{Duration, NaiveDateTime, NaiveTime};
use serde::Deserialize;
use tracing::{info, warn};

/// Where the settings are read from unless `--config` says otherwise.
pub const DEFAULT_PATH: &str = "amber.json";

/// A scheduled time that passed longer ago than this is skipped, such as one
/// that went by while the machine slept.
const GRACE: Duration = Duration::minutes(5);

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    #[serde(default)]
    scheduled_work: Vec<String>,
}

/// What the settings file asks for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub scheduled_work: Vec<NaiveTime>,
}

impl Config {
    /// Read the settings at `path`. A file that is not there is the same as
    /// an empty one when `required` is false.
    pub fn load(path: &Path, required: bool) -> Result<Self, String> {
        let name = path.display();
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound && !required => {
                info!(path = %name, "no settings file; scheduled work is off");
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("Can't read {name}: {error}")),
        };
        let config = Self::parse(&text).map_err(|error| format!("{name}: {error}"))?;
        info!(path = %name, scheduled_work = ?config.scheduled_work, "settings loaded");
        Ok(config)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let settings: Settings = serde_json::from_str(text).map_err(|error| error.to_string())?;
        let mut scheduled_work = settings
            .scheduled_work
            .iter()
            .map(|time| {
                NaiveTime::parse_from_str(time, "%H:%M").map_err(|_| {
                    format!("scheduled_work time “{time}” is not HH:MM, such as “09:30”")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        scheduled_work.sort();
        scheduled_work.dedup();
        Ok(Self { scheduled_work })
    }
}

/// Times of day that come round every day. Ask [`due`](Self::due) as often as
/// is convenient: each time is reported once, on the first ask after it passes.
#[derive(Debug)]
pub struct Schedule {
    times: Vec<NaiveTime>,
    asked: NaiveDateTime,
}

impl Schedule {
    /// `now` is when the schedule starts; times before it do not count.
    pub fn new(times: Vec<NaiveTime>, now: NaiveDateTime) -> Self {
        Self { times, asked: now }
    }

    pub fn len(&self) -> usize {
        self.times.len()
    }

    /// The scheduled time that has passed since the last ask, the latest if
    /// there are several, or `None`.
    pub fn due(&mut self, now: NaiveDateTime) -> Option<NaiveTime> {
        let since = self.asked;
        self.asked = now.max(since);
        let mut passed: Vec<NaiveDateTime> = [since.date(), now.date()]
            .into_iter()
            .flat_map(|date| self.times.iter().map(move |time| date.and_time(*time)))
            .filter(|at| *at > since && *at <= now)
            .collect();
        passed.sort();
        passed.dedup();
        let latest = passed.pop()?;
        if now - latest > GRACE {
            warn!(%latest, %now, "scheduled time went by too long ago; skipped");
            return None;
        }
        Some(latest.time())
    }

    /// How long until the next scheduled time, or `None` with no times.
    pub fn until_next(&self, now: NaiveDateTime) -> Option<std::time::Duration> {
        [now.date(), now.date() + Duration::days(1)]
            .into_iter()
            .flat_map(|date| self.times.iter().map(move |time| date.and_time(*time)))
            .filter(|at| *at > now)
            .min()
            .and_then(|at| (at - now).to_std().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn time(text: &str) -> NaiveTime {
        NaiveTime::parse_from_str(text, "%H:%M").unwrap()
    }

    #[test]
    fn times_are_read_sorted_and_without_repeats() {
        let config = Config::parse(r#"{"scheduled_work": ["14:30", "09:00", "9:00"]}"#).unwrap();
        assert_eq!(config.scheduled_work, [time("09:00"), time("14:30")]);
        assert_eq!(Config::parse("{}").unwrap(), Config::default());
    }

    #[test]
    fn a_bad_time_or_unknown_setting_says_what_is_wrong() {
        let error = Config::parse(r#"{"scheduled_work": ["25:00"]}"#).unwrap_err();
        assert!(error.contains("25:00"), "{error}");
        let error = Config::parse(r#"{"scheduled_wrk": []}"#).unwrap_err();
        assert!(error.contains("scheduled_wrk"), "{error}");
        assert!(Config::parse("not json").is_err());
    }

    #[test]
    fn a_missing_file_is_only_an_error_when_it_was_asked_for() {
        let missing = std::env::temp_dir().join("amber-no-such-config.json");
        assert_eq!(Config::load(&missing, false), Ok(Config::default()));
        let error = Config::load(&missing, true).unwrap_err();
        assert!(error.contains("amber-no-such-config.json"), "{error}");
    }

    #[test]
    fn a_time_is_reported_once_when_it_passes() {
        let mut schedule = Schedule::new(vec![time("09:00")], at("2026-11-03 08:59:00"));
        assert_eq!(schedule.due(at("2026-11-03 08:59:30")), None);
        assert_eq!(schedule.due(at("2026-11-03 09:00:01")), Some(time("09:00")));
        assert_eq!(schedule.due(at("2026-11-03 09:00:02")), None);
        assert_eq!(schedule.due(at("2026-11-03 17:00:00")), None);
    }

    #[test]
    fn it_comes_round_again_the_next_day() {
        let mut schedule = Schedule::new(vec![time("09:00")], at("2026-11-03 09:30:00"));
        assert_eq!(schedule.due(at("2026-11-03 23:59:59")), None);
        assert_eq!(schedule.due(at("2026-11-04 00:00:01")), None);
        assert_eq!(schedule.due(at("2026-11-04 09:00:00")), Some(time("09:00")));
    }

    #[test]
    fn a_time_equal_to_the_start_does_not_count_but_one_across_midnight_does() {
        let mut schedule = Schedule::new(vec![time("00:00")], at("2026-11-03 00:00:00"));
        assert_eq!(schedule.due(at("2026-11-03 00:00:01")), None);
        assert_eq!(schedule.due(at("2026-11-03 23:59:59")), None);
        assert_eq!(schedule.due(at("2026-11-04 00:00:01")), Some(time("00:00")));
    }

    #[test]
    fn a_time_that_went_by_while_asleep_is_skipped() {
        let mut schedule = Schedule::new(vec![time("09:00")], at("2026-11-03 08:00:00"));
        assert_eq!(schedule.due(at("2026-11-03 11:00:00")), None);
        assert_eq!(schedule.due(at("2026-11-03 11:00:01")), None);
    }

    #[test]
    fn the_wait_is_until_the_next_time_today_or_tomorrow() {
        let schedule = Schedule::new(
            vec![time("09:00"), time("14:00")],
            at("2026-11-03 00:00:00"),
        );
        let wait = |now| schedule.until_next(at(now)).unwrap().as_secs();
        assert_eq!(wait("2026-11-03 08:00:00"), 3600);
        assert_eq!(wait("2026-11-03 09:00:00"), 5 * 3600);
        assert_eq!(wait("2026-11-03 15:00:00"), 18 * 3600);
        let none = Schedule::new(Vec::new(), at("2026-11-03 00:00:00"));
        assert_eq!(none.len(), 0);
        assert_eq!(none.until_next(at("2026-11-03 08:00:00")), None);
    }
}
