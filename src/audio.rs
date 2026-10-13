//! Turns sound into an [`Excitement`] for a [`GradientView`](crate::GradientView).
//!
//! [`Analyzer`] is pure: feed it mono samples and ask what the music is doing.
//! With the `audio` feature, [`Capture`] listens to the machine's own output (or
//! a microphone) and runs an analyzer on what it hears. Nothing here records,
//! stores or sends sound anywhere.

use crate::Excitement;

const FFT_SIZE: usize = 1024;
/// New samples between two looks at the spectrum (about 10 ms at 48 kHz).
const HOP: usize = 512;
/// Spectrum range that carries the beat: kicks and snares, not hiss.
const BEAT_RANGE_HZ: (f32, f32) = (30.0, 4000.0);

/// Quietest peak the loudness is measured against, so hiss never fills the scale.
const MIN_REFERENCE: f32 = 0.01;
/// Below this RMS the picture is told it is silent.
const SILENCE: f32 = 0.002;
/// How long the loudness reference holds a peak before forgetting it.
const REFERENCE_SECONDS: f32 = 5.0;
const LEVEL_ATTACK_SECONDS: f32 = 0.12;
const LEVEL_RELEASE_SECONDS: f32 = 0.6;
/// A hit rises over a couple of frames, not at once, and fades slowly enough
/// to read as a swell and an ease, not a flash.
const PULSE_ATTACK_SECONDS: f32 = 0.025;
const PULSE_DECAY_SECONDS: f32 = 0.45;
/// How fast the usual amount of change in the spectrum is learned.
const FLUX_SECONDS: f32 = 1.0;
/// A hit rises this many typical deviations above the usual change, and is at
/// least `PULSE_FLOOR`. Any onset counts, a plucked note as well as a kick, but
/// a soft one is a small pulse. Shimmer wanders within a deviation or two and
/// does not count.
const FLUX_DEVIATIONS: f32 = 4.0;
/// Spectral flux (see `Analyzer::hop`) below this is the ordinary shimmer of
/// held sound, vibrato and reverb, which are not hits however still the music.
const PULSE_FLOOR: f32 = 0.06;
const MIN_BEAT_GAP_SECONDS: f32 = 0.18;
/// Pulse strength of an onset that only just clears the threshold.
const SOFTEST_PULSE: f32 = 0.25;
/// Flux where `activity` starts to rise and where it reaches 1.
const ACTIVITY_FROM: f32 = 0.03;
const ACTIVITY_TO: f32 = 0.12;
/// Activity follows the flux smoothed over this long, so that it tracks phrases
/// and not single frames.
const FLUX_SLOW_SECONDS: f32 = 0.3;
/// Each hit adds this much (times its strength) to a tally of recent hits, which
/// fades over `DENSITY_SECONDS`: a run of notes fills it, a lone chord does not.
const DENSITY_STEP: f32 = 0.3;
const DENSITY_SECONDS: f32 = 1.0;
const ACTIVITY_ATTACK_SECONDS: f32 = 0.1;
const ACTIVITY_RELEASE_SECONDS: f32 = 0.8;
/// The spectral centroid is followed between these two frequencies.
const TONE_RANGE_HZ: (f32, f32) = (100.0, 4000.0);
const TONE_SECONDS: f32 = 0.3;
/// The tone ignores wobbles smaller than this and follows only the excess, so a
/// held note with vibrato stays put.
const TONE_DEADBAND: f32 = 0.012;

/// Listens to a stream of samples for loudness and beats.
pub struct Analyzer {
    sample_rate: f32,
    sensitivity: f32,
    window: Vec<f32>,
    cos: Vec<f32>,
    sin: Vec<f32>,
    ring: Vec<f32>,
    write: usize,
    since_hop: usize,
    hops: u64,
    re: Vec<f32>,
    im: Vec<f32>,
    previous: Vec<f32>,
    flux_mean: f32,
    flux_deviation: f32,
    flux_slow: f32,
    density: f32,
    tone_smooth: Option<f32>,
    reference: f32,
    level: f32,
    /// What the last hit asks of the pulse; it fades by itself.
    hit: f32,
    pulse: f32,
    held_pulse: f32,
    activity: f32,
    tone: Option<f32>,
    since_beat: f32,
    hits: u64,
    heard: bool,
}

impl Analyzer {
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate.max(8_000) as f32;
        let tau = std::f32::consts::TAU;
        let bins = |hz: f32| ((hz * FFT_SIZE as f32 / sample_rate) as usize).clamp(1, FFT_SIZE / 2);
        let (low, high) = (bins(BEAT_RANGE_HZ.0), bins(BEAT_RANGE_HZ.1));
        Self {
            sample_rate,
            sensitivity: 1.0,
            window: (0..FFT_SIZE)
                .map(|i| 0.5 - 0.5 * (tau * i as f32 / FFT_SIZE as f32).cos())
                .collect(),
            cos: (0..FFT_SIZE / 2)
                .map(|i| (tau * i as f32 / FFT_SIZE as f32).cos())
                .collect(),
            sin: (0..FFT_SIZE / 2)
                .map(|i| -(tau * i as f32 / FFT_SIZE as f32).sin())
                .collect(),
            ring: vec![0.0; FFT_SIZE],
            write: 0,
            since_hop: 0,
            hops: 0,
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            previous: vec![0.0; high.saturating_sub(low) + 1],
            flux_mean: 0.0,
            flux_deviation: 0.0,
            flux_slow: 0.0,
            density: 0.0,
            tone_smooth: None,
            reference: MIN_REFERENCE,
            level: 0.0,
            hit: 0.0,
            pulse: 0.0,
            held_pulse: 0.0,
            activity: 0.0,
            tone: None,
            since_beat: MIN_BEAT_GAP_SECONDS,
            hits: 0,
            heard: false,
        }
    }

    /// Scales the level (not the beats): above 1 for quiet playback, below for loud.
    pub fn with_sensitivity(mut self, sensitivity: f32) -> Self {
        if sensitivity.is_finite() && sensitivity > 0.0 {
            self.sensitivity = sensitivity;
        }
        self
    }

    /// True once any sound at all has arrived. Silence from a source that was
    /// supposed to be playing usually means the capture was not allowed.
    pub fn heard(&self) -> bool {
        self.heard
    }

    /// How many hits have been found since the start.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Feed mono samples in -1..1. Any chunk size works.
    pub fn push(&mut self, samples: &[f32]) {
        for &sample in samples {
            let sample = if sample.is_finite() { sample } else { 0.0 };
            if sample.abs() > 1e-4 {
                self.heard = true;
            }
            self.ring[self.write] = sample;
            self.write = (self.write + 1) % FFT_SIZE;
            self.since_hop += 1;
            if self.since_hop == HOP {
                self.since_hop = 0;
                self.hop();
            }
        }
    }

    /// The state to show now. The pulse is the strongest one since the last
    /// call, so a beat that decayed between two frames is not missed.
    pub fn take(&mut self) -> Excitement {
        let excitement = Excitement {
            level: self.level,
            pulse: self.held_pulse.max(self.pulse),
            activity: self.activity,
            tone: self.tone,
        };
        self.held_pulse = self.pulse;
        excitement
    }

    fn hop(&mut self) {
        let dt = HOP as f32 / self.sample_rate;
        let blend = |seconds: f32| 1.0 - (-dt / seconds).exp();

        // The last FFT_SIZE samples, oldest first.
        let mut energy = 0.0;
        for i in 0..FFT_SIZE {
            let sample = self.ring[(self.write + i) % FFT_SIZE];
            energy += sample * sample;
            self.re[i] = sample * self.window[i];
            self.im[i] = 0.0;
        }
        let rms = (energy / FFT_SIZE as f32).sqrt();
        self.fft();

        // Loudness against the recent peak, so quiet and loud sources both reach 1.
        self.reference = rms.max(self.reference * (-dt / REFERENCE_SECONDS).exp());
        let target = if rms < SILENCE {
            0.0
        } else {
            let relative = rms / self.reference.max(MIN_REFERENCE);
            (relative * relative * self.sensitivity).clamp(0.0, 1.0)
        };
        let rate = blend(if target > self.level {
            LEVEL_ATTACK_SECONDS
        } else {
            LEVEL_RELEASE_SECONDS
        });
        self.level += (target - self.level) * rate;

        // How much new sound appeared across the spectrum since the last look,
        // and where in the spectrum the sound is centered.
        let bin_hz = self.sample_rate / FFT_SIZE as f32;
        // Quiet music is lifted before the logarithm, so that how much changes
        // does not depend on how loud the volume is set.
        let drive = 15.0 / self.reference.clamp(0.1, 0.6);
        let low = ((BEAT_RANGE_HZ.0 / bin_hz) as usize).clamp(1, FFT_SIZE / 2);
        let (mut flux, mut weight, mut weighted_hz) = (0.0, 0.0, 0.0);
        for (slot, bin) in self.previous.iter_mut().zip(low..) {
            let amplitude = self.re[bin].hypot(self.im[bin]) / (FFT_SIZE as f32 / 4.0);
            let now = (1.0 + drive * amplitude).ln();
            flux += (now - *slot).max(0.0);
            *slot = now;
            let hz = bin as f32 * bin_hz;
            if (TONE_RANGE_HZ.0..=TONE_RANGE_HZ.1).contains(&hz) {
                // Squared, so the strong partials of a melody outweigh the skirts.
                weight += now * now;
                weighted_hz += now * now * hz;
            }
        }
        flux /= self.previous.len() as f32;

        self.hops += 1;
        self.since_beat += dt;
        self.hit *= (-dt / PULSE_DECAY_SECONDS).exp();
        self.density *= (-dt / DENSITY_SECONDS).exp();
        let threshold = (self.flux_mean + FLUX_DEVIATIONS * self.flux_deviation).max(PULSE_FLOOR);
        // The first looks compare against an empty spectrum, which is not a hit.
        let warm = self.hops > (FFT_SIZE / HOP) as u64 + 1;
        let sounding = warm && rms >= SILENCE;
        if sounding && flux > threshold && self.since_beat >= MIN_BEAT_GAP_SECONDS {
            let strength = SOFTEST_PULSE
                + (1.0 - SOFTEST_PULSE) * ((flux - threshold) / threshold).clamp(0.0, 1.0);
            self.hit = self.hit.max(strength);
            self.density = (self.density + DENSITY_STEP * strength).min(1.0);
            self.since_beat = 0.0;
            self.hits += 1;
        }

        // How much the sound is changing: how many hits there have been lately, or
        // how much the spectrum churns above its ordinary shimmer.
        let target = if sounding {
            let churn = (self.flux_slow - ACTIVITY_FROM) / (ACTIVITY_TO - ACTIVITY_FROM);
            self.density.max(churn).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let rate = blend(if target > self.activity {
            ACTIVITY_ATTACK_SECONDS
        } else {
            ACTIVITY_RELEASE_SECONDS
        });
        self.activity += (target - self.activity) * rate;

        // Where that sound sits between dark and bright, 0..1 over octaves. It
        // moves as the melody does; in silence there is no tone at all.
        self.tone = if sounding && weight > 0.0 {
            let octaves = (weighted_hz / weight / TONE_RANGE_HZ.0).log2();
            let raw = (octaves / (TONE_RANGE_HZ.1 / TONE_RANGE_HZ.0).log2()).clamp(0.0, 1.0);
            // Smoothed first, so a vibrato or beating partials do not wag the
            // picture, then moved only by changes that clear the deadband.
            let smooth = match self.tone_smooth {
                Some(smooth) => smooth + (raw - smooth) * blend(TONE_SECONDS),
                None => raw,
            };
            self.tone_smooth = Some(smooth);
            Some(match self.tone {
                Some(tone) => {
                    let excess = (smooth - tone).abs() - TONE_DEADBAND;
                    if excess > 0.0 {
                        tone + (smooth - tone).signum() * excess
                    } else {
                        tone
                    }
                }
                None => smooth,
            })
        } else {
            self.tone_smooth = None;
            None
        };
        // Hits themselves are left out of what counts as usual.
        if flux <= threshold {
            self.flux_deviation +=
                ((flux - self.flux_mean).abs() - self.flux_deviation) * blend(FLUX_SECONDS);
            self.flux_mean += (flux - self.flux_mean) * blend(FLUX_SECONDS);
        }
        self.flux_slow += (flux - self.flux_slow) * blend(FLUX_SLOW_SECONDS);
        self.pulse += (self.hit - self.pulse) * blend(PULSE_ATTACK_SECONDS);
        self.held_pulse = self.held_pulse.max(self.pulse);
    }

    /// In-place radix-2 transform of `re`/`im`.
    fn fft(&mut self) {
        let n = FFT_SIZE;
        let bits = n.trailing_zeros();
        for i in 0..n {
            let j = i.reverse_bits() >> (usize::BITS - bits);
            if j > i {
                self.re.swap(i, j);
                self.im.swap(i, j);
            }
        }
        let mut size = 2;
        while size <= n {
            let (half, step) = (size / 2, n / size);
            for start in (0..n).step_by(size) {
                for k in 0..half {
                    let (c, s) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = self.re[b] * c - self.im[b] * s;
                    let ti = self.re[b] * s + self.im[b] * c;
                    self.re[b] = self.re[a] - tr;
                    self.im[b] = self.im[a] - ti;
                    self.re[a] += tr;
                    self.im[a] += ti;
                }
            }
            size *= 2;
        }
    }
}

/// Where to listen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Whatever the machine is playing. macOS (14.6 or later, with permission
    /// to record system audio) and Windows can tap the default output; other
    /// systems fall back to a "monitor" input if there is one.
    System,
    /// The default microphone, which hears speakers only acoustically.
    Microphone,
    /// The first device whose name contains this text (any case).
    Named(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Still opening the device, which may wait on a permission prompt.
    Starting,
    /// Capturing from this device.
    Listening(String),
    Failed(String),
}

#[cfg(feature = "audio")]
pub use capture::{Capture, list_devices};

#[cfg(feature = "audio")]
mod capture {
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{Device, Error, ErrorKind, FromSample, Sample, SampleFormat};
    use tracing::{debug, info, warn};

    use super::{Analyzer, Source, Status};
    use crate::Excitement;

    /// How often what was heard is summed up in the log.
    const SUMMARY_EVERY: Duration = Duration::from_secs(10);

    /// What the polls since the last log line saw.
    struct Summary {
        since: Instant,
        polls: u32,
        level_sum: f32,
        level_max: f32,
        activity_sum: f32,
        beats: u32,
    }

    impl Summary {
        fn new() -> Self {
            Self {
                since: Instant::now(),
                polls: 0,
                level_sum: 0.0,
                level_max: 0.0,
                activity_sum: 0.0,
                beats: 0,
            }
        }

        fn add(&mut self, excitement: &Excitement, hits: u32) {
            self.polls += 1;
            self.level_sum += excitement.level;
            self.level_max = self.level_max.max(excitement.level);
            self.activity_sum += excitement.activity;
            self.beats += hits;
        }

        /// Log the summary and start a new one, once it covers long enough.
        fn log_when_due(&mut self) {
            if self.since.elapsed() < SUMMARY_EVERY || self.polls == 0 {
                return;
            }
            let polls = self.polls as f32;
            info!(
                seconds = self.since.elapsed().as_secs(),
                level_mean = self.level_sum / polls,
                level_max = self.level_max,
                activity_mean = self.activity_sum / polls,
                beats = self.beats,
                "audio heard"
            );
            *self = Self::new();
        }
    }

    #[derive(Default)]
    struct Shared {
        analyzer: Mutex<Option<Analyzer>>,
        problem: Mutex<Option<String>>,
    }

    /// Listens on its own thread; the owner polls for the current excitement.
    /// Dropping it stops the capture.
    pub struct Capture {
        shared: Arc<Shared>,
        opened: mpsc::Receiver<Result<String, String>>,
        status: Status,
        summary: Summary,
        hits_counted: u64,
        started: Instant,
        _stop: mpsc::Sender<()>,
    }

    impl Capture {
        pub fn start(source: Source, sensitivity: f32) -> Self {
            info!(?source, sensitivity, "audio capture starting");
            let shared = Arc::new(Shared::default());
            let (opened_tx, opened) = mpsc::channel();
            let (stop, stopped) = mpsc::channel::<()>();
            let worker = Arc::clone(&shared);
            thread::spawn(move || match open(&source, sensitivity, &worker) {
                Ok((stream, name)) => {
                    info!(device = name, "audio capture opened");
                    let _ = opened_tx.send(Ok(name));
                    // Keep the stream alive until the Capture is dropped.
                    let _ = stopped.recv();
                    drop(stream);
                }
                Err(message) => {
                    warn!(%message, "audio capture could not open");
                    let _ = opened_tx.send(Err(message));
                }
            });
            Self {
                shared,
                opened,
                status: Status::Starting,
                summary: Summary::new(),
                hits_counted: 0,
                started: Instant::now(),
                _stop: stop,
            }
        }

        pub fn status(&mut self) -> &Status {
            if self.status == Status::Starting
                && let Ok(result) = self.opened.try_recv()
            {
                self.status = match result {
                    Ok(name) => Status::Listening(name),
                    Err(message) => Status::Failed(message),
                };
            }
            if let Status::Listening(_) = self.status
                && let Some(problem) = self.shared.problem.lock().unwrap().take()
            {
                warn!(%problem, "audio capture failed while listening");
                self.status = Status::Failed(problem);
            }
            &self.status
        }

        /// Calm until listening has started.
        pub fn poll(&mut self) -> Excitement {
            if !matches!(self.status(), Status::Listening(_)) {
                return Excitement::default();
            }
            let (excitement, hits) = match self.shared.analyzer.lock().unwrap().as_mut() {
                Some(analyzer) => (analyzer.take(), analyzer.hits()),
                None => (Excitement::default(), self.hits_counted),
            };
            let new_hits = u32::try_from(hits - self.hits_counted).unwrap_or(u32::MAX);
            self.hits_counted = hits;
            self.summary.add(&excitement, new_hits);
            self.summary.log_when_due();
            excitement
        }

        /// False while listening has produced only silence.
        pub fn heard(&self) -> bool {
            self.shared
                .analyzer
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(Analyzer::heard)
        }
    }

    impl Drop for Capture {
        fn drop(&mut self) {
            info!(
                seconds = self.started.elapsed().as_secs(),
                status = ?self.status,
                "audio capture stopped"
            );
        }
    }

    /// Names that `Source::Named` can match, one per line of output.
    pub fn list_devices() -> Result<Vec<String>, String> {
        let host = cpal::default_host();
        let devices = host.devices().map_err(|error| error.to_string())?;
        Ok(devices
            .map(|device| {
                let kind = match (device.supports_input(), device.supports_output()) {
                    (true, true) => "input+output",
                    (true, false) => "input",
                    _ => "output",
                };
                format!("{} ({kind})", describe(&device))
            })
            .collect())
    }

    fn describe(device: &Device) -> String {
        device
            .description()
            .map(|description| description.name().to_string())
            .or_else(|_| device.id().map(|id| id.to_string()))
            .unwrap_or_else(|_| "unnamed device".to_string())
    }

    fn find(host: &cpal::Host, text: &str) -> Option<Device> {
        let wanted = text.to_lowercase();
        host.devices().ok()?.find(|device| {
            describe(device).to_lowercase().contains(&wanted)
                || device
                    .id()
                    .is_ok_and(|id| id.to_string().to_lowercase().contains(&wanted))
        })
    }

    fn choose(host: &cpal::Host, source: &Source) -> Result<Device, String> {
        let missing = |what: &str| format!("no {what} found");
        match source {
            Source::Microphone => host
                .default_input_device()
                .ok_or_else(|| missing("microphone")),
            Source::Named(text) => {
                find(host, text).ok_or_else(|| format!("no audio device matches \"{text}\""))
            }
            Source::System if cfg!(any(target_os = "macos", target_os = "windows")) => host
                .default_output_device()
                .ok_or_else(|| missing("output device")),
            Source::System => find(host, "monitor")
                .or_else(|| host.default_input_device())
                .ok_or_else(|| missing("monitor or input device")),
        }
    }

    fn open(
        source: &Source,
        sensitivity: f32,
        shared: &Arc<Shared>,
    ) -> Result<(cpal::Stream, String), String> {
        let host = cpal::default_host();
        let device = choose(&host, source)?;
        let name = describe(&device);
        // An output device is tapped as a loopback: the input config does not
        // exist, so ask for its output config instead.
        let config = if device.supports_input() {
            device.default_input_config()
        } else {
            device.default_output_config()
        }
        .map_err(|error| format!("{name}: {error}"))?;
        let channels = usize::from(config.channels()).max(1);
        info!(
            device = name,
            sample_rate = config.sample_rate(),
            channels,
            format = %config.sample_format(),
            "audio device chosen"
        );
        *shared.analyzer.lock().unwrap() =
            Some(Analyzer::new(config.sample_rate()).with_sensitivity(sensitivity));

        let problems = Arc::clone(shared);
        let on_error = move |error: Error| match error.kind() {
            // The stream carries on (or is rerouted) after these.
            ErrorKind::DeviceChanged | ErrorKind::Xrun | ErrorKind::RealtimeDenied => {
                debug!(kind = ?error.kind(), "audio stream event ignored");
            }
            _ => *problems.problem.lock().unwrap() = Some(error.to_string()),
        };
        let stream_config = config.into();
        let stream = match config.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                stream_config,
                feed::<f32>(shared, channels),
                on_error,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                stream_config,
                feed::<i16>(shared, channels),
                on_error,
                None,
            ),
            SampleFormat::I32 => device.build_input_stream(
                stream_config,
                feed::<i32>(shared, channels),
                on_error,
                None,
            ),
            other => return Err(format!("{name}: unsupported sample format {other}")),
        }
        .map_err(|error| format!("{name}: {error}"))?;
        stream.play().map_err(|error| format!("{name}: {error}"))?;
        Ok((stream, name))
    }

    /// A callback that mixes every frame down to mono and hands it to the analyzer.
    fn feed<T>(
        shared: &Arc<Shared>,
        channels: usize,
    ) -> impl FnMut(&[T], &cpal::InputCallbackInfo) + Send + use<T>
    where
        T: Sample + Send + 'static,
        f32: FromSample<T>,
    {
        let shared = Arc::clone(shared);
        let mut mono = Vec::new();
        move |data: &[T], _| {
            mono.clear();
            mono.extend(data.chunks(channels).map(|frame| {
                frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>() / frame.len() as f32
            }));
            if let Some(analyzer) = shared.analyzer.lock().unwrap().as_mut() {
                analyzer.push(&mono);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn tone(hz: f32, amplitude: f32, seconds: f32) -> Vec<f32> {
        (0..(seconds * RATE as f32) as usize)
            .map(|i| amplitude * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin())
            .collect()
    }

    /// Feed in 33 ms slices like a 30 fps host, collecting each frame's reading.
    fn frames(analyzer: &mut Analyzer, samples: &[f32]) -> Vec<Excitement> {
        samples
            .chunks(RATE as usize / 30)
            .map(|chunk| {
                analyzer.push(chunk);
                analyzer.take()
            })
            .collect()
    }

    /// A kick drum every `period` seconds: a short, falling 60 Hz thump.
    fn kicks(seconds: f32, period: f32) -> Vec<f32> {
        let mut out = vec![0.0; (seconds * RATE as f32) as usize];
        let mut start = 0.2;
        while start < seconds {
            let first = (start * RATE as f32) as usize;
            for i in 0..(0.12 * RATE as f32) as usize {
                let t = i as f32 / RATE as f32;
                if let Some(slot) = out.get_mut(first + i) {
                    *slot = 0.8
                        * (-t / 0.04).exp()
                        * (std::f32::consts::TAU * (60.0 - 120.0 * t) * t).sin();
                }
            }
            start += period;
        }
        out
    }

    /// Hits are the climbs upward (a pulse fades too slowly to fall back between
    /// kicks); a climb over two frames counts once.
    fn strikes(readings: &[Excitement], least_rise: f32) -> usize {
        let mut count = 0;
        let mut climbing = false;
        for pair in readings.windows(2) {
            let rising = pair[1].pulse > pair[0].pulse + least_rise;
            if rising && !climbing {
                count += 1;
            }
            climbing = rising;
        }
        count
    }

    #[test]
    fn the_transform_finds_a_pure_tone_in_its_bin() {
        let mut analyzer = Analyzer::new(RATE);
        let bin = 40;
        for i in 0..FFT_SIZE {
            analyzer.re[i] =
                (std::f32::consts::TAU * bin as f32 * i as f32 / FFT_SIZE as f32).sin();
            analyzer.im[i] = 0.0;
        }
        analyzer.fft();
        let magnitude = |k: usize| analyzer.re[k].hypot(analyzer.im[k]);
        assert!((magnitude(bin) - FFT_SIZE as f32 / 2.0).abs() < 0.5);
        assert!(magnitude(bin + 3) < 0.01 && magnitude(bin - 5) < 0.01);
    }

    #[test]
    fn silence_is_calm_and_unheard() {
        let mut analyzer = Analyzer::new(RATE);
        let readings = frames(&mut analyzer, &vec![0.0; RATE as usize * 2]);
        assert!(
            readings
                .iter()
                .all(|reading| *reading == Excitement::default())
        );
        assert!(!analyzer.heard());
    }

    #[test]
    fn hiss_below_the_floor_is_silence() {
        let mut analyzer = Analyzer::new(RATE);
        let hiss = tone(1000.0, 0.001, 2.0);
        let readings = frames(&mut analyzer, &hiss);
        assert!(
            readings
                .iter()
                .all(|reading| reading.level == 0.0 && reading.pulse == 0.0)
        );
        assert!(analyzer.heard());
    }

    #[test]
    fn a_steady_tone_is_loud_but_has_no_beats() {
        let mut analyzer = Analyzer::new(RATE);
        let readings = frames(&mut analyzer, &tone(220.0, 0.3, 4.0));
        let settled = &readings[60..];
        assert!(
            settled.iter().all(|reading| reading.level > 0.8),
            "{settled:?}"
        );
        assert!(settled.iter().all(|reading| reading.pulse < 0.1));
    }

    #[test]
    fn loudness_follows_the_music_up_and_down() {
        let mut analyzer = Analyzer::new(RATE);
        let mut music = tone(220.0, 0.4, 2.0);
        music.extend(tone(220.0, 0.04, 2.0));
        let readings = frames(&mut analyzer, &music);
        let loud = readings[50].level;
        let quiet = readings[readings.len() - 1].level;
        assert!(loud > 0.8 && quiet < 0.2, "loud {loud}, quiet {quiet}");
    }

    #[test]
    fn a_kick_drum_strikes_once_per_beat() {
        let mut analyzer = Analyzer::new(RATE);
        // 120 BPM for 6 seconds: kicks at 0.2, 0.7, ... 5.7.
        let readings = frames(&mut analyzer, &kicks(6.0, 0.5));
        assert_eq!(
            strikes(&readings, 0.1),
            12,
            "{:?}",
            readings.iter().map(|r| r.pulse).collect::<Vec<_>>()
        );
        // A kick is short, so loudness rises only part way: the pulse carries it.
        assert!(readings.iter().map(|r| r.level).fold(0.0, f32::max) > 0.2);
    }

    #[test]
    fn every_kick_counts_as_one_hit() {
        let mut analyzer = Analyzer::new(RATE);
        frames(&mut analyzer, &kicks(6.0, 0.5));
        assert_eq!(analyzer.hits(), 12);
    }

    #[test]
    fn a_pulse_between_two_frames_is_still_reported() {
        let mut analyzer = Analyzer::new(RATE);
        let samples = kicks(3.0, 0.5);
        // Look only once a second: every kick has decayed by then, but the
        // strongest pulse since the last look is kept.
        let looks: Vec<f32> = samples
            .chunks(RATE as usize)
            .map(|chunk| {
                analyzer.push(chunk);
                analyzer.take().pulse
            })
            .collect();
        assert!(looks[1..].iter().all(|pulse| *pulse > 0.45), "{looks:?}");
    }

    #[test]
    fn works_at_other_sample_rates_and_ignores_garbage() {
        let mut analyzer = Analyzer::new(44_100).with_sensitivity(f32::NAN);
        analyzer.push(&[f32::NAN, f32::INFINITY, -f32::INFINITY]);
        let reading = analyzer.take();
        assert_eq!(reading, Excitement::default());
        let hz = 44_100.0;
        let samples: Vec<f32> = (0..44_100)
            .map(|i| 0.3 * (std::f32::consts::TAU * 200.0 * i as f32 / hz).sin())
            .collect();
        analyzer.push(&samples);
        assert!(analyzer.take().level > 0.5);
    }

    /// Plucked sine notes, one after another, with no drums and no bass.
    fn melody(notes: &[f32], seconds_each: f32) -> Vec<f32> {
        notes
            .iter()
            .flat_map(|&hz| {
                tone(hz, 0.3, seconds_each)
                    .into_iter()
                    .enumerate()
                    .map(|(i, sample)| {
                        let t = i as f32 / RATE as f32;
                        sample * (t / 0.01).min(1.0) * (-t / 0.4).exp()
                    })
            })
            .collect()
    }

    fn mean(readings: &[Excitement], value: impl Fn(&Excitement) -> f32) -> f32 {
        readings.iter().map(value).sum::<f32>() / readings.len() as f32
    }

    #[test]
    fn notes_without_any_drums_still_strike_and_keep_it_busy() {
        let scale = [262.0, 330.0, 392.0, 523.0, 392.0, 330.0, 262.0, 196.0];
        let notes: Vec<f32> = scale.iter().cycle().take(16).copied().collect();
        let mut analyzer = Analyzer::new(RATE);
        let readings = frames(&mut analyzer, &melody(&notes, 0.25));
        // Notes come faster than a pulse fades, so each lifts it only a little.
        let hits = strikes(&readings, 0.03);
        assert!((12..=20).contains(&hits), "{hits} strikes for 16 notes");
        let busy = mean(&readings[30..], |reading| reading.activity);
        assert!(busy > 0.2, "activity {busy}");

        let mut drone = Analyzer::new(RATE);
        let held = frames(&mut drone, &tone(330.0, 0.3, 4.0));
        assert!(mean(&held[30..], |reading| reading.activity) < 0.05);
    }

    #[test]
    fn the_tone_rises_and_falls_with_the_melody() {
        let up = [220.0, 262.0, 330.0, 440.0, 523.0, 660.0, 880.0, 1100.0];
        let mut analyzer = Analyzer::new(RATE);
        let readings = frames(&mut analyzer, &melody(&up, 0.3));
        let at = |note: usize| {
            readings[(note as f32 * 0.3 * 30.0) as usize + 6]
                .tone
                .unwrap()
        };
        assert!(at(7) > at(0) + 0.2, "{} then {}", at(0), at(7));
        assert!(at(4) > at(1));

        let down: Vec<f32> = up.iter().rev().copied().collect();
        let mut analyzer = Analyzer::new(RATE);
        let readings = frames(&mut analyzer, &melody(&down, 0.3));
        let at = |note: usize| {
            readings[(note as f32 * 0.3 * 30.0) as usize + 6]
                .tone
                .unwrap()
        };
        assert!(at(7) < at(0) - 0.2);
    }

    #[test]
    fn silence_has_no_tone_and_a_held_note_keeps_its_own() {
        let mut analyzer = Analyzer::new(RATE);
        assert!(
            frames(&mut analyzer, &vec![0.0; 4800])
                .iter()
                .all(|r| r.tone.is_none())
        );
        let held = frames(&mut analyzer, &tone(440.0, 0.3, 2.0));
        let settled: Vec<f32> = held[30..].iter().map(|r| r.tone.unwrap()).collect();
        let spread = settled.iter().cloned().fold(0.0, f32::max)
            - settled.iter().cloned().fold(1.0, f32::min);
        assert!(spread < 0.02, "a held note drifted by {spread}");
    }

    /// A quiet pad: a slowly swelling chord with a little vibrato and hiss, a new
    /// chord every three seconds. Nothing in it is a hit.
    fn calm(seconds: f32) -> Vec<f32> {
        let chords = [
            [220.0, 277.2, 329.6],
            [196.0, 246.9, 293.7],
            [174.6, 220.0, 261.6],
        ];
        let mut noise = 12345_u32;
        (0..(seconds * RATE as f32) as usize)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let swell = 1.0 + 0.3 * (std::f32::consts::TAU * t / 6.0).sin();
                let (index, into) = ((t / 3.0) as usize, (t % 3.0) / 0.5);
                let blend = into.min(1.0);
                let voice = |chord: usize| -> f32 {
                    chords[chord % 3]
                        .iter()
                        .map(|hz| {
                            let vibrato = 1.0 + 0.003 * (std::f32::consts::TAU * 5.0 * t).sin();
                            (std::f32::consts::TAU * hz * vibrato * t).sin()
                        })
                        .sum::<f32>()
                        / 3.0
                };
                let pad = if index == 0 {
                    voice(0)
                } else {
                    voice(index - 1) * (1.0 - blend) + voice(index) * blend
                };
                noise = noise.wrapping_mul(1664525).wrapping_add(1013904223);
                let hiss = (noise >> 8) as f32 / (1 << 24) as f32 - 0.5;
                0.08 * swell * pad + 0.01 * hiss
            })
            .collect()
    }

    /// How restless the readings of a calm passage are, once it has settled.
    fn restlessness(readings: &[Excitement]) -> (usize, f32, f32, f32, f32) {
        let settled = &readings[90..];
        let jumps = strikes(settled, 0.03);
        let variation = |value: fn(&Excitement) -> f32| {
            settled
                .windows(2)
                .map(|p| (value(&p[1]) - value(&p[0])).abs())
                .sum::<f32>()
        };
        let steering: f32 = settled
            .windows(2)
            .filter_map(|p| Some((p[1].tone? - p[0].tone?).abs() * 20.0))
            .sum();
        (
            jumps,
            mean(settled, |r| r.activity),
            variation(|r| r.activity),
            variation(|r| r.level),
            steering,
        )
    }

    #[test]
    fn calm_music_is_not_restless() {
        let mut analyzer = Analyzer::new(RATE);
        let readings = frames(&mut analyzer, &calm(15.0));
        let seconds = (readings.len() - 90) as f32 / 30.0;
        let (jumps, activity, _, _, steering) = restlessness(&readings);
        // A swelling pad with vibrato and hiss has no hits and no melody to follow.
        assert!(jumps as f32 / seconds < 0.5, "{jumps} jumps in {seconds} s");
        assert!(activity < 0.15, "activity {activity}");
        assert!(
            steering / seconds < 3.0,
            "steering {} rad/s",
            steering / seconds
        );
    }
}
