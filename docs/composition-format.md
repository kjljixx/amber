# Compositions

A composition describes a picture by what is in it, as a handful of colors and
small bounded numbers. It is what Jev fills in, and it is also a compact way to
write a scene by hand. `Composition::to_scene` expands it into the elements
described in [scene-format.md](scene-format.md), so nothing below it changes.

```sh
cargo run --offline -- --scene examples/compositions/city.json
```

`--scene` and `Scene::load` accept a composition file wherever they accept a
scene. Four examples are in `examples/compositions/`.

## Why parts, not scene names

Nothing in a composition names a forest or a city. A forest is dense upright
features with beams of light over a floor; a city is features with lights in
front of a glowing horizon; a lighthouse is a dark sea, one tall feature and a
big light. New kinds of pictures are new answers, not new code.

## Fields

Every level is a number from 0 up to its maximum. Values in between blend the
neighboring settings, so answers stay smooth, and below 0.5 a part is absent
(beams and lights need 1.5).

| Field | Range | Meaning |
|---|---|---|
| `colors` | required | `sky`, `horizon`, `ground`, `feature`, `light`: a family name or `#rrggbb`. Parts use shades of these. `horizon` is also the haze color. |
| `horizon_height` | 0–4 (2) | Where sky meets ground: 0 high in the picture, 4 very low. |
| `land` | 0–4 (0) | 0 none, 1 flat (water, plains), 2 gentle hills, 3 rolling hills, 4 jagged mountains. 2 and up add ridges. |
| `features` | 0–4 (0) | Upright things (trunks, towers): none, a few, several, many, a dense crowd. Split into far, middle and near rows. |
| `feature_width` | 0–3 (1) | Very thin to massive. |
| `rays` | 0–3 (0) | Slanted beams of light; they need 1.5 or more, since a lone streak looks like a mistake. |
| `lights` | 0–4 (0) | Small lights (windows, stars): none to a great many. Needs 1.5 or more, since Jev scores prompts with no lights up to about 1.2. |
| `light_size` | 0–3 (1) | Pinpoints to large orbs. |
| `light_glow` | 0–3 (0) | How far each light spills: none, then core and bloom, plus a few faint orbs behind. |
| `lights_at` | `sky`, `horizon`, `ground`, `everywhere` | Where the lights sit. |
| `glow` | 0–3 (0) | One big source of light: none, faint, bright, blazing. |
| `glow_height` | 0–3 (0) | On the horizon to overhead. |
| `clouds` | 0–3 (0) | Soft horizontal wisps across the sky: none, a few, several, a sky full. |
| `atmosphere` | 0–3 (0) | Haze: crystal clear to thick fog. |
| `concrete` | 0–3 (3) | How much this is a place you could see rather than a feeling or idea. Below 1.5 the picture is not a scene at all: it is the original broad soft color regions, arranged by `layout`. Above it, scenery (features, beams, lights, glow) fades in with the score. |
| `layout` | `sky`, `horizon`, `glow`, `scatter` (default), `superscatter`, `wash` | How a feeling's colors are arranged when `concrete` is low. `sky`, `horizon` and `glow` are the original layouts with the same names; the sky, horizon and ground colors become their main, second and accent colors. |
| `style` | as in a scene | Lightness, saturation, contrast, energy and focus. |

Flat land (water or plains) gets a few soft horizontal streaks of sky light.
Hills are layered: the farther ones are darker, hazier versions of the horizon
color, and only the nearest takes the ground color.

When a scene has land, features or lights, the warp is lowered, the motion flows,
softness is capped and contrast is raised to at least 2.5, so layers and shapes
stay recognizable. A picture of only sky and glow keeps the full warp and the
style as asked.

## Questions Jev answers

One request asks the five colors, `lights_at`, and one score per level above
plus the five style scores. The number of labels on each score question is its
maximum plus one; a test keeps the two in step.
