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
    → five color families, lights_at, and scores for what is in the
      picture and the overall look
    → Composition (see composition-format.md)
    → deterministic Scene, then validated Recipe
    → GradientView
```

Jev chooses bounded values rather than writing JSON coordinates or rendering
code. Its answers describe generic parts of a picture (sky, land, upright
features, beams, lights, one big glow, haze) and local code turns them into
points. No scenes are named: different pictures are different answers to the
same questions. Descriptions that are a feeling rather than a place skip
the scene and use one of six broad color layouts (sky, horizon, glow,
scatter, superscatter, wash). The prompt seeds the random placement, so the same prompt and
answers always give the same picture. Seed stays 58.

The network request runs on a worker thread with a 20-second timeout. The old
gradient keeps animating while it is pending. Failures preserve the old recipe
and show a retry message. Enter is ignored while a request is already pending.
Successful descriptions are cached during the session to avoid repeated calls.
Requests are billed to the configured OpenRouter account; tests use local mocks.

## Embedding generation

`amber::generation::JevClient` is separate from the renderer. A host can construct
it with its own key and model, then call `generate(description)` on a worker.
The result exposes the validated recipe, the `Composition` Jev decided on,
playback speed, and optional API-reported cost. Configuration loading is explicit; the library never
reads `.env` merely because it is imported. Credentials are not included in
debug output or server-error messages.

For renderer-only consumers, use `default-features = false` in the Amber Cargo
dependency. This removes the optional HTTP client and generation API.

API references:
- [OpenRouter Jev Decisions tutorial](https://openrouter.ai/docs/guides/community/jev-tutorial)
- [Mesh website](https://mesh.iamnoman.com)
