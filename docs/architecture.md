# Amber architecture

Amber is a library (`src/lib.rs`) and three binaries. The library renders animated gradients as Ratatui half-block cells. The binaries add input, timing, and network features.

```text
┌──────────────────────────────── BINARIES ────────────────────────────────┐
│  amber (main.rs, cli.rs)      jev (bin/jev.rs)       timer (bin/timer.rs)│
│  preset/recipe/scene viewer   prompt → gradient TUI  pomodoro clock      │
│                                                                          │
│  local modules: speed_control · picker · alarm · focus_overlay           │
└───────┬──────────────────────────┬───────────────────────┬───────────────┘
        │                          │                       │
        │                          ▼                       ▼
        │      ┌──────────── INPUT MODULES (worker threads + mpsc) ────────────┐
        │      │ generation ◄─ moment text ─ ambient ──► Open-Meteo, ipwho.is  │
        │      │   │  ▲                                                        │
        │      │   │  └── dotenv_settings ── tasks ──► Todoist / Markdown file │
        │      │   ▼                                                           │
        │      │ OpenRouter (Jev)       audio ──► system audio / microphone    │
        │      │                        focus ──► screen + OpenRouter (Luna)   │
        │      └───────┬───────────────────┬──────────────────┬────────────────┘
        │              │ Design            │ Excitement       │ off-task event
        │              ▼                   │                  ▼
        │      ┌── RECIPE SOURCES ──┐      │          alarm / focus_overlay
        │      │ scene (elements,   │      │          (chime, notification,
        │      │  color, templates) │      │           eframe window)
        │      │ presets            │      │
        │      │ recipe/scene JSON  │      │
        │      └────────┬───────────┘      │
        │               │ compile          │
        ▼               ▼                  ▼
┌─────────────────────────────── RENDERING CORE ───────────────────────────┐
│                                                                          │
│   Recipe ──► CompiledRecipe ──► PreparedGradient ──► Preview ──► ▀ cells │
│   (recipe)   (recipe)           (gradient)           (half_blocks)       │
│                                      │                                   │
│                                      ▼                                   │
│                                 WarpStack (warp)                         │
│                                                                          │
│   GradientView (view) drives all of it:                                  │
│     Animation phase (animation) · Playback · Excitement · Reactivity     │
│   color: Rgb ↔ OKLab, used by gradient, warp, scene, overlay             │
└──────────────────────────────────┬───────────────────────────────────────┘
                                   │ cells already painted
                                   ▼
┌──────────────────────────────── WIDGETS ─────────────────────────────────┐
│  Digits (digits) ──► blend_samples (overlay)     picker, progress bars   │
└──────────────────────────────────────────────────────────────────────────┘
```

## Layers

| Layer | Modules | Role |
|---|---|---|
| Binaries | `main`, `bin/jev`, `bin/timer` | Own the terminal, event loop, and key handling. |
| Input modules | `generation`, `ambient`, `tasks`, `audio`, `focus` | Gather outside data on worker threads. Each one reports back through a channel. |
| Recipe sources | `scene`, `presets`, JSON files | Produce a `Recipe`. Scenes compile to points. |
| Rendering core | `view`, `recipe`, `gradient`, `warp`, `animation`, `color`, `half_blocks` | Turn a `Recipe` plus time into colored half-block cells. No network, no input. |
| Widgets | `digits`, `overlay` | Blend shapes over cells that the gradient already drew. |

## Which binary uses what

| Module | `amber` | `jev` | `timer` |
|---|---|---|---|
| `view` / rendering core | yes | yes | yes |
| `presets`, `scene` | yes | via `generation` | via `generation` |
| `generation` | no | yes | yes |
| `ambient`, `tasks`, `audio`, `focus` | no | no | yes |
| `digits`, `overlay` | no | no | yes |

## Data flows

**Gradient render (every frame)**
`GradientView::update(now)` → `Animation` phase → `PreparedGradient` samples points through `WarpStack` → `Preview` → `HalfBlocks` widget → `Digits` blends over it.

**Prompt to gradient (Jev)**
Task text + `ambient::describe` → `JevClient` request → `Design` (hues, layout, lightness, energy) → `Scene` of elements → `Recipe` → `GradientView::set_recipe`.

**Audio reaction**
`Capture` → `Analyzer` → `Excitement { level, pulse, activity, tone }` → `GradientView::set_excitement`.

**Focus check (timer only)**
Every 20 seconds in a work phase: screenshot → Luna Decisions → `Event::OffTask` → desktop notification and `focus_overlay` window.

## Feature flags

| Feature | Enables |
|---|---|
| `generation` | `generation`, `ureq`, `getrandom`; required by `jev` and `timer` |
| `audio` | `audio::Capture` (`cpal`) |
| `focus-check` | `focus`, `focus_overlay` (`xcap`, `image`, `base64`, `eframe`) |

`ambient` and `tasks` always compile; only their network lookups use `generation`.

## Logging

`amber::log` routes `tracing` events to five files by module path:
`gradient.log`, `tasks.log`, `audio.log`, `focus.log`, and `timer.log`, in
`logs/run-<timestamp>-<pid>/`. Each binary calls `log::init()` once. See
`docs/generation.md` for the file contents, `--debug`, and `RUST_LOG`.
