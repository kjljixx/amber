//! Prints what the audio analysis hears (level, how much is changing, tone and
//! pulse), for tuning and for checking that capture works.
//!
//! `cargo run --offline --example listen -- [system|mic|DEVICE] [seconds] [sensitivity] [--save clip.wav]`
//!
//! `--save` also writes the sound it hears to a WAV file, to replay with the
//! `trace` example.
//! `cargo run --offline --example listen -- --list`
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use amber::audio::{Capture, Source, Status, list_devices};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let save = args.iter().position(|arg| arg == "--save").map(|at| {
        let path = args.get(at + 1).cloned();
        args.drain(at..(at + 2).min(args.len()));
        path
    });
    let save = match save {
        Some(Some(path)) => Some(PathBuf::from(path)),
        Some(None) => {
            eprintln!("listen: --save needs a file name such as clip.wav");
            return;
        }
        None => None,
    };
    if args.first().map(String::as_str) == Some("--list") {
        match list_devices() {
            Ok(devices) => devices.iter().for_each(|device| println!("{device}")),
            Err(message) => eprintln!("listen: {message}"),
        }
        return;
    }
    let source = match args.first().map(String::as_str) {
        None | Some("system") => Source::System,
        Some("mic") => Source::Microphone,
        Some(device) => Source::Named(device.to_string()),
    };
    let seconds: f32 = args
        .get(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(10.0);
    let sensitivity: f32 = args
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(1.0);

    let mut capture = Capture::start_recording(source, sensitivity, save);
    let start = Instant::now();
    let mut said = false;
    while start.elapsed().as_secs_f32() < seconds {
        match capture.status().clone() {
            Status::Starting => {}
            Status::Failed(message) => {
                eprintln!("listen: {message}");
                return;
            }
            Status::Listening(device) => {
                if !said {
                    said = true;
                    println!("listening to {device}");
                }
                let heard = capture.poll();
                let bar = "#".repeat((heard.level * 20.0).round() as usize);
                let busy = "=".repeat((heard.activity * 10.0).round() as usize);
                let tone = heard
                    .tone
                    .map_or("  - ".to_string(), |tone| format!("{tone:4.2}"));
                let beat = if heard.pulse > 0.45 { "HIT" } else { "" };
                println!(
                    "level {:4.2} {bar:<20} busy {:4.2} {busy:<10} tone {tone} pulse {:4.2} {beat}",
                    heard.level, heard.activity, heard.pulse
                );
            }
        }
        thread::sleep(Duration::from_millis(33));
    }
    println!("heard anything: {}", capture.heard());
}
