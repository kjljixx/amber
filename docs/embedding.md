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

## Layering

Render the gradient first and other widgets afterward. Solid panels should clear
their rectangle and set an explicit background color. Text that only changes
foreground color inherits the bottom half's background, but replaces the
half-block character, losing the cell's two-color split. Alpha blending is not
provided by the terminal or this API.

Run `cargo run --offline --example background` for a complete host application
with a solid panel over the gradient. Space pauses; `q` exits.

For non-Ratatui output, use `Recipe::compile()`, then
`CompiledRecipe::prepare(aspect, phase, warped, drifting)` and `sample(x, y)`.
Coordinates are normalized from the top-left. This lower-level API uses no
terminal or event loop. JSON loading is optional through `Recipe::load(path)`.
