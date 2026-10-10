# Embedding Amber in a Ratatui app

Amber exposes a library alongside the demo binary. In another local project:

```toml
[dependencies]
amber = { path = "../amber" }
ratatui = "0.30"
```

Create one `GradientView` per independently controlled background or panel:

```rust
use std::time::Instant;
use amber::{GradientView, presets};

let mut gradient = GradientView::new(&presets::presets()[1].recipe, Instant::now())?;
```

The host owns terminal initialization, input, layout, and the event loop:

```text
redraw |= gradient.update(now)

if redraw:
    terminal.draw(frame):
        gradient.render(frame, desired_rectangle)
        draw your panels, text, and controls afterward

wait for input or the earliest deadline among your components
```

`render` always paints the cached colors into Ratatui's current frame, but only
resamples when time, recipe, effects, or dimensions change. It supports offset
rectangles and handles empty areas. Render separate areas with separate views;
one view holds a single preview cache. Rectangles must fit the frame buffer.
`frame.render_widget(&mut gradient, area)` is also supported.

## Controls and timing

The library handles no keys. Use `playback()` and `set_playback(settings, now)`
to change pause, warp, and drift. `set_recipe(&recipe)` validates the new settings
before replacing the current gradient. `set_compiled` accepts an already cached
recipe. Switching recipes preserves the animation phase and playback settings.

`set_timing(interval, radians_per_second, now)` changes frame pacing and speed
without resetting the phase. Defaults match the demo: 50 ms and 3 radians per
second. For website-like pacing, use 0.15 radians per second.
`wait_duration(now)` returns the next wakeup delay, or `None` when paused, speed
is zero, or both effects are disabled. The host should combine this with other
components' deadlines; Amber never waits or polls events itself. Call `update`
even if another component triggers the redraw. New recipes and playback changes
make `update` report that a redraw is needed. Layout changes require a host redraw.

`stats()` reports the latest sampling duration and total preview rebuilds.
The host measures total frame time, including its own widgets and terminal output.

## Reacting to something outside

`set_excitement(Excitement { level, pulse, activity, tone })` stirs the view up.
All but `tone` run from 0 to 1:

- `level` is sustained loudness: livelier and faster.
- `pulse` is a hit that decays by itself: a surge forward and a harder warp.
- `activity` is how much the sound is changing, with no beat needed: a run of
  notes keeps it up, a held chord lets it fall. It speeds and warps continuously.
- `tone` is where the sound sits between dark and bright. Its *changes* steer the
  picture, forward as it rises and back as it falls, so a melody pushes the
  gradient around and a held note leaves it be. `None` means no steering.

At zero the picture is exactly the recipe's own, and a paused view stays still.
`set_reactivity(Reactivity { speed, kick, busy, warp, steer })` sets how much full
excitement adds. The speeds are radians per second *added* to the view's own, so
the music moves a calm gradient and a fast one by the same amount, and the speed
control does not change how hard it hits: the defaults are 2.25 from `level`, 4.5
from `pulse` and 3.75 from `activity`. Warp strength grows by `warp` (0.75 grows it
by three quarters) at full `pulse`, half as much from `activity` and a quarter from
`level`. `steer` is 10 radians across the whole tone range (about 2 per octave). Keep the frame
interval short (about 33 ms) while excited so a beat is seen.

`amber::audio` is one source of it. `Analyzer` takes mono samples and reports
`level` (loudness against the last few seconds, so quiet and loud music both
reach the top), `pulse` (any onset in the spectrum between 30 Hz and 4 kHz, a
plucked note as well as a kick, with soft ones giving small pulses), `activity`
(that change against the biggest lately) and `tone` (the spectral centroid,
100 Hz to 4 kHz over octaves).
`take()` returns the strongest pulse since its last call. With the `audio`
feature (on by default; it needs ALSA headers on Linux, so use
`default-features = false, features = ["generation"]` to leave it out), `Capture`
runs an analyzer on a device:

```rust
let mut capture = Capture::start(Source::System, 1.0);
// every pass of the event loop
view.set_excitement(capture.poll());
// capture.status() is Starting, Listening(device) or Failed(message)
```

`Source::System` records what the machine plays: through a Core Audio tap on
macOS 14.6 or later (the first use asks to record system audio) and WASAPI
loopback on Windows. Elsewhere it looks for a "monitor" input, which is
untested. `Source::Microphone` and `Source::Named("BlackHole")` choose other
devices. Sound is analyzed as it arrives and never stored or sent anywhere.

`cargo run --offline --example listen` prints what it hears: level, activity, tone and pulse. In the timer, `a` turns it on or off, and
`--audio`, `--audio-from`, `--sensitivity` and `--list-audio` choose how.

## Layering

Render the gradient first and other widgets afterward. Solid panels should clear
their rectangle and set an explicit background color. Text that only changes
foreground color inherits the bottom half's background, but replaces the
half-block character, losing the cell's two-color split. The terminal has no
alpha blending, so ordinary widgets cannot be translucent; `Digits` (below)
blends into the half-block colors itself.

## Big digits

`Digits` is a separate widget with no link to the gradient code. Render it after
the gradient, into the same rectangle:

```rust
let mut digits = Digits::new("25:49", [255, 255, 255], 0.85)?;

terminal.draw(|frame| {
    gradient.render(frame, area);
    frame.render_widget(&mut digits, area);
})?;
```

It reads the two colors already in each `▀` cell and blends the digits over
them, so they have the gradient's full vertical resolution and real
translucency: `alpha` 1 is opaque, lower lets what is underneath show through
(blended in OKLab, so its hue is kept). Cells that are not colored half blocks
are left alone. Render it once per frame, after the content it covers; drawing
it twice would blend twice. It works over any half-block content, not only
`GradientView`.

Text may use `0` to `9`, `:`, `.`, `-` and spaces, with `\n` for several lines.
Each pixel of the 5 by 9 digit font becomes an `n` by `n` square of samples,
where `n` is the largest whole number that fits the area with a 10% margin, and
the text is centered. Strokes are therefore all the same width and edges are
hard, but the size moves in steps (`n` 2 to 3 is a 50% jump) and the text
rarely fills the full width. If even `n = 1` does not fit, nothing is drawn: at
`n = 1` a digit is 5 samples wide and 9 tall, so "25:49" needs about 32 columns
and 6 terminal rows once the margin is counted. Every digit fills the same box and has the
same width, so a clock does not shift as it changes.

`Digits::bounds(area)` gives the rectangle of cells the digits occupy when
rendered into `area`, for placing something beside them; the timer's prompt
line uses it.

The coverage is computed once per area size, so animation costs only the blend
over covered samples. Pick a color with contrast against the whole gradient:
pale text fades over light regions. To change the text, build a new `Digits`.

To draw other translucent shapes the same way, `blend_samples(buffer, area,
color, alpha, covered)` blends a color over whichever half-block samples
`covered(column, row)` selects, where `row` counts two samples per terminal row.
`Digits` is built on it, and `src/bin/timer.rs` uses it for its progress bars.

The shapes are the `glyph()` table in `src/digits.rs`. `tools/digit-editor.html`
is a standalone browser editor for them, with a live preview and an export in
the same form.

Run `cargo run --offline --example digits -- 25:47 0.85` to try it, or
`cargo run --offline --bin timer` for a pomodoro clock built on it.

Run `cargo run --offline --example background` for a complete host application
with a solid panel over the gradient. Space pauses; `q` exits.

For non-Ratatui output, use `Recipe::compile()`, then
`CompiledRecipe::prepare(aspect, phase, warped, drifting)` and `sample(x, y)`.
Coordinates are normalized from the top-left. This lower-level API uses no
terminal or event loop. JSON loading is optional through `Recipe::load(path)`.
