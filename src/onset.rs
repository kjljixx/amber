//! Finds hits the way librosa does (`onset_strength` and `peak_pick`), on a stream.
//!
//! [`MelFlux`] measures how much louder each mel band got since the last look, in
//! decibels. [`PeakPicker`] says whether the latest value stands out from the ones
//! before it. Nothing here looks ahead, and decibels are not clipped to the loudest
//! value of the whole clip, since a stream has no whole clip.

use std::collections::VecDeque;

/// Samples in one look at the spectrum.
pub(crate) const MEL_FFT_SIZE: usize = 2048;
const MEL_BANDS: usize = 128;
/// Quietest power that still counts (-60 dB). Below it, rounding noise and the faint
/// leakage of a pure tone would swing by tens of dB and look like new sound.
const MIN_POWER: f32 = 1e-6;
/// A hit is the largest of the looks in this long, ...
const PEAK_SECONDS: f32 = 0.03;
/// ... and stands out from the average of the looks in this long.
const AVERAGE_SECONDS: f32 = 0.10;
/// How far above that average, in dB, a hit has to stand.
const HIT_DELTA_DB: f32 = 1.1;

/// `twiddles(n)` gives the cosines and sines that [`fft`] needs for size `n`.
pub(crate) fn twiddles(n: usize) -> (Vec<f32>, Vec<f32>) {
    let tau = std::f32::consts::TAU;
    let angle = |i: usize| tau * i as f32 / n as f32;
    (
        (0..n / 2).map(|i| angle(i).cos()).collect(),
        (0..n / 2).map(|i| -angle(i).sin()).collect(),
    )
}

/// In-place radix-2 transform of `re`/`im`, whose length is a power of two.
pub(crate) fn fft(re: &mut [f32], im: &mut [f32], cos: &[f32], sin: &[f32]) {
    let n = re.len();
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut size = 2;
    while size <= n {
        let (half, step) = (size / 2, n / size);
        for start in (0..n).step_by(size) {
            for k in 0..half {
                let (c, s) = (cos[k * step], sin[k * step]);
                let (a, b) = (start + k, start + k + half);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        size *= 2;
    }
}

const SLANEY_LINEAR_HZ: f32 = 200.0 / 3.0;
const SLANEY_LOG_FROM_HZ: f32 = 1000.0;

fn slaney_mel(hz: f32) -> f32 {
    let log_from_mel = SLANEY_LOG_FROM_HZ / SLANEY_LINEAR_HZ;
    if hz < SLANEY_LOG_FROM_HZ {
        hz / SLANEY_LINEAR_HZ
    } else {
        log_from_mel + (hz / SLANEY_LOG_FROM_HZ).ln() / (6.4_f32.ln() / 27.0)
    }
}

fn slaney_hz(mel: f32) -> f32 {
    let log_from_mel = SLANEY_LOG_FROM_HZ / SLANEY_LINEAR_HZ;
    if mel < log_from_mel {
        mel * SLANEY_LINEAR_HZ
    } else {
        SLANEY_LOG_FROM_HZ * ((mel - log_from_mel) * (6.4_f32.ln() / 27.0)).exp()
    }
}

/// One triangular mel band over the bins from `start`, scaled to equal area.
struct Band {
    start: usize,
    weights: Vec<f32>,
}

/// Mean over the mel bands of the rise in decibels since the previous look.
pub(crate) struct MelFlux {
    window: Vec<f32>,
    cos: Vec<f32>,
    sin: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    bands: Vec<Band>,
    previous: Option<Vec<f32>>,
}

impl MelFlux {
    pub(crate) fn new(sample_rate: f32) -> Self {
        let tau = std::f32::consts::TAU;
        let (cos, sin) = twiddles(MEL_FFT_SIZE);
        let bin_hz = sample_rate / MEL_FFT_SIZE as f32;
        let top = slaney_mel(sample_rate / 2.0);
        let edges: Vec<f32> = (0..MEL_BANDS + 2)
            .map(|i| slaney_hz(top * i as f32 / (MEL_BANDS + 1) as f32))
            .collect();
        let bands = (0..MEL_BANDS)
            .map(|i| {
                let (low, mid, high) = (edges[i], edges[i + 1], edges[i + 2]);
                let area = 2.0 / (high - low);
                let weight = |bin: usize| {
                    let hz = bin as f32 * bin_hz;
                    ((hz - low) / (mid - low)).min((high - hz) / (high - mid)).max(0.0) * area
                };
                let first = (low / bin_hz) as usize;
                let last = ((high / bin_hz) as usize + 1).min(MEL_FFT_SIZE / 2);
                Band {
                    start: first,
                    weights: (first..=last).map(weight).collect(),
                }
            })
            .collect();
        Self {
            window: (0..MEL_FFT_SIZE)
                .map(|i| 0.5 - 0.5 * (tau * i as f32 / MEL_FFT_SIZE as f32).cos())
                .collect(),
            cos,
            sin,
            re: vec![0.0; MEL_FFT_SIZE],
            im: vec![0.0; MEL_FFT_SIZE],
            bands,
            previous: None,
        }
    }

    /// Looks at the last [`MEL_FFT_SIZE`] samples of a ring buffer that `write` is about to
    /// overwrite next. The first look has nothing to compare with and gives 0.
    pub(crate) fn measure(&mut self, ring: &[f32], write: usize) -> f32 {
        for i in 0..MEL_FFT_SIZE {
            self.re[i] = ring[(write + i) % MEL_FFT_SIZE] * self.window[i];
            self.im[i] = 0.0;
        }
        fft(&mut self.re, &mut self.im, &self.cos, &self.sin);
        let decibels: Vec<f32> = self
            .bands
            .iter()
            .map(|band| {
                let power: f32 = band
                    .weights
                    .iter()
                    .enumerate()
                    .map(|(i, weight)| {
                        let bin = band.start + i;
                        weight * (self.re[bin] * self.re[bin] + self.im[bin] * self.im[bin])
                    })
                    .sum();
                10.0 * power.max(MIN_POWER).log10()
            })
            .collect();
        let rise = self.previous.as_ref().map_or(0.0, |previous| {
            let total: f32 = decibels
                .iter()
                .zip(previous)
                .map(|(now, before)| (now - before).max(0.0))
                .sum();
            total / MEL_BANDS as f32
        });
        self.previous = Some(decibels);
        rise
    }
}

/// What the picker made of the latest value.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pick {
    /// The recent average plus [`HIT_DELTA_DB`]; the value has to reach it.
    pub threshold: f32,
    /// The value is at least as large as every look in the last [`PEAK_SECONDS`].
    pub is_peak: bool,
}

/// Judges each new value against the looks just before it.
pub(crate) struct PeakPicker {
    peak_looks: usize,
    average_looks: usize,
    history: VecDeque<f32>,
}

impl PeakPicker {
    pub(crate) fn new(sample_rate: f32, hop: usize) -> Self {
        let looks = |seconds: f32| (seconds * sample_rate / hop as f32) as usize;
        Self {
            peak_looks: looks(PEAK_SECONDS),
            average_looks: looks(AVERAGE_SECONDS),
            history: VecDeque::new(),
        }
    }

    pub(crate) fn push(&mut self, value: f32) -> Pick {
        self.history.push_back(value);
        while self.history.len() > self.average_looks + 1 {
            self.history.pop_front();
        }
        let average = self.history.iter().sum::<f32>() / self.history.len() as f32;
        let recent_peak = self
            .history
            .iter()
            .rev()
            .take(self.peak_looks + 1)
            .fold(f32::MIN, |most, &v| most.max(v));
        Pick {
            threshold: average + HIT_DELTA_DB,
            is_peak: value >= recent_peak,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mel_scale_round_trips_and_is_linear_below_a_kilohertz() {
        assert!((slaney_mel(500.0) - 7.5).abs() < 1e-4);
        assert!((slaney_mel(1000.0) - 15.0).abs() < 1e-4);
        for hz in [50.0, 999.0, 1000.0, 4000.0, 20_000.0] {
            assert!((slaney_hz(slaney_mel(hz)) / hz - 1.0).abs() < 1e-4, "{hz}");
        }
    }

    #[test]
    fn a_new_sound_raises_the_flux_and_a_held_one_does_not() {
        let mut flux = MelFlux::new(44_100.0);
        let mut ring = vec![0.0; MEL_FFT_SIZE];
        let mut look = |ring: &[f32]| flux.measure(ring, 0);
        assert_eq!(look(&ring), 0.0);
        assert!(look(&ring) < 1e-3);
        for (i, sample) in ring.iter_mut().enumerate() {
            *sample = 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 44_100.0).sin();
        }
        assert!(look(&ring) > 1.0);
        assert!(look(&ring) < 1e-3);
    }

    #[test]
    fn a_value_has_to_be_the_largest_and_above_the_average_to_stand_out() {
        let mut picker = PeakPicker::new(44_100.0, 512);
        for _ in 0..20 {
            picker.push(1.0);
        }
        let spike = picker.push(6.0);
        assert!(spike.is_peak && 6.0 >= spike.threshold);
        let after = picker.push(5.0);
        assert!(!after.is_peak);
    }
}
