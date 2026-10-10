# Roadmap

Ideas discussed but not scheduled. Points are the only renderer primitive;
scenes (see [scene-format.md](scene-format.md)) are built from generators that
emit points, plus a warp stack.

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

- **Jev choosing scene parts:** tried in commit `732a49d` (a `Composition` of
  land, features, lights, glow and so on, filled in by Jev) and removed. It
  made recognizable places but pulled Jev's output away from plain gradients.
  Jev picks colors and one of the six layouts again; scenes remain for
  hand-written files.
- **More elements:** soft clouds or blobs, stars, a ground plane, and
  rows of repeated shapes (fences, waves). Each is one variant in
  `src/scene/elements.rs`.
- **Backdrop balance:** elements currently need `radius` and `strength` tuned
  so shapes show through gradients (see Layering in the scene docs). A helper
  that sets a backdrop's weight from the shapes in front would remove that.
- **Retire the six layouts' special casing:** `grid` keeps their random-draw
  order so output stays identical. If exact reproduction stops mattering, they
  could be rebuilt from the newer elements with fewer points.
- **Spatial culling:** a coarse grid so each pixel only evaluates nearby
  points (cutoff about `8 * radius^2` past the nearest). Needed before several
  hundred points are practical; output should match brute force within one
  color level.
- **Point limit as a validation constant:** drop the fixed `[f32; MAX_POINTS]`
  stack array in `sample()` and keep only a generous load-time limit.
