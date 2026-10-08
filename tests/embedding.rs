use std::time::{Duration, Instant};

use amber::{GradientView, Playback, presets};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Paragraph, Widget},
};

fn view(now: Instant) -> GradientView {
    GradientView::new(&presets::presets()[0].recipe, now).unwrap()
}

#[test]
fn cached_view_repaints_in_offset_panels_and_resizes() {
    let now = Instant::now();
    let mut view = view(now);
    let area = Rect::new(3, 2, 8, 4);
    let mut first = Buffer::empty(Rect::new(0, 0, 16, 8));
    Widget::render(&mut view, area, &mut first);
    assert_eq!(view.stats().rebuilds, 1);
    assert!(!view.update(now));
    let mut second = Buffer::empty(first.area);
    Widget::render(&mut view, area, &mut second);
    assert_eq!(first, second);
    assert_eq!(view.stats().rebuilds, 1);
    assert_eq!(second[(0, 0)].symbol(), " ");
    assert_eq!(second[(3, 2)].symbol(), "▀");
    Widget::render(&mut view, Rect::new(2, 1, 5, 3), &mut second);
    assert_eq!(view.stats().rebuilds, 2);
    Widget::render(&mut view, Rect::new(0, 0, 0, 0), &mut second);
    assert_eq!(view.stats().rebuilds, 3);
}

#[test]
fn overlays_survive_on_top_of_a_repainted_background() {
    let now = Instant::now();
    let mut view = view(now);
    let area = Rect::new(0, 0, 20, 8);
    let mut buffer = Buffer::empty(area);
    Widget::render(&mut view, area, &mut buffer);
    Paragraph::new("Menu")
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .render(Rect::new(5, 3, 10, 2), &mut buffer);
    assert_eq!(buffer[(5, 3)].symbol(), "M");
    assert_eq!(buffer[(5, 3)].bg, Color::Black);
    assert_eq!(buffer[(1, 1)].symbol(), "▀");
}

#[test]
fn views_have_independent_playback_and_no_paused_wakeups() {
    let now = Instant::now();
    let mut paused = view(now);
    let mut active = view(now);
    paused.set_playback(
        Playback {
            playing: false,
            ..Playback::default()
        },
        now,
    );
    let area = Rect::new(0, 0, 10, 5);
    let mut before = Buffer::empty(area);
    Widget::render(&mut paused, area, &mut before);
    Widget::render(&mut active, area, &mut Buffer::empty(area));
    let later = now + Duration::from_secs(60);
    assert!(!paused.update(later));
    assert_eq!(paused.wait_duration(later), None);
    assert!(active.update(later));
    let mut after = Buffer::empty(area);
    Widget::render(&mut paused, area, &mut after);
    assert_eq!(before, after);
    paused.set_playback(Playback::default(), later);
    assert!(paused.update(later));
    Widget::render(&mut paused, area, &mut after);
    assert_eq!(before, after);
    assert_eq!(paused.wait_duration(later), Some(Duration::from_millis(50)));
    assert!(paused.update(later + Duration::from_millis(50)));
}

#[test]
fn replacing_a_recipe_is_validated_and_invalidates_the_cache() {
    let now = Instant::now();
    let mut view = view(now);
    let area = Rect::new(0, 0, 10, 5);
    let mut before = Buffer::empty(area);
    Widget::render(&mut view, area, &mut before);
    let mut invalid = presets::presets()[1].recipe.clone();
    invalid.motion = -1.0;
    assert!(view.set_recipe(&invalid).is_err());
    assert!(!view.update(now));
    let mut after = Buffer::empty(area);
    Widget::render(&mut view, area, &mut after);
    assert_eq!(before, after);
    view.set_recipe(&presets::presets()[1].recipe).unwrap();
    assert!(view.update(now));
    Widget::render(&mut view, area, &mut after);
    assert_ne!(before, after);
    assert_eq!(view.stats().rebuilds, 2);
}

#[test]
fn timing_is_configurable_and_zero_speed_sleeps() {
    let now = Instant::now();
    let mut view = view(now);
    assert!(view.set_timing(Duration::ZERO, 1.0, now).is_err());
    assert!(
        view.set_timing(Duration::from_millis(20), f64::NAN, now)
            .is_err()
    );
    view.set_timing(Duration::from_millis(20), 0.0, now)
        .unwrap();
    assert_eq!(view.wait_duration(now), None);
    view.set_timing(Duration::from_millis(20), 0.15, now)
        .unwrap();
    let area = Rect::new(0, 0, 10, 5);
    Widget::render(&mut view, area, &mut Buffer::empty(area));
    assert_eq!(view.wait_duration(now), Some(Duration::from_millis(20)));
    assert!(!view.update(now + Duration::from_millis(19)));
    assert!(view.update(now + Duration::from_millis(20)));
    view.set_playback(
        Playback {
            warped: false,
            drifting: false,
            ..Playback::default()
        },
        now,
    );
    assert_eq!(view.wait_duration(now), None);
}
