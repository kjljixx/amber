# JSON gradient recipes

Run `cargo run --offline -- --recipe examples/complex.json` to load the example.
The app starts in Loaded mode; `m` cycles through Simple, Complex, and Loaded.
Without `--recipe`, only Simple and Complex are available. Use `--help` for CLI usage.

All fields except `motion_style` are required. Unknown fields are rejected to catch spelling mistakes.

| Field | Meaning | Allowed values |
| --- | --- | --- |
| `points` | Color centers | 2–24 objects |
| `points[].position` | `[x, y]`, from top-left to bottom-right | Each coordinate 0–1 |
| `points[].color` | sRGB `[red, green, blue]` | Each channel an integer 0–255 |
| `softness` | Width of color transitions | 0–1 |
| `distortion` | Strength of folds | 0–1.5 |
| `swirl` | Rotation strength and direction | −1–1 |
| `seed` | Reproducible fold pattern | Integer 0–999 |
| `motion` | Drift strength (flow speed or wandering amplitude) | 0–1.5 |
| `motion_style` | Animation behavior; defaults to `loop` | `loop`, `flow`, `wander` |

Playback speed and terminal controls are separate from the recipe. `motion: 0`
disables drift; Space, `w`, and `d` keep their usual behavior.
Files are read and validated at startup, before entering the terminal UI.

`loop` preserves the original repeating point drift and distortion. `flow` keeps
color points anchored and continuously moves the distortion field. `wander`
moves points along bounded, seeded noise paths and also moves the distortion
field. Swirl pulsing remains periodic in all modes. Playback speed scales both
the periodic phase and continuous travel; pausing freezes both.

Jev-generated Sky and Horizon recipes use `flow`; Glow, Scatter, Superscatter, and Wash use
`wander`. Existing presets and JSON recipes without this field retain `loop`.
