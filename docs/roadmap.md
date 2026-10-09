# Roadmap

Ideas discussed but not scheduled. Points are the only renderer primitive;
scenes (redwood forest, desert hills, city lights) should be built from
generators that emit points, plus a warp stack.

## Per-point `wobble`

An optional number on each point (0 to about 2, default 1) that scales how much
of the warp that point feels.

- The warp already computes one displacement per pixel, `delta = warped - p`.
  Each point would measure its distance from `p + wobble * delta`.
- `1` is today's behavior and is omitted from JSON, so existing recipes are
  unchanged. `0` keeps a point's region exactly as authored.
- Lets one image mix treatments: crisp round city lights (`0`), nearly upright
  trunks (`~0.2`) and fully organic hills (`1`), without lowering distortion
  for the whole picture.
- Cost should be one multiply-add per point per pixel. Untested; check that
  seams between points with different wobble stay smooth.

## Other deferred ideas

- **Element layer:** generators (band, ridge, glow, scatter, columns, shaft)
  that emit points, with named palette roles and depth-based atmospheric
  perspective. The current `Layout` enum, `superscatter_positions` and the
  per-layout motion mapping move into it.
- **Spatial culling:** a coarse grid so each pixel only evaluates nearby
  points (cutoff about `8 * radius^2` past the nearest). Needed before several
  hundred points are practical; output should match brute force within one
  color level.
- **Point limit as a validation constant:** drop the fixed `[f32; MAX_POINTS]`
  stack array in `sample()` and keep only a generous load-time limit.
- **Jev scene picking:** choose a scene template and colors per palette role
  instead of one of six layouts. Derive the warp seed from the prompt hash;
  generated recipes all use seed 58 today.
