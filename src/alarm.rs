//! Sounds and notifications for the timer, the same on every platform.
//!
//! The chimes are synthesized here, so there is nothing to bundle and no audio
//! library to build. Each is written to a WAV file in the temporary directory
//! and handed to a player the operating system already has: `afplay` on macOS,
//! PowerShell's `SoundPlayer` on Windows, and on Linux and others the first of
//! `paplay`, `pw-play`, `aplay` and `ffplay` that works. If none does, the
//! terminal bell rings instead. Loudness is the system volume; the chimes are
//! normalized close to full scale.
use std::{
    f32::consts::TAU,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread::{self, JoinHandle},
    time::Duration,
};

use tracing::{debug, info, warn};

const SAMPLE_RATE: u32 = 44_100;
/// Fraction of full scale the loudest sample reaches.
const PEAK: f32 = 0.95;

/// Which sound to play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chime {
    /// A work period is over: a bright arpeggio that climbs.
    WorkDone,
    /// A break is over: the same notes coming back down.
    BreakDone,
}

impl Chime {
    fn name(self) -> &'static str {
        match self {
            Self::WorkDone => "work-done",
            Self::BreakDone => "break-done",
        }
    }

    /// Note frequencies in hertz, in the order they start.
    fn notes(self) -> [f32; 4] {
        const C5: f32 = 523.25;
        const E5: f32 = 659.25;
        const G5: f32 = 783.99;
        const C6: f32 = 1046.5;
        match self {
            Self::WorkDone => [C5, E5, G5, C6],
            Self::BreakDone => [C6, G5, E5, C5],
        }
    }
}

/// Seconds between the starts of notes, and how long each rings.
const NOTE_SPACING: f32 = 0.2;
const RING_SECONDS: f32 = 1.4;
/// Seconds of silence-to-full at the start of a note, so it does not click.
const ATTACK: f32 = 0.004;

/// A chime as the bytes of a 16-bit mono WAV file.
///
/// Each note is a bell: its fundamental and two overtones, fading away
/// exponentially. The last note rings out for [`RING_SECONDS`].
pub fn synthesize(chime: Chime) -> Vec<u8> {
    let notes = chime.notes();
    let seconds = NOTE_SPACING * (notes.len() - 1) as f32 + RING_SECONDS;
    let count = (seconds * SAMPLE_RATE as f32) as usize;
    let mut samples = vec![0.0_f32; count];
    for (index, frequency) in notes.into_iter().enumerate() {
        let start = (index as f32 * NOTE_SPACING * SAMPLE_RATE as f32) as usize;
        for (offset, sample) in samples[start..].iter_mut().enumerate() {
            let time = offset as f32 / SAMPLE_RATE as f32;
            if time > RING_SECONDS {
                break;
            }
            let envelope = (time / ATTACK).min(1.0) * (-time * 3.2).exp();
            let tone = (TAU * frequency * time).sin()
                + 0.45 * (TAU * 2.0 * frequency * time).sin()
                + 0.2 * (TAU * 3.0 * frequency * time).sin();
            *sample += envelope * tone;
        }
    }
    let loudest = samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    let scale = PEAK * f32::from(i16::MAX) / loudest.max(f32::EPSILON);
    let pcm: Vec<u8> = samples
        .iter()
        .flat_map(|sample| ((sample * scale) as i16).to_le_bytes())
        .collect();
    wav(&pcm)
}

/// Wrap 16-bit mono PCM in a WAV header.
fn wav(pcm: &[u8]) -> Vec<u8> {
    let length = pcm.len() as u32;
    let mut file = Vec::with_capacity(44 + pcm.len());
    file.extend(b"RIFF");
    file.extend((36 + length).to_le_bytes());
    file.extend(b"WAVEfmt ");
    file.extend(16_u32.to_le_bytes()); // size of the fmt chunk
    file.extend(1_u16.to_le_bytes()); // PCM
    file.extend(1_u16.to_le_bytes()); // mono
    file.extend(SAMPLE_RATE.to_le_bytes());
    file.extend((SAMPLE_RATE * 2).to_le_bytes()); // bytes per second
    file.extend(2_u16.to_le_bytes()); // bytes per frame
    file.extend(16_u16.to_le_bytes()); // bits per sample
    file.extend(b"data");
    file.extend(length.to_le_bytes());
    file.extend(pcm);
    file
}

/// A program and its arguments.
type Invocation = (String, Vec<String>);

fn command(program: &str, args: &[&str]) -> Invocation {
    (
        program.to_owned(),
        args.iter().map(|arg| (*arg).to_owned()).collect(),
    )
}

/// The players to try for the WAV file at `path`, best first, on the operating
/// system named as in [`std::env::consts::OS`].
fn players(os: &str, path: &Path) -> Vec<Invocation> {
    let path = path.to_string_lossy();
    match os {
        "macos" => vec![command("afplay", &[&path])],
        "windows" => {
            let script = format!(
                "(New-Object Media.SoundPlayer '{}').PlaySync()",
                path.replace('\'', "''")
            );
            vec![command("powershell", &["-NoProfile", "-Command", &script])]
        }
        _ => vec![
            command("paplay", &[&path]),
            command("pw-play", &[&path]),
            command("aplay", &["-q", &path]),
            command(
                "ffplay",
                &["-nodisp", "-autoexit", "-loglevel", "quiet", &path],
            ),
        ],
    }
}

/// The command that shows a desktop notification: `osascript` on macOS, a
/// PowerShell tray balloon on Windows, and `notify-send` on Linux and similar.
fn notifier(os: &str, title: &str, body: &str) -> Invocation {
    match os {
        "macos" => {
            let quote =
                |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
            let script = format!(
                "display notification {} with title {}",
                quote(body),
                quote(title)
            );
            command("osascript", &["-e", &script])
        }
        "windows" => {
            // A balloon from a tray icon shows as a toast on Windows 10 and 11
            // without the program having to register itself. The icon has to
            // outlive the balloon, so the script waits before removing it.
            let quote = |text: &str| format!("'{}'", text.replace('\'', "''"));
            let script = format!(
                "Add-Type -AssemblyName System.Windows.Forms; \
                 Add-Type -AssemblyName System.Drawing; \
                 $icon = New-Object System.Windows.Forms.NotifyIcon; \
                 $icon.Icon = [System.Drawing.SystemIcons]::Information; \
                 $icon.Visible = $true; \
                 $icon.ShowBalloonTip(10000, {}, {}, [System.Windows.Forms.ToolTipIcon]::Info); \
                 Start-Sleep -Seconds 11; \
                 $icon.Dispose()",
                quote(title),
                quote(body)
            );
            command(
                "powershell",
                &["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script],
            )
        }
        _ => command("notify-send", &[title, body]),
    }
}

/// Run a command to completion with its output discarded. True if it ran and
/// succeeded.
fn run((program, args): &Invocation) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// How a chime ended up being played.
#[derive(Debug, PartialEq, Eq)]
pub enum Played {
    /// By this program, the first of the candidates that worked.
    Player(String),
    /// No player worked, so the terminal bell rang.
    Bell,
}

fn wav_path(chime: Chime) -> PathBuf {
    std::env::temp_dir().join(format!(
        "amber-timer-{}-{}.wav",
        std::process::id(),
        chime.name()
    ))
}

/// Play a chime without waiting for it. The thread's result says how it was
/// played; most callers can ignore it.
pub fn play(chime: Chime) -> JoinHandle<Played> {
    thread::spawn(move || {
        let path = wav_path(chime);
        if std::fs::write(&path, synthesize(chime)).is_ok() {
            for candidate in players(std::env::consts::OS, &path) {
                if run(&candidate) {
                    info!(chime = chime.name(), player = candidate.0, "chime played");
                    return Played::Player(candidate.0);
                }
                debug!(
                    chime = chime.name(),
                    player = candidate.0,
                    "chime player failed"
                );
            }
        }
        warn!(
            chime = chime.name(),
            "no audio player worked, so the terminal bell rang"
        );
        // A single byte, so it cannot split the escape sequences the screen
        // is drawn with.
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x07").and_then(|()| out.flush());
        Played::Bell
    })
}

/// Show a desktop notification without waiting for it. The thread's result is
/// whether the notifier ran; a missing or refused notification is not an error
/// for the timer, so most callers can ignore it.
pub fn notify(title: &str, body: &str) -> JoinHandle<bool> {
    let command = notifier(std::env::consts::OS, title, body);
    let (title, body) = (title.to_owned(), body.to_owned());
    thread::spawn(move || {
        let shown = run(&command);
        if shown {
            info!(title, body, "notification shown");
        } else {
            warn!(title, body, notifier = command.0, "notification failed");
        }
        shown
    })
}

/// Handle for the cross-platform dimming overlay child process.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub struct FocusOverlay {
    child: Option<std::process::Child>,
    input: Option<std::process::ChildStdin>,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl FocusOverlay {
    /// Whether the overlay child process is still alive.
    pub fn is_running(&mut self) -> bool {
        self.child
            .as_mut()
            .is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_none()))
    }

    /// Ask the overlay to close, then reap its process in the background.
    pub fn close(mut self) {
        self.request_close();
        if let Some(mut child) = self.child.take() {
            thread::spawn(move || {
                let started = std::time::Instant::now();
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) | Err(_) => break,
                        Ok(None) if started.elapsed() < Duration::from_secs(2) => {
                            thread::sleep(Duration::from_millis(50));
                        }
                        Ok(None) => {
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                    }
                }
            });
        }
    }

    fn request_close(&mut self) {
        if let Some(mut input) = self.input.take() {
            let _ = input.write_all(b"close\n").and_then(|()| input.flush());
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl Drop for FocusOverlay {
    fn drop(&mut self) {
        self.request_close();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Open the cross-platform dimming overlay without blocking the timer.
pub fn show_focus_overlay(message: &str) -> std::io::Result<Option<FocusOverlay>> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("--focus-overlay")
            .arg(message)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn()?;
        let input = child.stdin.take();
        Ok(Some(FocusOverlay {
            child: Some(child),
            input,
        }))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = message;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(wav: &[u8]) -> Vec<f32> {
        wav[44..]
            .chunks_exact(2)
            .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / f32::from(i16::MAX))
            .collect()
    }

    /// How strongly `frequency` is present in `window` (the Goertzel filter).
    fn strength(window: &[f32], frequency: f32) -> f32 {
        let coefficient = 2.0 * (TAU * frequency / SAMPLE_RATE as f32).cos();
        let (mut one, mut two) = (0.0_f32, 0.0_f32);
        for sample in window {
            let next = sample + coefficient * one - two;
            two = one;
            one = next;
        }
        (one * one + two * two - coefficient * one * two).sqrt() / window.len() as f32
    }

    fn rms(window: &[f32]) -> f32 {
        (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt()
    }

    #[test]
    fn a_chime_is_a_valid_loud_unclipped_wav_of_a_couple_of_seconds() {
        for chime in [Chime::WorkDone, Chime::BreakDone] {
            let file = synthesize(chime);
            assert_eq!(&file[0..4], b"RIFF");
            assert_eq!(&file[8..16], b"WAVEfmt ");
            let word = |at: usize| u32::from_le_bytes(file[at..at + 4].try_into().unwrap());
            let half = |at: usize| u16::from_le_bytes(file[at..at + 2].try_into().unwrap());
            assert_eq!((word(16), half(20), half(22)), (16, 1, 1)); // fmt chunk, PCM, mono
            assert_eq!(
                (word(24), word(28), half(32), half(34)),
                (44_100, 88_200, 2, 16)
            );
            assert_eq!(&file[36..40], b"data");
            // Sizes in the header match what follows.
            assert_eq!(word(40) as usize, file.len() - 44);
            assert_eq!(word(4) as usize, file.len() - 8);

            let pcm = samples(&file);
            let seconds = pcm.len() as f32 / SAMPLE_RATE as f32;
            assert!((1.8..2.4).contains(&seconds), "{chime:?}: {seconds}s");
            // As loud as it can be without clipping.
            let peak = pcm.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
            assert!((0.93..0.97).contains(&peak), "{chime:?}: peak {peak}");
            // It starts without a click and fades out to nothing.
            assert!(pcm[0].abs() < 0.01);
            assert!(rms(&pcm[pcm.len() - 2000..]) < 0.01);
        }
    }

    #[test]
    fn work_done_climbs_and_break_done_descends() {
        let window = (0.12 * SAMPLE_RATE as f32) as usize;
        for chime in [Chime::WorkDone, Chime::BreakDone] {
            let pcm = samples(&synthesize(chime));
            let notes = chime.notes();
            for (index, frequency) in notes.into_iter().enumerate() {
                // Just after a note starts, its own pitch is there and the notes
                // still to come are not, so it beats all of them.
                let start = (index as f32 * NOTE_SPACING * SAMPLE_RATE as f32) as usize + 200;
                let heard = &pcm[start..start + window];
                let own = strength(heard, frequency);
                assert!(
                    own > strength(heard, 100.0) * 10.0,
                    "{chime:?} note {index}"
                );
                for later in &notes[index + 1..] {
                    assert!(own > strength(heard, *later), "{chime:?} note {index}");
                }
            }
        }
        // The two sounds are different, and mirror images in pitch.
        assert_ne!(synthesize(Chime::WorkDone), synthesize(Chime::BreakDone));
        let (up, down) = (Chime::WorkDone.notes(), Chime::BreakDone.notes());
        assert!(up.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(down.windows(2).all(|pair| pair[0] > pair[1]));
    }

    #[test]
    fn each_platform_uses_its_own_player_and_the_file_it_was_given() {
        let path = Path::new("/tmp/chime.wav");
        assert_eq!(
            players("macos", path),
            vec![command("afplay", &["/tmp/chime.wav"])]
        );

        let linux = players("linux", path);
        assert_eq!(linux[0], command("paplay", &["/tmp/chime.wav"]));
        let programs: Vec<&str> = linux.iter().map(|(program, _)| program.as_str()).collect();
        assert_eq!(programs, ["paplay", "pw-play", "aplay", "ffplay"]);
        assert!(
            linux
                .iter()
                .all(|(_, args)| args.last().unwrap() == "/tmp/chime.wav")
        );
        // Other Unix systems get the same list as Linux.
        assert_eq!(players("freebsd", path), linux);

        // Windows runs PowerShell, with a quote in the path escaped.
        let windows = players("windows", Path::new("C:\\Temp\\it's.wav"));
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].0, "powershell");
        assert!(
            windows[0]
                .1
                .last()
                .unwrap()
                .contains("'C:\\Temp\\it''s.wav'")
        );
    }

    #[test]
    fn notifications_use_the_standard_tool_and_escape_their_text() {
        let mac = notifier("macos", "Pomodoro", "Time for a 5 minute break");
        assert_eq!(mac.0, "osascript");
        assert_eq!(
            mac.1,
            [
                "-e",
                "display notification \"Time for a 5 minute break\" with title \"Pomodoro\""
            ]
        );
        // Quotes and backslashes cannot break out of the script.
        let tricky = notifier("macos", "a\"b", "c\\d\"e");
        assert!(tricky.1[1].contains("\"a\\\"b\"") && tricky.1[1].contains("\"c\\\\d\\\"e\""));

        assert_eq!(
            notifier("linux", "Pomodoro", "Back to work"),
            command("notify-send", &["Pomodoro", "Back to work"])
        );
        assert_eq!(notifier("freebsd", "t", "b").0, "notify-send");

        // Windows runs PowerShell with the text in single quotes, a quote in
        // it doubled so it stays text.
        let windows = notifier("windows", "Pomodoro", "it's time");
        assert_eq!(windows.0, "powershell");
        let script = windows.1.last().unwrap();
        assert!(script.contains("ShowBalloonTip(10000, 'Pomodoro', 'it''s time',"));
        assert!(windows.1.iter().any(|arg| arg == "Hidden"));
    }

    #[test]
    fn a_missing_program_is_just_a_failure() {
        assert!(!run(&command("amber-no-such-player-anywhere", &[])));
        assert!(run(&command("true", &[])) || cfg!(windows));
    }
}
