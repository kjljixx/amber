# Amber

A Rust terminal app built with Ratatui and Crossterm.

## Run

```sh
cargo run
```

This starts the pomodoro timer. The gradient viewer is `cargo run --bin amber`.

Press `q`, `Esc`, or `Ctrl+C` to quit. The app restores the terminal on exit.

The preview displays a fixed half-block test pattern: cyan on the upper half
and amber on the lower half of every cell. It resizes with the terminal and
waits for input while idle. Use a terminal with truecolor and Unicode support.

If your environment sets `NO_COLOR`, run `env -u NO_COLOR cargo run` to enable
the colored preview.

The renderer roadmap is in [docs/renderer-plan.md](docs/renderer-plan.md).
