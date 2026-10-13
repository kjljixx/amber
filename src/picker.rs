//! The list of tasks to choose from, shown below the box where the task is
//! typed. What has been typed filters the list, and Enter takes the highlighted
//! row: a task, or what was typed when it is not on the list.
//!
//! The nearest match sits just below the box and the rest stack downward, so
//! moving down the list is moving down the screen.
use amber::{blend_samples, tasks::Task};
use chrono::{Local, NaiveDate};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use tracing::{debug, info};
use unicode_width::UnicodeWidthStr;

use super::{BLACK, fit, put_text};

/// Most rows shown at once.
const MAX_ROWS: usize = 8;
/// The solid background of a row, and of the highlighted row.
const ROW_BACKGROUND: [u8; 3] = BLACK;
const HIGHLIGHT_BACKGROUND: [u8; 3] = [48, 48, 48];
const TEXT: Color = Color::Rgb(215, 215, 215);
const DIM: Color = Color::Rgb(165, 165, 165);

/// One line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    label: String,
    /// What goes at the right, most worth showing first: not all of it is shown
    /// when the row is narrow.
    details: Vec<String>,
    /// What choosing the row makes the task.
    choice: String,
}

pub struct Picker {
    tasks: Vec<Task>,
    /// Why there are no tasks, when there are none.
    message: Option<String>,
    /// The highlighted row, counting down from the one nearest the box.
    selected: usize,
    /// The first row in view, counting the same way.
    offset: usize,
    /// The day due dates are said relative to.
    today: NaiveDate,
}

impl Picker {
    pub fn new(tasks: Vec<Task>, message: Option<String>) -> Self {
        debug!(tasks = tasks.len(), message = ?message, "picker opened");
        Self {
            tasks,
            message,
            selected: 0,
            offset: 0,
            today: Local::now().date_naive(),
        }
    }

    /// Replace the tasks, keeping the highlight where it is: more may arrive
    /// while the list is open.
    pub fn set(&mut self, tasks: Vec<Task>, message: Option<String>) {
        self.tasks = tasks;
        self.message = message;
    }

    /// The tasks that match `query`, then, when something has been typed, a row
    /// for using it as it is.
    fn rows(&self, query: &str) -> Vec<Row> {
        let typed = query.trim();
        let mut rows: Vec<Row> = self
            .tasks
            .iter()
            .filter(|task| task.matches(typed))
            .map(|task| Row {
                label: task.title.clone(),
                details: task.details(self.today),
                choice: task.title.clone(),
            })
            .collect();
        if !typed.is_empty() {
            rows.push(Row {
                label: format!("Use “{typed}”"),
                details: Vec::new(),
                choice: typed.to_owned(),
            });
        }
        rows
    }

    /// The text the highlighted row makes the task, if there is a row.
    pub fn choose(&self, query: &str) -> Option<String> {
        let rows = self.rows(query);
        let last = rows.len().checked_sub(1)?;
        let index = self.selected.min(last);
        let typed = !query.trim().is_empty() && index == last;
        info!(
            choice = %rows[index].choice,
            typed,
            rows = rows.len(),
            "task chosen"
        );
        Some(rows[index].choice.clone())
    }

    /// Highlight the next row down the screen, or up with `down` false. It stops
    /// at the ends.
    pub fn step(&mut self, down: bool, query: &str) {
        let last = self.rows(query).len().saturating_sub(1);
        let selected = self.selected.min(last);
        self.selected = if down {
            (selected + 1).min(last)
        } else {
            selected.saturating_sub(1)
        };
    }

    /// Back to the best match, for when what is typed has changed.
    pub fn reset(&mut self) {
        self.selected = 0;
        self.offset = 0;
    }

    /// Draw the rows that fit below `input`, the row where the task is typed.
    pub fn draw(&mut self, buffer: &mut Buffer, input: Rect, query: &str) {
        let room = usize::from(buffer.area.bottom().saturating_sub(input.y + 1)).min(MAX_ROWS);
        let input = input.intersection(buffer.area);
        if room == 0 || input.width < 6 {
            return;
        }
        let mut rows = self.rows(query);
        let highlighted = if rows.is_empty() {
            if let Some(message) = &self.message {
                rows.push(Row {
                    label: message.clone(),
                    details: Vec::new(),
                    choice: String::new(),
                });
            }
            None
        } else {
            Some(self.selected.min(rows.len() - 1))
        };
        self.offset = match highlighted {
            Some(row) if row < self.offset => row,
            Some(row) if row >= self.offset + room => row + 1 - room,
            _ => self.offset,
        }
        .min(rows.len().saturating_sub(room));

        let columns = usize::from(input.width) - 2;
        for (below, (index, row)) in rows.iter().enumerate().skip(self.offset).enumerate() {
            if below >= room {
                break;
            }
            let y = input.y + 1 + below as u16;
            let on = highlighted == Some(index);
            let area = Rect::new(input.x, y, input.width, 1);
            let background = if on { HIGHLIGHT_BACKGROUND } else { ROW_BACKGROUND };
            blend_samples(buffer, area, background, 1.0, |_, _| true);

            let marker = if on { "› " } else { "  " };
            let label = fit(&row.label, columns - marker.width());
            let style = if on {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(if highlighted.is_some() { TEXT } else { DIM })
            };
            let text = format!("{marker}{label}");
            let used = text.width();
            put_text(buffer, input.x + 1, y, &text, columns as u16, style);
            // Right-aligned, as many of the details as fit without crowding the title.
            let mut right = String::new();
            for detail in &row.details {
                let longer = if right.is_empty() {
                    detail.clone()
                } else {
                    format!("{right}  {detail}")
                };
                if used + 3 + longer.width() > columns {
                    break;
                }
                right = longer;
            }
            if !right.is_empty() {
                let width = right.width();
                let left = input.x + 1 + (columns - width) as u16;
                put_text(
                    buffer,
                    left,
                    y,
                    &right,
                    width as u16,
                    Style::default().fg(DIM),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(title: &str, group: Option<&str>) -> Task {
        Task {
            title: title.into(),
            group: group.map(str::to_owned),
            ..Task::default()
        }
    }

    fn picker() -> Picker {
        Picker::new(
            vec![
                task("write the report", Some("Work")),
                task("email Sam", Some("Work")),
                task("water the plants", Some("Home")),
            ],
            None,
        )
    }

    fn labels(picker: &Picker, query: &str) -> Vec<String> {
        picker
            .rows(query)
            .into_iter()
            .map(|row| row.label)
            .collect()
    }

    #[test]
    fn with_nothing_typed_every_task_is_listed_in_order() {
        let picker = picker();
        assert_eq!(
            labels(&picker, ""),
            ["write the report", "email Sam", "water the plants"]
        );
        assert_eq!(labels(&picker, "   "), labels(&picker, ""));
        assert_eq!(picker.choose("").as_deref(), Some("write the report"));
    }

    #[test]
    fn typing_narrows_the_list_and_adds_a_row_for_the_text_itself() {
        let picker = picker();
        assert_eq!(
            labels(&picker, "work"),
            ["write the report", "email Sam", "Use “work”"]
        );
        assert_eq!(
            labels(&picker, " the "),
            ["write the report", "water the plants", "Use “the”"]
        );
        assert_eq!(picker.choose("sam").as_deref(), Some("email Sam"));
    }

    #[test]
    fn text_that_matches_nothing_is_the_only_row_and_what_gets_chosen() {
        let picker = picker();
        assert_eq!(labels(&picker, "a misty forest"), ["Use “a misty forest”"]);
        assert_eq!(
            picker.choose("  a misty forest ").as_deref(),
            Some("a misty forest")
        );
    }

    #[test]
    fn an_empty_list_has_nothing_to_choose_until_something_is_typed() {
        let mut picker = Picker::new(Vec::new(), Some("No open tasks".into()));
        assert_eq!(picker.choose(""), None);
        picker.step(true, "");
        assert_eq!(picker.choose(""), None);
        assert_eq!(picker.choose("read").as_deref(), Some("read"));
    }

    #[test]
    fn moving_up_goes_to_later_rows_and_stops_at_both_ends() {
        let mut picker = picker();
        picker.step(false, "");
        assert_eq!(picker.choose("").as_deref(), Some("write the report"));
        picker.step(true, "");
        assert_eq!(picker.choose("").as_deref(), Some("email Sam"));
        picker.step(true, "");
        picker.step(true, "");
        picker.step(true, "");
        assert_eq!(picker.choose("").as_deref(), Some("water the plants"));
        // With text typed the row for the text is one further up.
        picker.step(true, "the");
        assert_eq!(picker.choose("the").as_deref(), Some("the"));
        picker.reset();
        assert_eq!(picker.choose("the").as_deref(), Some("write the report"));
    }

    #[test]
    fn a_selection_past_the_end_of_a_shorter_list_lands_on_its_last_row() {
        let mut picker = picker();
        for _ in 0..3 {
            picker.step(true, "");
        }
        assert_eq!(picker.choose("").as_deref(), Some("water the plants"));
        assert_eq!(picker.choose("sam").as_deref(), Some("sam"));
    }

    // ---- drawing ----

    const GREY: Color = Color::Rgb(128, 128, 128);

    /// Half blocks all over, as the gradient leaves them.
    fn screen(width: u16, rows: u16) -> Buffer {
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, rows));
        for cell in &mut buffer.content {
            cell.set_symbol("▀").set_fg(GREY).set_bg(GREY);
        }
        buffer
    }

    fn line(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    #[test]
    fn rows_stack_downward_from_just_below_the_box_with_the_highlight_marked() {
        let mut picker = picker();
        let mut buffer = screen(50, 20);
        picker.draw(&mut buffer, Rect::new(10, 8, 30, 1), "");
        assert!(
            line(&buffer, 9).contains("› write the report"),
            "{}",
            line(&buffer, 9)
        );
        assert!(
            line(&buffer, 10).contains("  email Sam"),
            "{}",
            line(&buffer, 10)
        );
        assert!(line(&buffer, 11).contains("  water the plants"));
        assert_eq!(line(&buffer, 12), "▀".repeat(50));
        // Nothing at or above the box, and nothing outside its columns.
        assert_eq!(line(&buffer, 8), "▀".repeat(50));
        assert_eq!(buffer[(9, 9)].symbol(), "▀");
        assert_eq!(buffer[(40, 9)].symbol(), "▀");
        // The rows are solid, the highlighted one lighter than the rest.
        assert_eq!(buffer[(10, 10)].bg, Color::Rgb(0, 0, 0));
        assert_eq!(buffer[(10, 10)].fg, Color::Rgb(0, 0, 0));
        assert_eq!(buffer[(10, 9)].bg, Color::Rgb(48, 48, 48));
    }

    #[test]
    fn a_group_goes_at_the_right_when_it_fits_and_is_left_off_when_it_does_not() {
        let mut picker = picker();
        let mut buffer = screen(50, 20);
        picker.draw(&mut buffer, Rect::new(10, 8, 40, 1), "");
        let row = line(&buffer, 9);
        // One column of padding is left after it.
        assert!(row.ends_with("Work▀"), "{row}");
        assert_eq!(row.matches("Work").count(), 1);

        let mut narrow = screen(50, 20);
        picker.draw(&mut narrow, Rect::new(10, 8, 24, 1), "");
        assert!(!line(&narrow, 9).contains("Work"), "{}", line(&narrow, 9));
        assert!(line(&narrow, 9).contains("write the report"));
    }

    fn detailed() -> Picker {
        use amber::tasks::Due;
        let today = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        let mut picker = Picker::new(
            vec![
                Task {
                    title: "write the report".into(),
                    group: Some("Work".into()),
                    due: Some(Due {
                        date: today,
                        time: None,
                    }),
                    labels: vec!["deep".into(), "quick".into()],
                },
                Task {
                    title: "email Sam".into(),
                    labels: vec!["errands".into()],
                    ..Task::default()
                },
            ],
            None,
        );
        picker.today = today;
        picker
    }

    #[test]
    fn the_due_date_labels_and_group_go_at_the_right_in_that_order() {
        let mut picker = detailed();
        let mut buffer = screen(80, 20);
        picker.draw(&mut buffer, Rect::new(5, 8, 70, 1), "");
        let row = line(&buffer, 9);
        assert!(row.contains("today  @deep @quick  Work"), "{row}");
        let other = line(&buffer, 10);
        assert!(other.contains("@errands"), "{other}");
    }

    #[test]
    fn a_narrow_row_drops_the_group_then_the_labels_then_the_due_date() {
        let ends = |width: u16| {
            let mut picker = detailed();
            let mut buffer = screen(80, 20);
            picker.draw(&mut buffer, Rect::new(5, 8, width, 1), "");
            line(&buffer, 9)
        };
        // "› write the report" is 18 columns, and a detail needs 3 apart from it.
        assert!(
            ends(52).contains("today  @deep @quick  Work"),
            "{}",
            ends(52)
        );
        let without_group = ends(46);
        assert!(
            without_group.contains("today  @deep @quick"),
            "{without_group}"
        );
        assert!(!without_group.contains("Work"));
        let only_due = ends(36);
        assert!(only_due.contains("today"), "{only_due}");
        assert!(!only_due.contains("@deep"));
        let none = ends(24);
        assert!(!none.contains("today"), "{none}");
        assert!(none.contains("› write the report"));
    }

    #[test]
    fn typed_text_finds_a_task_by_its_label() {
        let picker = detailed();
        assert_eq!(labels(&picker, "errands"), ["email Sam", "Use “errands”"]);
        assert_eq!(
            labels(&picker, "quick work"),
            ["write the report", "Use “quick work”"]
        );
    }

    #[test]
    fn a_title_too_long_for_the_row_is_cut_with_an_ellipsis() {
        let mut picker = Picker::new(
            vec![task("a very long title indeed for such a row", None)],
            None,
        );
        let mut buffer = screen(40, 20);
        picker.draw(&mut buffer, Rect::new(10, 8, 20, 1), "");
        let row = line(&buffer, 9);
        assert!(row.contains("› a very long tit…▀"), "{row}");
        assert_eq!(buffer[(30, 9)].symbol(), "▀");
    }

    #[test]
    fn rows_for_typed_text_show_what_would_be_used() {
        let mut picker = picker();
        let mut buffer = screen(50, 20);
        picker.draw(&mut buffer, Rect::new(5, 8, 40, 1), "sam");
        assert!(line(&buffer, 9).contains("› email Sam"));
        assert!(
            line(&buffer, 10).contains("Use “sam”"),
            "{}",
            line(&buffer, 10)
        );
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_highlight_in_view() {
        let tasks = (0..20).map(|n| task(&format!("task {n}"), None)).collect();
        let mut picker = Picker::new(tasks, None);
        let mut buffer = screen(40, 24);
        let input = Rect::new(5, 2, 30, 1);
        picker.draw(&mut buffer, input, "");
        // Eight rows, the first nearest the box.
        assert!(line(&buffer, 3).contains("› task 0"));
        assert!(line(&buffer, 10).contains("task 7"));
        assert_eq!(line(&buffer, 11), "▀".repeat(40));

        for _ in 0..9 {
            picker.step(true, "");
        }
        let mut buffer = screen(40, 24);
        picker.draw(&mut buffer, input, "");
        assert!(
            line(&buffer, 10).contains("› task 9"),
            "{}",
            line(&buffer, 10)
        );
        assert!(line(&buffer, 3).contains("task 2"));

        // Coming back up moves the view only once the highlight leaves it.
        picker.step(false, "");
        let mut buffer = screen(40, 24);
        picker.draw(&mut buffer, input, "");
        assert!(line(&buffer, 9).contains("› task 8"));
        assert!(line(&buffer, 3).contains("task 2"));

        picker.reset();
        let mut buffer = screen(40, 24);
        picker.draw(&mut buffer, input, "");
        assert!(line(&buffer, 3).contains("› task 0"));
    }

    #[test]
    fn it_uses_only_the_room_below_the_box() {
        let mut picker = picker();
        let mut buffer = screen(50, 12);
        picker.draw(&mut buffer, Rect::new(10, 9, 30, 1), "");
        assert!(line(&buffer, 10).contains("› write the report"));
        assert!(line(&buffer, 11).contains("  email Sam"));

        let mut none = screen(50, 12);
        picker.draw(&mut none, Rect::new(10, 11, 30, 1), "");
        assert!(none.content.iter().all(|cell| cell.symbol() == "▀"));
    }

    #[test]
    fn with_no_tasks_the_reason_is_shown_dimmed_and_cannot_be_chosen() {
        let mut picker = Picker::new(Vec::new(), Some("No open tasks in list.md".into()));
        let mut buffer = screen(50, 20);
        picker.draw(&mut buffer, Rect::new(5, 8, 40, 1), "");
        assert!(line(&buffer, 9).contains("No open tasks in list.md"));
        assert!(!line(&buffer, 9).contains('›'));
        assert_eq!(buffer[(7, 9)].fg, DIM);
        assert_eq!(picker.choose(""), None);
    }
}
