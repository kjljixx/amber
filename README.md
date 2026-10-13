# Amber

A Rust terminal app built with Ratatui and Crossterm.

## Run

```sh
cargo run
```

This starts `amber`: the home clock and the pomodoro. The gradient viewer is `cargo run --bin gradient`.

Press `Ctrl+C` to quit `amber` (`q` and `Esc` never quit it); the gradient viewer also quits with `q` or `Esc`. The app restores the terminal on exit.

The preview displays a fixed half-block test pattern: cyan on the upper half
and amber on the lower half of every cell. It resizes with the terminal and
waits for input while idle. Use a terminal with truecolor and Unicode support.

If your environment sets `NO_COLOR`, run `env -u NO_COLOR cargo run` to enable
the colored preview.

The renderer roadmap is in [docs/renderer-plan.md](docs/renderer-plan.md).
