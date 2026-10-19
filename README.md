# Amber

A beautifully designed TUI productivity hub made with Ratatui

## Run

```sh
cargo run
```

Press `t` to choose tasks

Put in openrouter (used to decide) and todoist api keys in for full features

## Lyrics setup

The startup gradient is seeded by a lyric line from `assets/phrases.txt`, which isn't in the repo. Build it once. It downloads the [Genius lyrics dataset](https://huggingface.co/datasets/theelderemo/genius-lyrics-cleaned) from Hugging Face (several GB) and keeps 200,000 rock and pop lines:

```sh
python3 -m venv .venv && source .venv/bin/activate && pip install -r scripts/requirements.txt && python scripts/05_build_phrase_pool.py
```

`scripts/01_download_lyrics.py` saves the full dataset as `lyrics.csv` (4.5 GB) if you want to browse it. Amber doesn't need that file.

## Cool features
On startup, a song lyric will be randomly chosen to be the seed for the gradient, based on the current time, season, weather, etc.

Gradients will react to audio/music, the beat, the swells, etc.

Gradients can be described w/ natural language!

Tasks connect to Todoist and you can choose a task from todoist!

With no task set, the central clock is just a clock; with a task set it's a pomodoro clock

Decision models check if you're on task or not, if you have a task set.

## License

GPL-3.0, see `LICENSE`.
