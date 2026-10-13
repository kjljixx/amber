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
cargo run --offline --bin amber -- --recipe glacier.json
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
Motion follows the layout: sky bands breathe while keeping their order, the
horizon seam bends and changes width, and a glow drifts and expands gently.
Scatter and superscatter points can cross one another; wash patches move slowly.
Each path is seeded, smooth, and non-looping. No prism, grain, or zoom is added.
The point composition stays deterministic, while each
new generated recipe receives a fresh uniformly random warp seed from the
operating system (0–999). Reusing a cached design also gets a fresh warp seed.

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

`amber::generation::Generator` is the non-blocking wrapper `jev` and `amber`
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

`cargo run --offline` is a pomodoro clock that uses it: press `t`
to choose the task, `Up`/`Down` to change the gradient's speed. Gradients play
at 8x the rate Jev suggests by default. Jev is told the task and the moment (see
below), and once it has made a gradient the task stays above the clock until a
new one replaces it.

The focus checker compares the selected task with a screenshot every 20
seconds by default, starting 20 seconds after you choose a task. It pauses when
the timer is paused or during Pomodoro breaks, then starts a fresh 20-second
delay when work resumes:

```sh
cargo run --offline
```

Pass `--no-focus-check` to disable it for a run.

Focus checks are logged to `logs/run-<timestamp>-<pid>/focus.log` (see
[Logs](#logs)). At the default `info` level each check logs its schedule,
screenshot size, model status and timing, decision, streak, notification, and
overlay events. Run with `--debug` to also save each
screenshot as `focus/check-NNNN.jpg` and the raw model response as
`focus/check-NNNN-response.json`, and to log the prompt. These files can contain
your screen contents, task text, and model response; `logs/` is ignored by Git.

GPT-6 Luna Decisions receives the screenshot and task text through OpenRouter.
From the second consecutive off-task decision onward, each off-task check sends
a desktop notification and shows an always-on-top overlay with a dark, padded
panel behind the reminder for eight seconds. The title is “Let's get back on
track” and the supporting line is “Don't get distracted. Stay focused on your
task” in both places. The timer caption combines the same two lines. After the
first off-task decision, checks run every five seconds until an on-task
decision, then return to twenty-second intervals. A later off-task decision
can show the overlay again for eight seconds if it has already closed. The
transparent overlay window does not switch to a separate fullscreen Space.
Choosing a new task starts a new sequence. To preview the overlay directly,
run
`cargo run --offline -- --focus-overlay
"Let's get back on track"`; it closes after eight seconds, or press Enter in
that terminal to close it sooner.
`FOCUS_MODEL` can override the default
`openai/gpt-6-luna-decisions`. The first macOS capture invokes the system
Screen Recording permission flow; allow Amber in System Settings → Privacy &
Security → Screen & System Audio Recording and restart Amber if capture does
not resume. Windows has no equivalent per-app screen recording prompt. If
capture or a model call fails, amber shows an error and retries on the next
check.

## Choosing the task from a list

`--tasks list.md` gives `t` a list to choose from: the unfinished items of a
markdown file, such as `- [ ] write the report`. A ticked item (`- [x]`) is
left out, a plain bullet or numbered item counts, and a heading is the group of
the items after it, shown at the right of each row. The file is read again each
time `t` is pressed, so edits show up at once. `amber::tasks` has the parser and
a `Task` type for other hosts.

The box above the clock filters the list as you type. The best match is just
below the box, the rest stack downward, and `Up`/`Down` move the highlight. Enter
takes the highlighted row. The last row, "Use “…”", is whatever you typed, for a
task that is not on the list; with no list, or nothing that matches, it is the
only row. A file that cannot be read, or has nothing unfinished in it, says so
in place of the rows.

### From Todoist

Amber chooses from your Todoist account by default; `--tasks` names a file
instead, and `--no-todoist` turns the list off (`--tasks` and `--todoist` are
alternatives). Without a token amber still starts and shows a notice. Make a personal API token in Todoist's settings
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
  never printed. Amber only reads: it never completes or changes a task.
  Todoist sees the filter you use. Reading is two requests (tasks, then project
  names) per fetch.

## A first gradient from the moment

At startup amber asks Jev for a gradient for the moment, so there is one
before you choose a task: `amber::ambient` builds a line such as "Friday golden
hour in October (autumn), light rain, cool" from the local weekday, time of day,
month and weather. Try `cargo run --offline --example ambient`.

That line does not go to the gradient request directly. Jev first chooses, from
255 random lyric lines, the one that best evokes the moment (one `choice`
question, as in `04_jev_choice.py`). The gradient is then made from that lyric
alone, and the lyric is the caption above the clock.

The lines come from `assets/phrases.txt`, which Git ignores. Make it once with
`python 05_build_phrase_pool.py`: it reads the Genius lyrics dataset (see
`01_download_lyrics.py`) and keeps 200,000 rock and pop lines, one per song, of 3 to 12 words.
Without that file the startup gradient fails and amber shows why.
`amber::phrases` has the sampling and the request; `Generator::submit_moment`
runs both requests on one worker.

Every task you choose after that is sent with only a hint of it, its light and
weather in a few words ("write the report, with a hint of golden hour, light
rain"). A task says little about color and the full line says a lot, so sending
the whole of it let the weather decide the gradient.

- The forecast comes from [Open-Meteo](https://open-meteo.com), which needs no
  key. Where the machine is comes from its IP address, looked up at `ipwho.is`,
  so both services see your IP. `--location Lisbon` names a place instead (it
  is looked up at Open-Meteo), and `--no-weather` makes no lookups at all and
  describes the time alone.
- Amber waits up to 3 seconds for the weather; the default gradient plays
  meanwhile. If the lookup fails or is slow, the description is the time alone.
- While amber runs, it refreshes the weather and moment every 5 minutes.
  It resolves the location once and reuses those coordinates for later forecasts.
- Only the description goes to Jev. The place's name never does.
- Day parts follow the sun when the forecast has sunrise and sunset (dawn,
  golden hour, dusk) and the clock otherwise. The time used is the local time
  at the weather's place, so the sun and the clock agree.
- It makes two Jev requests per launch: the lyric, then the gradient. If there
  is no API key, no phrase file, or a request fails, nothing is shown and the
  timer shows the reason briefly, and a description you type is unaffected. A gradient
  you ask for before it arrives is kept.

For renderer-only consumers, use `default-features = false` in the Amber Cargo
dependency. This removes the optional HTTP client and generation API.

API references:
- [OpenRouter Jev Decisions tutorial](https://openrouter.ai/docs/guides/community/jev-tutorial)
- [Mesh website](https://mesh.iamnoman.com)

## Logs

Every run of `amber`, `jev`, or `gradient` writes to a new directory,
`logs/run-<timestamp>-<pid>/`, and prints its path when it exits. Nothing is
logged to the terminal. A file appears when its first line is written.

| File | Parts |
|---|---|
| `gradient.log` | weather, lyric choice, Jev requests, scene, recipe, view |
| `tasks.log` | task list, Todoist, task picker |
| `audio.log` | audio capture and analysis |
| `focus.log` | focus checks, decisions, overlay |
| `amber.log` | clock phases, alarm, startup options, shutdown |

The level is `info`. `--debug` (accepted by `amber`, `jev`, and `gradient`) makes it
`debug` for every file. `RUST_LOG` adds to either, for example
`RUST_LOG=amber::audio=trace` for one part. API keys and tokens are never logged.
