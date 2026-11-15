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

## Scheduled work

Copy `amber.example.json` to `amber.json` (or pass `--config FILE`) and list times of day:

```json
{ "scheduled_work": ["09:00", "14:00"] }
```

At each time, every day: on the home screen Amber starts a task called "Choose a task in Amber"; on a paused task it resumes; otherwise it does nothing.

## Sync across computers

Sync is on by default, once there is a key. Make one, and put it in `.env` on every computer as `SYNC_SECRET`:

```sh
cargo run -- --sync-keygen
```

Without a key Amber runs on its own and shows "Sync is off" at the start. `--no-sync` turns sync off even when a key is set.

`--sync-keygen` prints 64 hex characters (32 random bytes). Use that, not a typed word: a short secret can be guessed from a recording of the network.

The first one to start is the primary. The others find it on the local network and mirror its timer, task and gradient. Pause, reset, choosing a task and going home work from any of them; the primary acts on them. Scheduled work runs on the primary only, and every computer shows when it starts. Audio, the gradient's animation and the focus check stay on each computer.

If the primary stops, a follower takes over within a few seconds. If two primaries meet again, for example after a laptop wakes, the group that was used last stays primary. Using Amber on any computer counts: pausing, resetting, choosing a task or opening the task menu. If nobody used anything, the original primary stays. If computers cannot find each other, name one with `--sync-peer <host>`; `--sync-port <number>` changes the port (default 47821). Sync logs go to `sync.log`.

Everything sent between computers is encrypted and checked for changes (Noise, `Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s`, with `snow`). A computer with a different key cannot join, read or change anything, and the key itself is never sent. Everyone who has the key is trusted. The UDP messages that find the primary are not signed, so someone on the network can disturb that search but cannot join. All computers must run the same version. If a computer is lost, make a new key for the others. See `docs/sync-security-plan.html`.

## Cool features
On startup, a song lyric will be randomly chosen to be the seed for the gradient, based on the current time, season, weather, etc.

Gradients will react to audio/music, the beat, the swells, etc.

Gradients can be described w/ natural language!

Tasks connect to Todoist and you can choose a task from todoist!

With no task set, the central clock is just a clock; with a task set it's a pomodoro clock

Decision models check if you're on task or not, if you have a task set.

## License

GPL-3.0, see `LICENSE`.
