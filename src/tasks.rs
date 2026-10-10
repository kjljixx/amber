//! Things to work on, read from a to-do list.
//!
//! A [`Task`] is a title and, where the list has one, the group it sits under.
//! [`parse_markdown`] reads them from markdown lines, and [`todoist`] from a
//! Todoist account. A [`Loader`] fetches either on a worker thread; nothing here
//! draws anything or knows about gradients.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};

#[cfg(feature = "generation")]
pub mod todoist;

/// The biggest list file read, in bytes. A to-do list is never this long.
const MAX_FILE_BYTES: u64 = 1_000_000;

/// When a task is due: a day, and the time of day if it has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Due {
    pub date: NaiveDate,
    pub time: Option<NaiveTime>,
}

impl Due {
    /// In words, from `today`: "today", "tomorrow", "yesterday", the weekday for
    /// the next few days, and otherwise the date, with the year if it is not this
    /// one. A time of day follows, as in "today 3:30pm".
    pub fn words(&self, today: NaiveDate) -> String {
        let days = (self.date - today).num_days();
        let mut text = match days {
            0 => "today".to_owned(),
            1 => "tomorrow".to_owned(),
            -1 => "yesterday".to_owned(),
            2..=6 => self.date.format("%a").to_string(),
            _ if self.date.year() == today.year() => self.date.format("%b %-d").to_string(),
            _ => self.date.format("%b %-d %Y").to_string(),
        };
        if let Some(time) = self.time {
            let minutes = if time.minute() == 0 { "" } else { ":%M" };
            let format = format!(" %-I{minutes}%P");
            text.push_str(&time.format(&format).to_string());
        }
        text
    }
}

/// One thing to do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Task {
    pub title: String,
    /// The heading it sits under, or the project it belongs to.
    pub group: Option<String>,
    pub due: Option<Due>,
    pub labels: Vec<String>,
}

impl Task {
    /// Whether every word of `query` is somewhere in the title, the group or
    /// the labels, ignoring case. An empty query matches everything.
    pub fn matches(&self, query: &str) -> bool {
        let text = format!(
            "{} {} {}",
            self.title,
            self.group.as_deref().unwrap_or_default(),
            self.labels.join(" ")
        )
        .to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .all(|word| text.contains(word))
    }

    /// What to show beside the title, most worth showing first: when it is due,
    /// its labels as `@label`, and its group.
    pub fn details(&self, today: NaiveDate) -> Vec<String> {
        let labels: Vec<String> = self
            .labels
            .iter()
            .map(|label| format!("@{label}"))
            .collect();
        [
            self.due.map(|due| due.words(today)),
            Some(labels.join(" ")).filter(|labels| !labels.is_empty()),
            self.group.clone(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// The unfinished items of a markdown list, in the order they are written.
///
/// An item is a line starting with `-`, `*`, `+` or a number and a dot or
/// bracket. One with a ticked box (`- [x]`) is done and left out; `- [ ]` and a
/// plain bullet both count. A heading names the group of the items after it, up
/// to the next heading. Indented items count as items, and code blocks are
/// skipped.
pub fn parse_markdown(text: &str) -> Vec<Task> {
    let mut tasks = Vec::new();
    let mut group: Option<String> = None;
    let mut fenced = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if line.starts_with('#') {
            let heading = line.trim_start_matches('#').trim().trim_end_matches('#');
            group = Some(heading.trim().to_owned()).filter(|heading| !heading.is_empty());
        } else if let Some(title) = item(line) {
            tasks.push(Task {
                title,
                group: group.clone(),
                ..Task::default()
            });
        }
    }
    tasks
}

/// The title of a list item that is not done, if `line` is one.
fn item(line: &str) -> Option<String> {
    let rest = if let Some(rest) = line
        .strip_prefix(['-', '*', '+'])
        .filter(|rest| rest.starts_with(' '))
    {
        rest
    } else {
        let digits = line.bytes().take_while(u8::is_ascii_digit).count();
        line[digits..]
            .strip_prefix(['.', ')'])
            .filter(|rest| digits > 0 && rest.starts_with(' '))?
    };
    let rest = rest.trim();
    let title = match rest.strip_prefix('[') {
        Some(boxed) => match boxed.split_once(']') {
            Some((" ", title)) => title,
            Some(("x" | "X", _)) => return None,
            _ => rest,
        },
        None => rest,
    };
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

/// Read the unfinished items of the markdown file at `path`, as
/// [`parse_markdown`] does.
pub fn read_markdown(path: &Path) -> Result<Vec<Task>, String> {
    let name = path.display();
    let size = fs::metadata(path)
        .map_err(|error| format!("Can't read {name}: {error}"))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(format!("{name} is too big to be a to-do list"));
    }
    let text = fs::read_to_string(path).map_err(|error| format!("Can't read {name}: {error}"))?;
    Ok(parse_markdown(&text))
}

/// Where tasks come from.
#[derive(Clone, Debug)]
pub enum Source {
    /// The unfinished items of a markdown file, as [`parse_markdown`] reads them.
    Markdown(PathBuf),
    #[cfg(feature = "generation")]
    Todoist(todoist::Todoist),
}

impl Source {
    /// Get the tasks, which may take a while for anything over the network.
    pub fn fetch(&self) -> Result<Vec<Task>, String> {
        match self {
            Self::Markdown(path) => read_markdown(path),
            #[cfg(feature = "generation")]
            Self::Todoist(todoist) => todoist.tasks(),
        }
    }

    /// How long a fetched list is good for. A file is cheap to read again, but a
    /// Todoist account is rate limited and does not change that fast.
    fn fresh_for(&self) -> Duration {
        match self {
            Self::Markdown(_) => Duration::ZERO,
            #[cfg(feature = "generation")]
            Self::Todoist(_) => Duration::from_secs(30),
        }
    }

    /// What to say when the list has nothing in it.
    fn nothing(&self) -> String {
        match self {
            Self::Markdown(path) => format!("No open tasks in {}", path.display()),
            #[cfg(feature = "generation")]
            Self::Todoist(todoist) => match todoist.filter() {
                Some(filter) => format!("Nothing in Todoist for “{filter}”"),
                None => "No active tasks in Todoist".to_owned(),
            },
        }
    }
}

/// Keeps a list of tasks up to date from a [`Source`], fetching on a worker
/// thread so a host's event loop never waits for it.
///
/// Call [`refresh`](Self::refresh) when the list is wanted, and
/// [`poll`](Self::poll) from the loop (every few milliseconds while
/// [`is_pending`](Self::is_pending)) to learn when a fetch has finished. The
/// last list that was fetched stays available while the next is on its way, and
/// after one fails.
pub struct Loader {
    source: Source,
    pending: Option<Receiver<Result<Vec<Task>, String>>>,
    fetched: Option<Instant>,
    tasks: Option<Vec<Task>>,
    /// Why the latest fetch failed, until one works.
    error: Option<String>,
}

impl Loader {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            pending: None,
            fetched: None,
            tasks: None,
            error: None,
        }
    }

    /// Start fetching, unless a fetch is already on its way or the list is
    /// recent enough for this kind of source.
    pub fn refresh(&mut self) {
        let recent = self
            .fetched
            .is_some_and(|at| at.elapsed() < self.source.fresh_for());
        if self.pending.is_some() || recent {
            return;
        }
        let (send, receive) = mpsc::channel();
        let source = self.source.clone();
        let spawned = std::thread::Builder::new()
            .name("tasks".into())
            .spawn(move || {
                let _ = send.send(source.fetch());
            });
        match spawned {
            Ok(_) => self.pending = Some(receive),
            Err(_) => self.error = Some("Could not start loading tasks".into()),
        }
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// `Some` once a fetch has finished: `Ok` when [`tasks`](Self::tasks) has a
    /// new list, or why it failed, in which case the old list is still there.
    pub fn poll(&mut self) -> Option<Result<(), String>> {
        let result = match self.pending.as_ref().map(Receiver::try_recv) {
            Some(Ok(result)) => result,
            Some(Err(TryRecvError::Empty)) | None => return None,
            Some(Err(TryRecvError::Disconnected)) => Err("Loading tasks stopped early".into()),
        };
        self.pending = None;
        self.fetched = Some(Instant::now());
        match result {
            Ok(tasks) => {
                self.tasks = Some(tasks);
                self.error = None;
                Some(Ok(()))
            }
            Err(message) => {
                self.error = Some(message.clone());
                Some(Err(message))
            }
        }
    }

    /// The latest list that was fetched, if any has been.
    pub fn tasks(&self) -> Option<&[Task]> {
        self.tasks.as_deref()
    }

    /// What to show in place of a list that is not there: that it is loading,
    /// why it failed, or that it is empty. `None` when there are tasks to show.
    pub fn message(&self) -> Option<String> {
        match &self.tasks {
            Some(tasks) if tasks.is_empty() => Some(self.source.nothing()),
            Some(_) => None,
            None => Some(
                self.error
                    .clone()
                    .unwrap_or_else(|| "Loading tasks…".into()),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(tasks: &[Task]) -> Vec<&str> {
        tasks.iter().map(|task| task.title.as_str()).collect()
    }

    #[test]
    fn unticked_boxes_and_plain_bullets_are_tasks_and_ticked_ones_are_not() {
        let tasks = parse_markdown(
            "- [ ] write the report\n\
             - [x] book the room\n\
             - [X] send the invite\n\
             * email Sam\n\
             + call the bank\n\
             1. first thing\n\
             2) second thing\n",
        );
        assert_eq!(
            titles(&tasks),
            [
                "write the report",
                "email Sam",
                "call the bank",
                "first thing",
                "second thing"
            ]
        );
    }

    #[test]
    fn things_that_are_not_items_are_left_out() {
        let tasks = parse_markdown(
            "Some notes about the week.\n\
             -not an item\n\
             -\n\
             - [ ]\n\
             - [ ] \n\
             2024 was a year\n\
             1.5 litres\n\
             ```\n\
             - [ ] inside a code block\n\
             ```\n\
             - [ ] after the block\n",
        );
        assert_eq!(titles(&tasks), ["after the block"]);
    }

    #[test]
    fn indented_items_count_and_spacing_is_tidied() {
        let tasks =
            parse_markdown("- [ ] plan\n    - [ ]   draft   the   outline  \n\t- [ ] edit\n");
        assert_eq!(titles(&tasks), ["plan", "draft the outline", "edit"]);
    }

    #[test]
    fn a_heading_is_the_group_of_the_items_after_it() {
        let tasks = parse_markdown(
            "- [ ] loose\n\
             # Work\n\
             - [ ] write the report\n\
             ## Home ##\n\
             - [ ] water the plants\n\
             #\n\
             - [ ] ungrouped again\n",
        );
        let groups: Vec<_> = tasks.iter().map(|task| task.group.as_deref()).collect();
        assert_eq!(
            groups,
            [None, Some("Work"), Some("Home"), None],
            "{tasks:?}"
        );
    }

    #[test]
    fn a_checkbox_that_is_not_one_stays_in_the_title() {
        let tasks = parse_markdown("- [link](https://example.com) to read\n- [maybe] decide\n");
        assert_eq!(
            titles(&tasks),
            ["[link](https://example.com) to read", "[maybe] decide"]
        );
    }

    #[test]
    fn a_query_needs_every_word_somewhere_in_the_title_or_group() {
        let task = Task {
            title: "Write the Report".into(),
            group: Some("Work".into()),
            labels: vec!["deep-focus".into()],
            ..Task::default()
        };
        assert!(task.matches(""));
        assert!(task.matches("   "));
        assert!(task.matches("report"));
        assert!(task.matches("REPORT write"));
        assert!(task.matches("work rep"));
        // Labels count too.
        assert!(task.matches("focus write"));
        assert!(!task.matches("report home"));
        let loose = Task {
            title: "Write".into(),
            ..Task::default()
        };
        assert!(loose.matches("wri"));
        assert!(!loose.matches("work"));
    }

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn a_due_date_is_said_in_the_fewest_words() {
        let today = day("2026-10-09"); // a Friday
        let words = |date: &str, time: Option<(u32, u32)>| {
            Due {
                date: day(date),
                time: time.and_then(|(hour, minute)| NaiveTime::from_hms_opt(hour, minute, 0)),
            }
            .words(today)
        };
        assert_eq!(words("2026-10-09", None), "today");
        assert_eq!(words("2026-10-10", None), "tomorrow");
        assert_eq!(words("2026-10-08", None), "yesterday");
        assert_eq!(words("2026-10-11", None), "Sun");
        assert_eq!(words("2026-10-15", None), "Thu");
        assert_eq!(words("2026-10-16", None), "Oct 16");
        assert_eq!(words("2026-10-06", None), "Oct 6");
        assert_eq!(words("2026-01-02", None), "Jan 2");
        assert_eq!(words("2027-01-02", None), "Jan 2 2027");
        assert_eq!(words("2025-12-31", None), "Dec 31 2025");
        assert_eq!(words("2026-10-09", Some((15, 0))), "today 3pm");
        assert_eq!(words("2026-10-10", Some((9, 30))), "tomorrow 9:30am");
        assert_eq!(words("2026-10-09", Some((0, 5))), "today 12:05am");
        assert_eq!(words("2026-10-09", Some((12, 0))), "today 12pm");
    }

    #[test]
    fn details_run_from_the_due_date_to_the_labels_to_the_group() {
        let today = day("2026-10-09");
        let task = Task {
            title: "Write".into(),
            group: Some("Work".into()),
            due: Some(Due {
                date: today,
                time: None,
            }),
            labels: vec!["deep".into(), "quick".into()],
        };
        assert_eq!(task.details(today), ["today", "@deep @quick", "Work"]);
        let bare = Task {
            title: "Write".into(),
            ..Task::default()
        };
        assert!(bare.details(today).is_empty());
        let only_group = Task {
            group: Some("Home".into()),
            ..bare
        };
        assert_eq!(only_group.details(today), ["Home"]);
    }

    #[test]
    fn a_missing_or_huge_file_is_an_error_saying_which() {
        let missing = std::env::temp_dir().join("amber-no-such-list.md");
        let error = read_markdown(&missing).unwrap_err();
        assert!(error.contains("amber-no-such-list.md"), "{error}");

        let path = std::env::temp_dir().join(format!("amber-list-{}.md", std::process::id()));
        fs::write(&path, "# Work\n- [ ] one\n- [x] two\n").unwrap();
        let read = read_markdown(&path);
        fs::remove_file(&path).unwrap();
        assert_eq!(titles(&read.unwrap()), ["one"]);
    }

    // ---- the loader ----

    fn wait(loader: &mut Loader) -> Result<(), String> {
        for _ in 0..400 {
            if let Some(result) = loader.poll() {
                return result;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the fetch never finished");
    }

    fn list(contents: &str) -> (PathBuf, Loader) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("amber-loader-{}-{n}.md", std::process::id()));
        fs::write(&path, contents).unwrap();
        (path.clone(), Loader::new(Source::Markdown(path)))
    }

    #[test]
    fn before_anything_arrives_the_loader_says_it_is_loading() {
        let (path, mut loader) = list("- [ ] one\n");
        assert_eq!(loader.tasks(), None);
        assert_eq!(loader.message().as_deref(), Some("Loading tasks…"));
        assert!(!loader.is_pending());
        loader.refresh();
        assert!(loader.is_pending());
        // Asking again while it is on its way starts nothing more.
        loader.refresh();
        assert_eq!(wait(&mut loader), Ok(()));
        fs::remove_file(path).unwrap();
        assert!(!loader.is_pending());
        assert_eq!(titles(loader.tasks().unwrap()), ["one"]);
        assert_eq!(loader.message(), None);
        assert_eq!(loader.poll(), None);
    }

    #[test]
    fn a_file_is_read_again_on_every_refresh() {
        let (path, mut loader) = list("- [ ] one\n");
        loader.refresh();
        wait(&mut loader).unwrap();
        fs::write(&path, "- [ ] one\n- [ ] two\n").unwrap();
        loader.refresh();
        wait(&mut loader).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(titles(loader.tasks().unwrap()), ["one", "two"]);
    }

    #[test]
    fn a_list_with_nothing_open_says_where_it_looked() {
        let (path, mut loader) = list("- [x] done\n");
        loader.refresh();
        wait(&mut loader).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(loader.tasks(), Some(&[][..]));
        assert_eq!(
            loader.message(),
            Some(format!("No open tasks in {}", path.display()))
        );
    }

    #[test]
    fn a_failure_is_reported_and_keeps_the_last_list() {
        let (path, mut loader) = list("- [ ] one\n");
        loader.refresh();
        wait(&mut loader).unwrap();
        fs::remove_file(&path).unwrap();
        loader.refresh();
        let error = wait(&mut loader).unwrap_err();
        assert!(error.contains("Can't read"), "{error}");
        assert_eq!(titles(loader.tasks().unwrap()), ["one"]);
        assert_eq!(loader.message(), None);

        // With no list yet, the reason stands in for it.
        let mut never = Loader::new(Source::Markdown(path));
        never.refresh();
        let error = wait(&mut never).unwrap_err();
        assert_eq!(never.message(), Some(error));
    }
}
