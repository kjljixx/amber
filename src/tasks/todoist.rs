//! Tasks from [Todoist](https://todoist.com), read with its v1 API and a
//! personal API token (Todoist's settings, under Integrations, then Developer).
//!
//! The token is only ever sent to `api.todoist.com`, and only read: nothing here
//! changes anything in the account. [`Todoist::tasks`] makes the requests, which
//! block, so a host runs it on a worker thread, as [`super::Loader`] does.
use std::{
    cmp::Reverse,
    collections::HashMap,
    fmt,
    path::Path,
    time::{Duration, Instant},
};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use tracing::{debug, warn};

use super::{Due, Task};
use crate::generation::dotenv_settings;

const BASE: &str = "https://api.todoist.com/api/v1";
const TIMEOUT: Duration = Duration::from_secs(15);
/// The most Todoist returns for a page, and how many pages are read: two
/// thousand tasks is far more than anyone chooses between.
const PAGE_SIZE: &str = "200";
const MAX_PAGES: usize = 10;
const MAX_BODY_BYTES: u64 = 2_000_000;

type Get<'a> = &'a dyn Fn(&str, &[(&str, &str)]) -> Result<String, String>;

/// A Todoist account to read tasks from, and which of them: all the active ones,
/// or those a [Todoist
/// filter](https://todoist.com/help/articles/introduction-to-filters-V98wIH)
/// matches, such as `today | overdue` or `#Work & p1`.
#[derive(Clone)]
pub struct Todoist {
    token: String,
    filter: Option<String>,
}

// The token stays out of anything printed.
impl fmt::Debug for Todoist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Todoist")
            .field("filter", &self.filter)
            .finish_non_exhaustive()
    }
}

impl Todoist {
    pub fn new(token: &str, filter: Option<&str>) -> Result<Self, String> {
        let token = token.trim();
        if token.is_empty() || token.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err("Todoist needs an API token: set TODOIST_API_TOKEN".into());
        }
        Ok(Self {
            token: token.to_owned(),
            filter: filter
                .map(str::trim)
                .filter(|filter| !filter.is_empty())
                .map(str::to_owned),
        })
    }

    /// The token from `TODOIST_API_TOKEN` in the environment or the dotenv file
    /// at `path`, the environment taking precedence, as for the Jev key.
    pub fn from_env(path: &Path, filter: Option<&str>) -> Result<Self, String> {
        let file = match std::fs::read_to_string(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(_) => return Err("Could not read local .env configuration".into()),
        };
        let token = std::env::var("TODOIST_API_TOKEN")
            .ok()
            .filter(|token| !token.trim().is_empty())
            .or_else(|| dotenv_settings(&file, &["TODOIST_API_TOKEN"]).remove("TODOIST_API_TOKEN"));
        Self::new(&token.unwrap_or_default(), filter)
    }

    /// The filter choosing which tasks are read, if not all of them.
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    /// The active tasks, or those the filter matches: the soonest due first, those
    /// with no date last, and on a day the most urgent first (Todoist's p1 before
    /// p4). Each is under the name of its project, if that can be had.
    pub fn tasks(&self) -> Result<Vec<Task>, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .into();
        let get = |path: &str, query: &[(&str, &str)]| {
            let mut request = agent
                .get(format!("{BASE}/{path}"))
                .header("Authorization", format!("Bearer {}", self.token));
            for (name, value) in query {
                request = request.query(name, value);
            }
            let started = Instant::now();
            let mut response = request.call().map_err(|error| {
                warn!(path, %error, "Todoist request failed");
                "Could not reach Todoist (connection or timeout)".to_string()
            })?;
            let status = response.status().as_u16();
            debug!(
                path,
                status,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "Todoist responded"
            );
            match status {
                200..=299 => {}
                400 => return Err("Todoist did not understand the filter".into()),
                401 | 403 => return Err("Todoist rejected the API token".into()),
                429 => return Err("Todoist rate limit reached; try again shortly".into()),
                code => return Err(format!("Todoist returned HTTP {code}")),
            }
            response
                .body_mut()
                .with_config()
                .limit(MAX_BODY_BYTES)
                .read_to_string()
                .map_err(|_| "Could not read Todoist's response".to_string())
        };
        self.tasks_with(&get)
    }

    /// [`tasks`](Self::tasks), with `get` making each request: a path under the
    /// API and its query, answering with the body.
    fn tasks_with(&self, get: Get) -> Result<Vec<Task>, String> {
        let found: Vec<Found> = match &self.filter {
            Some(filter) => pages(get, "tasks/filter", &[("query", filter)])?,
            None => pages(get, "tasks", &[])?,
        };
        // Names are only a nicety, so not having them is not worth failing for.
        let projects: HashMap<String, String> = pages::<Project>(get, "projects", &[])
            .unwrap_or_default()
            .into_iter()
            .map(|project| (project.id, project.name))
            .collect();
        Ok(tasks_from(found, &projects))
    }
}

/// Everything under `path`, a page at a time, following Todoist's cursors.
fn pages<T: DeserializeOwned>(
    get: Get,
    path: &str,
    query: &[(&str, &str)],
) -> Result<Vec<T>, String> {
    #[derive(Deserialize)]
    struct Page<T> {
        #[serde(default = "Vec::new")]
        results: Vec<T>,
        next_cursor: Option<String>,
    }

    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let body = {
            let mut query = query.to_vec();
            query.push(("limit", PAGE_SIZE));
            if let Some(cursor) = &cursor {
                query.push(("cursor", cursor));
            }
            get(path, &query)?
        };
        let page: Page<T> = serde_json::from_str(&body)
            .map_err(|_| "Todoist returned something unexpected".to_string())?;
        items.extend(page.results);
        cursor = page.next_cursor.filter(|cursor| !cursor.is_empty());
        if cursor.is_none() {
            break;
        }
    }
    Ok(items)
}

/// A task as Todoist describes it, with only what is used here.
#[derive(Deserialize)]
struct Found {
    content: String,
    project_id: Option<String>,
    /// 1 for normal up to 4 for what the app calls p1.
    #[serde(default = "normal")]
    priority: u8,
    /// An object with the date, or the date and time, it is due.
    due: Option<Value>,
    labels: Option<Vec<String>>,
}

fn normal() -> u8 {
    1
}

#[derive(Deserialize)]
struct Project {
    id: String,
    name: String,
}

/// When something is due, from the `date` Todoist gives: a day, a day and a
/// time as written on the clock, or a day and time in UTC, which is turned
/// into this machine's local time.
fn parse_due(date: &str) -> Option<Due> {
    let date = date.trim();
    if let Ok(day) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        return Some(Due {
            date: day,
            time: None,
        });
    }
    let at = DateTime::parse_from_rfc3339(date)
        .map(|at| at.with_timezone(&Local).naive_local())
        .or_else(|_| NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M:%S%.f"))
        .ok()?;
    Some(Due {
        date: at.date(),
        time: Some(at.time()),
    })
}

fn tasks_from(found: Vec<Found>, projects: &HashMap<String, String>) -> Vec<Task> {
    let mut found: Vec<(u8, Task)> = found
        .into_iter()
        .filter_map(|task| {
            let title = clean(&task.content);
            if title.is_empty() {
                return None;
            }
            let due = task
                .due
                .as_ref()
                .and_then(|due| parse_due(due.get("date")?.as_str()?));
            let group = task
                .project_id
                .and_then(|id| projects.get(&id))
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty());
            let labels = task
                .labels
                .unwrap_or_default()
                .into_iter()
                .map(|label| label.trim().to_owned())
                .filter(|label| !label.is_empty())
                .collect();
            Some((
                task.priority,
                Task {
                    title,
                    group,
                    due,
                    labels,
                },
            ))
        })
        .collect();
    // By day, then priority, then time of day, a day with no time first. Stable,
    // so Todoist's own order breaks any tie, and no date is after every date.
    found.sort_by_key(|(priority, task)| {
        (
            task.due.map_or(NaiveDate::MAX, |due| due.date),
            Reverse(*priority),
            task.due.and_then(|due| due.time).unwrap_or(NaiveTime::MIN),
        )
    });
    found.into_iter().map(|(_, task)| task).collect()
}

/// A task's text with its markdown links replaced by what they say, as in
/// `[the report](https://example.com)` and spacing tidied. Todoist keeps links
/// in the title when a task is added from a web page.
fn clean(content: &str) -> String {
    let mut text = String::new();
    let mut rest = content;
    while let Some(open) = rest.find('[') {
        let link = rest[open + 1..]
            .split_once("](")
            .and_then(|(label, after)| {
                let (url, after) = after.split_once(')')?;
                (!label.contains(['[', ']', '\n']) && !url.contains(['(', ' ', '\n']))
                    .then_some((label, after))
            });
        match link {
            Some((label, after)) => {
                text.push_str(&rest[..open]);
                text.push_str(label);
                rest = after;
            }
            None => {
                text.push_str(&rest[..=open]);
                rest = &rest[open + 1..];
            }
        }
    }
    text.push_str(rest);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    const TASKS: &str = r#"{"results":[
        {"id":"1","content":"Write the report","project_id":"p-work","priority":1,
         "labels":["deep","quick"],
         "due":{"date":"2026-10-09","string":"today","is_recurring":false}},
        {"id":"2","content":"Reply to [Sam](https://mail.example.com/1)  about the invoice",
         "project_id":"p-work","priority":4,"due":{"date":"2026-10-09T15:00:00"}},
        {"id":"3","content":"Water the plants","project_id":"p-home","priority":4,
         "due":{"date":"2026-10-08"}},
        {"id":"4","content":"Call the dentist","project_id":"p-gone","priority":1,"due":null},
        {"id":"5","content":"Renew the car insurance","project_id":"p-home","priority":2,
         "due":{"date":"2026-10-09"}},
        {"id":"6","content":"   ","project_id":"p-home","priority":4,"due":null},
        {"id":"7","content":"Undated and urgent","project_id":null,"priority":4}
    ],"next_cursor":null}"#;

    const PROJECTS: &str = r#"{"results":[
        {"id":"p-work","name":"Work","color":"blue"},
        {"id":"p-home","name":"Home","color":"red"}],"next_cursor":null}"#;

    /// A path and the query it was asked with.
    type Asked = (String, Vec<(String, String)>);

    /// Answers like Todoist, and remembers what it was asked.
    struct Fake {
        asked: RefCell<Vec<Asked>>,
        tasks: Vec<&'static str>,
        projects: Result<&'static str, &'static str>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                asked: RefCell::new(Vec::new()),
                tasks: vec![TASKS],
                projects: Ok(PROJECTS),
            }
        }

        fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<String, String> {
            let query: Vec<_> = query
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect();
            let cursor = query.iter().find(|(name, _)| name == "cursor");
            let asked = self
                .asked
                .borrow()
                .iter()
                .filter(|(p, _)| p == path)
                .count();
            self.asked
                .borrow_mut()
                .push((path.to_owned(), query.clone()));
            match path {
                "tasks" | "tasks/filter" => {
                    assert_eq!(cursor.is_some(), asked > 0, "{query:?}");
                    Ok(self.tasks[asked.min(self.tasks.len() - 1)].to_owned())
                }
                "projects" => self.projects.map(str::to_owned).map_err(str::to_owned),
                other => panic!("asked for {other}"),
            }
        }
    }

    fn todoist() -> Todoist {
        Todoist::new("secret-token", None).unwrap()
    }

    fn titles(tasks: &[Task]) -> Vec<(&str, Option<&str>)> {
        tasks
            .iter()
            .map(|task| (task.title.as_str(), task.group.as_deref()))
            .collect()
    }

    #[test]
    fn tasks_come_soonest_due_first_then_most_urgent_with_their_project_names() {
        let fake = Fake::new();
        let tasks = todoist().tasks_with(&|p, q| fake.get(p, q)).unwrap();
        assert_eq!(
            titles(&tasks),
            [
                // Yesterday's.
                ("Water the plants", Some("Home")),
                // Today's, p4 then p2 then p1.
                ("Reply to Sam about the invoice", Some("Work")),
                ("Renew the car insurance", Some("Home")),
                ("Write the report", Some("Work")),
                // With no date, p4 then p1; a project that is not known is no group.
                ("Undated and urgent", None),
                ("Call the dentist", None),
            ]
        );
    }

    #[test]
    fn tasks_carry_their_due_dates_and_labels() {
        let fake = Fake::new();
        let tasks = todoist().tasks_with(&|p, q| fake.get(p, q)).unwrap();
        let day = |text: &str| NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap();
        let by_title = |title: &str| tasks.iter().find(|task| task.title == title).unwrap();
        assert_eq!(
            by_title("Water the plants").due,
            Some(Due {
                date: day("2026-10-08"),
                time: None
            })
        );
        assert_eq!(
            by_title("Reply to Sam about the invoice").due,
            Some(Due {
                date: day("2026-10-09"),
                time: NaiveTime::from_hms_opt(15, 0, 0)
            })
        );
        assert_eq!(by_title("Call the dentist").due, None);
        assert_eq!(by_title("Write the report").labels, ["deep", "quick"]);
        assert!(by_title("Water the plants").labels.is_empty());
    }

    #[test]
    fn due_dates_are_read_as_days_clock_times_or_utc_times() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        assert_eq!(
            parse_due("2026-10-09"),
            Some(Due {
                date: day,
                time: None
            })
        );
        assert_eq!(
            parse_due("2026-10-09T09:30:00"),
            Some(Due {
                date: day,
                time: NaiveTime::from_hms_opt(9, 30, 0)
            })
        );
        assert_eq!(
            parse_due("2026-10-09T09:30:00.250000"),
            Some(Due {
                date: day,
                time: NaiveTime::from_hms_milli_opt(9, 30, 0, 250)
            })
        );
        // UTC times are shown on this machine's clock, whichever it is: the
        // same instant, a day either side at most.
        let utc = parse_due("2026-10-09T12:00:00Z").unwrap();
        assert!(utc.time.is_some());
        assert!((utc.date - day).num_days().abs() <= 1);
        assert_eq!(parse_due("someday"), None);
        assert_eq!(parse_due(""), None);
    }

    #[test]
    fn it_asks_for_every_active_task_unless_given_a_filter() {
        let fake = Fake::new();
        todoist().tasks_with(&|p, q| fake.get(p, q)).unwrap();
        let asked = fake.asked.borrow();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0].0, "tasks");
        assert_eq!(asked[0].1, [("limit".into(), "200".into())]);
        assert_eq!(asked[1].0, "projects");
        assert_eq!(todoist().filter(), None);
        assert_eq!(Todoist::new("t", Some("  ")).unwrap().filter(), None);
    }

    #[test]
    fn a_filter_is_sent_trimmed_to_the_filter_endpoint() {
        let fake = Fake::new();
        let todoist = Todoist::new("secret-token", Some("  #Work & p1 ")).unwrap();
        todoist.tasks_with(&|p, q| fake.get(p, q)).unwrap();
        let asked = fake.asked.borrow();
        assert_eq!(asked[0].0, "tasks/filter");
        assert_eq!(
            asked[0].1,
            [
                ("query".into(), "#Work & p1".into()),
                ("limit".into(), "200".into())
            ]
        );
        assert_eq!(todoist.filter(), Some("#Work & p1"));
    }

    #[test]
    fn more_pages_are_followed_until_the_cursor_runs_out() {
        let mut fake = Fake::new();
        fake.tasks = vec![
            r#"{"results":[{"content":"one","priority":1}],"next_cursor":"abc"}"#,
            r#"{"results":[{"content":"two","priority":1}],"next_cursor":"def"}"#,
            r#"{"results":[{"content":"three","priority":1}],"next_cursor":null}"#,
        ];
        let tasks = todoist().tasks_with(&|p, q| fake.get(p, q)).unwrap();
        assert_eq!(
            tasks.iter().map(|task| &*task.title).collect::<Vec<_>>(),
            ["one", "two", "three"]
        );
        let asked = fake.asked.borrow();
        let cursors: Vec<_> = asked
            .iter()
            .filter(|(path, _)| path == "tasks")
            .map(|(_, query)| {
                query
                    .iter()
                    .find(|(name, _)| name == "cursor")
                    .map(|(_, c)| c.as_str())
            })
            .collect();
        assert_eq!(cursors, [None, Some("abc"), Some("def")]);
    }

    #[test]
    fn a_list_that_never_ends_is_cut_off() {
        let get = |path: &str, _: &[(&str, &str)]| {
            Ok(if path == "projects" {
                r#"{"results":[]}"#.to_owned()
            } else {
                r#"{"results":[{"content":"again"}],"next_cursor":"more"}"#.to_owned()
            })
        };
        assert_eq!(todoist().tasks_with(&get).unwrap().len(), MAX_PAGES);
    }

    #[test]
    fn failing_to_get_project_names_does_not_lose_the_tasks() {
        let mut fake = Fake::new();
        fake.projects = Err("Todoist returned HTTP 500");
        let tasks = todoist().tasks_with(&|p, q| fake.get(p, q)).unwrap();
        assert_eq!(tasks.len(), 6);
        assert!(tasks.iter().all(|task| task.group.is_none()));
    }

    #[test]
    fn failing_to_get_tasks_says_why() {
        let rejected =
            |_: &str, _: &[(&str, &str)]| Err("Todoist rejected the API token".to_string());
        assert_eq!(
            todoist().tasks_with(&rejected).unwrap_err(),
            "Todoist rejected the API token"
        );
        let nonsense = |_: &str, _: &[(&str, &str)]| Ok("<html>".to_string());
        assert_eq!(
            todoist().tasks_with(&nonsense).unwrap_err(),
            "Todoist returned something unexpected"
        );
        let empty =
            |_: &str, _: &[(&str, &str)]| Ok(r#"{"results":[],"next_cursor":null}"#.to_string());
        assert_eq!(todoist().tasks_with(&empty).unwrap(), []);
    }

    #[test]
    fn links_in_a_title_become_their_words() {
        assert_eq!(clean("plain"), "plain");
        assert_eq!(clean("[x](a(b)"), "[x](a(b)");
        assert_eq!(
            clean("read [the report](https://example.com) today"),
            "read the report today"
        );
        assert_eq!(clean("[a](u) and [b](v)"), "a and b");
        // Brackets that are not a link stay.
        assert_eq!(clean("[maybe] decide"), "[maybe] decide");
        assert_eq!(clean("[unclosed](x and more"), "[unclosed](x and more");
        assert_eq!(clean("a [b] [c](d e) f"), "a [b] [c](d e) f");
        assert_eq!(clean("  spaced \n out  "), "spaced out");
    }

    #[test]
    fn a_token_is_needed_and_stays_out_of_what_is_printed() {
        assert!(Todoist::new("", None).is_err());
        assert!(Todoist::new("  ", None).is_err());
        assert!(Todoist::new("two words", None).is_err());
        assert!(Todoist::new("line\nbreak", None).is_err());
        let printed = format!("{:?}", todoist());
        assert!(!printed.contains("secret-token"), "{printed}");
        let filtered = format!(
            "{:?}",
            Todoist::new("secret-token", Some("today | overdue")).unwrap()
        );
        assert!(filtered.contains("today | overdue"), "{filtered}");
        assert!(!filtered.contains("secret-token"), "{filtered}");
    }

    #[test]
    fn the_token_comes_from_the_dotenv_file_when_the_environment_has_none() {
        // The environment may hold a real token, which wins; only test the file
        // when it does not.
        if std::env::var("TODOIST_API_TOKEN").is_ok_and(|token| !token.trim().is_empty()) {
            return;
        }
        let path = std::env::temp_dir().join(format!("amber-todoist-{}.env", std::process::id()));
        std::fs::write(
            &path,
            "OPENROUTER_API_KEY=other\nexport TODOIST_API_TOKEN=\"from-file\" # note\n",
        )
        .unwrap();
        let from_file = Todoist::from_env(&path, None);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(from_file.unwrap().token, "from-file");

        let missing = std::env::temp_dir().join("amber-todoist-no-such.env");
        assert!(
            Todoist::from_env(&missing, None)
                .unwrap_err()
                .contains("TODOIST_API_TOKEN")
        );
    }
}
