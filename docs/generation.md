# Prompt-driven gradients with Jev

Start the generator with `cargo run --offline --bin jev`.
Use Up / Down to change animation speed from 0.125x to 16x.
The multiplier stays selected when you generate another gradient; Ctrl+P pauses.
Type a description and press Enter. Ctrl+P pauses motion; Esc or Ctrl+C exits.
The original `cargo run` renderer demo and JSON loading remain available.

Configure `OPENROUTER_API_KEY` in your environment or the local `.env` file.
`.env` is ignored by Git. `.env.example` contains only empty/example settings.
An optional `JEV_MODEL` overrides `typesafe/jev-1.13`.

Generate and save a recipe without entering the TUI:

```sh
cargo run --offline --bin jev -- --prompt "glacier ice" --output glacier.json
cargo run --offline -- --recipe glacier.json
```

Saving refuses to overwrite an existing file. Recipe files contain no credentials.
The saved recipe contains visual settings; generated playback speed is used by
the generator UI, but remains separate from the recipe format.

## How generation works

```text
description
    → one Jev Decisions API request
    → color families, top color, layout, lightness, saturation,
      contrast, energy, and edge softness
    → local deterministic composition with 18 color points
    → validated Recipe
    → GradientView
```

Jev chooses bounded values rather than writing JSON coordinates or rendering
code. Palette conversion and point placement happen locally. Six layouts are
available: sky, horizon, glow, scatter, superscatter, and wash. Superscatter uses
18 irregularly spaced points (9 primary, 6 secondary, 3 accent), spreads the
color families throughout the image, and reduces softness to keep smaller pockets visible. The website uses this same
decision-to-composition approach; our point placement is adapted for this renderer.
No prism, grain, zoom, or seed randomization is added. Seed stays 58.

The network request runs on a worker thread with a 20-second timeout. The old
gradient keeps animating while it is pending. Failures preserve the old recipe
and show a retry message. Enter is ignored while a request is already pending.
Successful descriptions are cached during the session to avoid repeated calls.
Requests are billed to the configured OpenRouter account; tests use local mocks.

## Embedding generation

`amber::generation::JevClient` is separate from the renderer. A host can construct
it with its own key and model, then call `generate(description)` on a worker.
The result exposes the validated recipe, design decisions, playback speed, and
optional API-reported cost. Configuration loading is explicit; the library never
reads `.env` merely because it is imported. Credentials are not included in
debug output or server-error messages.

`amber::generation::Generator` is the non-blocking wrapper `jev` and `timer`
share, for a host that does not want to manage a thread itself:

```rust
let mut generator = Generator::new();

// when the user submits a description
if let Some(cached) = generator.submit("glacier ice") {
    // this prompt was already answered this session: use it now
}

// every pass of the event loop
if let Some(result) = generator.poll() {
    // Ok(GeneratedGradient) or Err(message); the request has finished
}
// wake every ~25 ms while generator.is_pending(); show generator.status()
```

`submit` runs `JevClient::generate` on a worker thread, using `OPENROUTER_API_KEY`
and `JEV_MODEL` from the environment or a `.env` file in the working directory.
It ignores a second request while one is in flight and remembers the last 32
answers, so repeating a description costs nothing.

`cargo run --offline --bin timer` is a pomodoro clock that uses it: press `t`
to choose the task, `Up`/`Down` to change the gradient's speed. Gradients play
at 8x the rate Jev suggests by default. Jev is told the task and the moment (see
below), and once it has made a gradient the task stays above the clock until a
new one replaces it.

## Choosing the task from a list

`--tasks list.md` gives `t` a list to choose from: the unfinished items of a
markdown file, such as `- [ ] write the report`. A ticked item (`- [x]`) is
left out, a plain bullet or numbered item counts, and a heading is the group of
the items after it, shown at the right of each row. The file is read again each
time `t` is pressed, so edits show up at once. `amber::tasks` has the parser and
a `Task` type for other hosts.

The box above the clock filters the list as you type. The best match is just
above the box, the rest stack upward, and `Up`/`Down` move the highlight. Enter
takes the highlighted row. The last row, "Use “…”", is whatever you typed, for a
task that is not on the list; with no list, or nothing that matches, it is the
only row. A file that cannot be read, or has nothing unfinished in it, says so
in place of the rows.

### From Todoist

`--todoist` chooses from your Todoist account instead of a file (`--tasks` and
`--todoist` are alternatives). Make a personal API token in Todoist's settings
under Integrations, then Developer, and set `TODOIST_API_TOKEN` in the
environment or the local `.env` file, as for the Jev key. `.env.example` has the
line.

- By default the tasks are all your active ones. `--todoist-filter "today |
  overdue"` (or any
  [Todoist filter](https://todoist.com/help/articles/introduction-to-filters-V98wIH),
  such as `#Work & p1`) reads only the tasks it matches instead; the filter is sent
  to Todoist, which does the choosing.
- They are listed soonest due first (overdue ones at the top, tasks with no date
  last), and within a day the most urgent first (p1 before p4). At the right of
  each row go when it is due ("today", "tomorrow", a weekday for the next few
  days, otherwise a date, with the time if it has one), its labels as `@label`
  and its project's name. A narrow terminal drops the project first, then the
  labels. Typing also finds tasks by their labels. Links in a task's title are
  replaced by their words. Up to 2,000 tasks are read.
- The list is fetched in the background at launch, so it is ready for the first
  `t`, and again when `t` is pressed if the last fetch was over 30 seconds ago.
  While it loads, the box says so. If a fetch fails the last list stays, with a
  message at the top, and if there never was a list the reason is shown.
- The token goes only to `api.todoist.com`, in an `Authorization` header, and is
  never printed. The timer only reads: it never completes or changes a task.
  Todoist sees the filter you use. Reading is two requests (tasks, then project
  names) per fetch.

## A first gradient from the moment

At startup the timer asks Jev for a gradient for the moment, so there is one
before you choose a task: `amber::ambient` builds a line such as "Friday golden
hour in October (autumn), light rain, cool" from the local weekday, time of day,
month and weather. Try `cargo run --offline --example ambient`.

Every task you choose after that is sent with only a hint of it, its light and
weather in a few words ("write the report, with a hint of golden hour, light
rain"). A task says little about color and the full line says a lot, so sending
the whole of it let the weather decide the gradient.

- The forecast comes from [Open-Meteo](https://open-meteo.com), which needs no
  key. Where the machine is comes from its IP address, looked up at `ipwho.is`,
  so both services see your IP. `--location Lisbon` names a place instead (it
  is looked up at Open-Meteo), and `--no-weather` makes no lookups at all and
  describes the time alone.
- The timer waits up to 3 seconds for the weather; the default gradient plays
  meanwhile. If the lookup fails or is slow, the description is the time alone.
- Only the description goes to Jev. The place's name never does.
- Day parts follow the sun when the forecast has sunrise and sunset (dawn,
  golden hour, dusk) and the clock otherwise. The time used is the local time
  at the weather's place, so the sun and the clock agree.
- It makes one Jev request per launch. If there is no API key or the request
  fails, nothing is shown, and a description you type is unaffected. A gradient
  you ask for before it arrives is kept.

For renderer-only consumers, use `default-features = false` in the Amber Cargo
dependency. This removes the optional HTTP client and generation API.

API references:
- [OpenRouter Jev Decisions tutorial](https://openrouter.ai/docs/guides/community/jev-tutorial)
- [Mesh website](https://mesh.iamnoman.com)
