# Scenes

A scene describes a picture such as hills at sunset, a misty forest, or a city
at night as a palette of named color roles plus a list of elements. Compiling it
produces an ordinary [recipe](recipe-format.md): the renderer only ever sees
points, and each element is a small generator that emits some of them.

```sh
cargo run --offline -- --scene desert_hills      # a bundled scene
cargo run --offline -- --scene my-scene.json     # your own file
```

`--scene` takes a file path if one exists, otherwise a bundled scene name:
`desert_hills`, `redwood_forest`, `city_lights`. Their JSON lives in
`src/scene/templates/`. Scenes are deterministic: the same file always compiles
to the same recipe. In Rust, `amber::Scene` deserializes the same JSON and
`Scene::compile()` returns the `Recipe`.

Unknown fields are rejected, and every value is range-checked with a message that
names the element, for example `element 2 (ridge): spacing must be finite and
between 0.02 and 0.5`.

## Scene fields

| Field | Meaning | Allowed values |
| --- | --- | --- |
| `palette` | Named color roles that elements refer to | 1–32 roles; each a family name or `#rrggbb` |
| `elements` | Emitted in order | 1–32 elements |
| `style` | Global look; see below | Optional |
| `haze` | Fades distant elements toward one role | Optional |
| `variation` | Seeds random placement and color variation; change it for a different arrangement of the same design | Integer, default 1 |
| `softness`, `distortion`, `swirl`, `motion` | Override the recipe setting of the same name | Optional; recipe ranges |
| `motion_style` | Override the animation behavior | `loop`, `flow`, `wander` (default `wander`) |
| `warp_seed` | The recipe `seed` that shapes the warp | 0–999, default 58 |
| `warp` | Extra [warp layers](recipe-format.md#warp-layers) | Optional |

Colors are one of: a family name (`red`, `coral`, `orange`, `amber`, `yellow`,
`lime`, `green`, `teal`, `cyan`, `sky`, `blue`, `indigo`, `violet`, `magenta`,
`pink`, `brown`, `cream`, `white`, `grey`, `black`), which follows the scene's
`style`, or an exact `#rrggbb`, which keeps its own chroma and is only moved
lighter or darker by `shade`. Because elements name roles, changing the palette
recolors a whole scene without moving anything.

`style` scores the overall feel, as Jev's decisions do (all optional):

| Field | Meaning | Allowed values (default) |
| --- | --- | --- |
| `lightness` | Very dark to very light | 0–4 (2) |
| `saturation` | Nearly neutral to intensely saturated | 0–4 (2) |
| `contrast` | How far `shade` moves colors | 0–3 (1.5) |
| `energy` | Warp strength and motion, unless overridden | 0–4 (2) |
| `focus` | Edge softness, unless overridden | 0–3 (1.5) |

`haze` has a `role`, a `strength` (0–1, default 0.75) and a `shade` (−1–1,
default 0.3). An element with `depth` between 0 and 1 is pulled that fraction
of `strength` toward the haze color, so distant hills turn pale and trunks fade
into mist. Elements with `depth: 0`, and scenes without a `haze`, are
unaffected.

## Elements

Every element has a `type`. `shade` is lighter (positive) or darker (negative),
scaled by `contrast`, roughly −1 to 1. Each element also adds a little random
variation to color, so no two points match exactly.

| `type` | What it makes | Fields (defaults in parentheses) |
| --- | --- | --- |
| `point` | One point | `position`, `role`, `shade` (0), `depth` (0), `radius` (1), `stretch` (1), `angle` (0), `strength` (1): the point attributes from the [recipe format](recipe-format.md) |
| `gradient` | Wide bands blending from one role to another | `from`, `to`, `y` (`[0, 1]`), `rows` 2–32 (8), `shade` `[first, last]` (`[0, 0]`), `depth` `[first, last]` (`[0, 0]`), `radius` (0.4), `stretch` (6), `strength` (0.5), `x` (0.5) |
| `ridge` | A wavy horizon of hills; stack several with growing `depth` | `role`, `y` crest height, `amplitude` 0–0.5 (0.06), `frequency` waves across the image 0.2–8 (1.5), `roughness` 0–1 (0.4), `x` (`[-0.15, 1.15]`), `spacing` 0.02–0.5 (0.1), `inset` below the crest (0.06), `radius` (0.5), `stretch` (3), `shade` (0), `shade_jitter` (0.1), `depth` (0), `follow` tilt points with the slope (true) |
| `columns` | Upright shapes: trunks, poles, buildings | `role`, `count` 1–64, `x` (`[0, 1]`), `y` center (0.6), `height` `[min, max]` as a multiple of `width`, 1–10 (`[4, 6]`), `width` 0.05–2 (0.25), `taper` how much the farthest narrow, 0–1 (0.5), `depth` `[near, far]` each column draws from (`[0, 0]`), `lean` degrees (2), `jitter` (0.6), `shade` (0), `shade_jitter` (0.1), `strength` (1) |
| `shafts` | Slanted streaks of light | `role`, `count` 1–24, `x` (`[0.1, 0.9]`), `y` (`[0.15, 0.55]`), `angle` (−62), `angle_jitter` (4), `length` 1–10 (8), `width` 0.05–1 (0.07), `shade` (0.6), `depth` (0), `strength` (1.5) |
| `lights` | Many small bright points, kept apart | `count` 1–200, `roles` 1–8 of `role` and `weight` (1), `x` (`[0, 1]`), `y` (`[0.6, 1]`), `radius` `[min, max]` 0.05–2 (`[0.06, 0.11]`), `strength` (6), `shade` `[min, max]` (`[0.4, 0.9]`), `depth` (0) |
| `grid` | The original eighteen-point layouts | `rule`: `sky`, `horizon`, `glow`, `scatter`, `superscatter`, `wash`. Uses the roles `main`, `second` and `accent` |

A scene may make at most 256 points in total. Positions that fall off the image
are clamped to the renderer's range (−0.5 to 1.5).

## Layering

Points compete by distance relative to their own size, so a shape only shows
through a backdrop if its `radius` is comparable to the backdrop's. A tall,
thin column is far from its own center in these terms, while the bands of a
gradient are always close by, so thin shapes drawn over a dense gradient fade
into it. That is why gradient bands default to a small `radius` (0.4) and a low
`strength` (0.5), and why the foreground elements default to larger radii. If a
shape is disappearing, raise its `radius` or `strength`, or lower the
backdrop's. Fewer, softer backdrop bands also reduce faint horizontal banding
across tall shapes.

For hills, stack `ridge` elements from far to near with decreasing `depth` and
darker `shade`, over a `gradient` sky. For a forest, use several `columns`
elements with different `width` and `depth` ranges. Wide trunks go in front,
thin hazy ones behind. City windows are `lights` over dark `columns`.

## The original layouts

`Design::to_recipe` (Jev's decisions) builds a scene with a `grid` element: its
three hues become the roles `main`, `second` and `accent`, and the prompt seeds
`variation`. Output is identical to the layouts before scenes existed; the random
draws happen in the same order.
