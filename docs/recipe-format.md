# JSON gradient recipes

Run `cargo run --offline --bin amber -- --recipe examples/complex.json` to load the example.
To describe a picture as named colors and elements instead of raw points, see [scene-format.md](scene-format.md).
The app starts in Loaded mode; `m` cycles through Simple, Complex, and Loaded.
Without `--recipe`, only Simple and Complex are available. Use `--help` for CLI usage.

All fields except `motion_style`, `warp`, and the optional `points[]` shape fields (`radius`, `stretch`, `strength`, `angle`) are required. Unknown fields are rejected to catch spelling mistakes.

| Field | Meaning | Allowed values |
| --- | --- | --- |
| `points` | Color centers | 2–256 objects |
| `points[].position` | `[x, y]`, from top-left to bottom-right; points outside 0–1 sit off the image | Each coordinate −0.5–1.5 |
| `points[].color` | sRGB `[red, green, blue]` | Each channel an integer 0–255 |
| `points[].radius` | Optional width and edge softness of this point's region; defaults to `1` | 0.05–10 |
| `points[].stretch` | Optional elongation of the region along its long axis; defaults to `1` | 0.1–10 |
| `points[].strength` | Optional weight multiplier on the point's color; defaults to `1` | 0.05–20 |
| `points[].angle` | Optional clockwise rotation of the region, in degrees; defaults to `0` | −180–180 |
| `softness` | Width of color transitions | 0–1 |
| `distortion` | Strength of folds | 0–1.5 |
| `swirl` | Rotation strength and direction | −1–1 |
| `seed` | Reproducible fold pattern | Integer 0–999 |
| `motion` | Drift strength (flow speed or wandering amplitude) | 0–1.5 |
| `motion_style` | Animation behavior; defaults to `loop`; `flow_wave` adds a slower angled wave over steady flow | `loop`, `flow`, `flow_wave`, `wander`, `glow`, `scatter`, `superscatter`, `wash` |
| `warp` | Optional extra warp layers, run in order after the classic ones | At most 16 layers in total, including the classic ones |

`radius` sets a point's width and edge softness relative to `softness`: below 1
gives a small, crisp spot (city lights), above 1 a broad glow. `stretch`
elongates the region without changing that width: below 1 reaches `radius / stretch`
vertically (tree trunks), above 1 reaches `radius * stretch` horizontally (bands
and horizon glows). Both are measured in aspect-corrected space.

`strength` multiplies how hard a point pulls its color. A small light among
dark points is normally blended down to a fraction of its color; `strength: 6`
lets it reach most of it. Values above 1 also push the point's seams outward a
little, so its region grows slightly; values below 1 make a faint tint that
never takes over. `angle` rotates the region clockwise on screen (positive
turns the long axis from horizontal toward down-right) and does nothing to a
round point (`stretch: 1`). Like `stretch`, it is measured in aspect-corrected
space, so on a wide image a 45° streak looks slightly flatter than 45°.

Points that omit these fields render exactly as before.
`examples/point-shapes.json` combines all four: a stretched horizon glow, tall
trunks, a diagonal light shaft, and small bright lights.

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

## Warp layers

A warp bends the sampling position before colors are looked up; it never moves
the points. `distortion`, `swirl`, and `seed` expand to the classic stack: a
swirl (when `swirl` is not 0), two `folds` layers, and a `noise` layer (when
`distortion` is not 0). Layers listed in `warp` run after it, in order. Set
`distortion` and `swirl` to 0 to use only your own layers. The `w` key turns the
whole warp off. Recipes without `warp` render exactly as before.

Each layer is an object with a `type`:

| `type` | Fields (defaults in parentheses) |
| --- | --- |
| `swirl` | `strength` −1–1; `falloff` 0.05–10 (1.4); `center` `[x, y]`, each −0.5–1.5 (`[0.5, 0.5]`); `pulse` 0–1 (0.2), the share of strength that pulses with the animation phase |
| `folds` | `displace` `"x"` or `"y"`: the coordinate that moves, driven by the other. `amplitude` 0–1.5; `waves` 1–4 objects of `amplitude` 0–0.5, `frequency` 0.1–20, `speed` −4–4 (1) and `phase` −100–100 (from the seed); `seed`; `animation` |
| `noise` | `strength` 0–1.5; `frequency` `[x, y]`, each 0.1–10 (`[1, 1]`), lower stretches features along that axis; `weight` `[x, y]`, each 0–2 (`[1, 1]`), displacement per axis; `orbit` 0–1 (0.12), the loop radius when cycling; `seed`; `animation` |
| `tilt` | `angle` −90–90 degrees; `center` |
| `ripple` | `amplitude` 0–0.5; `frequency` 0.5–60 waves per unit radius; `center`; `decay` 0–20 (0), fade with distance; `animation` |

`seed` (0–999) defaults to the recipe's `seed`. `animation` is optional:

| Field | Meaning | Allowed values |
| --- | --- | --- |
| `mode` | `static` holds still, `cycle` loops, `drift` slides steadily, `wander` follows a smooth seeded path. Defaults to the recipe's `motion_style`: `loop` cycles, `flow` drifts, and other moving styles wander | `static`, `cycle`, `drift`, `wander` |
| `direction` | Drift direction; its length scales the distance | Each −5–5 (`[-1, -0.35]`) |
| `speed` | Multiplies drift speed | 0–10 (1) |

Drift distance scales with the recipe's `motion`, so `motion: 0` stops it.
`examples/warp-layers.json` sways upright trunks with vertical folds and noise
stretched vertically, which the classic warp cannot do. A horizontal-band scene
might instead use `tilt` plus `folds` with `"displace": "y"`.
