//! 10-band graphic equalizer with preamp (peaking biquads, RBJ cookbook).

use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

pub const BANDS: usize = 10;
/// Band centers of Winamp 5 (as labeled in its built-in equalizer bitmap).
pub const WINAMP_FREQS: [f32; BANDS] = [70.0, 180.0, 320.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0];
/// ISO octave band centers (Hz).
pub const ISO_FREQS: [f32; BANDS] = [31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
/// Slider range in dB.
pub const MAX_DB: f32 = 12.0;

const Q: f32 = 1.4;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EqSettings {
    pub enabled: bool,
    pub preamp_db: f32,
    pub bands_db: [f32; BANDS],
    /// Use ISO octave centers instead of the classic ones.
    pub iso: bool,
}

impl Default for EqSettings {
    fn default() -> Self {
        Self { enabled: false, preamp_db: 0.0, bands_db: [0.0; BANDS], iso: false }
    }
}

/// Lock-free settings shared with the audio callback.
#[derive(Default)]
pub struct SharedEq {
    enabled: AtomicBool,
    iso: AtomicBool,
    preamp: AtomicU32,
    bands: [AtomicU32; BANDS],
    version: AtomicU64,
}

impl SharedEq {
    pub fn set(&self, s: &EqSettings) {
        self.enabled.store(s.enabled, Ordering::Relaxed);
        self.iso.store(s.iso, Ordering::Relaxed);
        self.preamp.store(s.preamp_db.clamp(-MAX_DB, MAX_DB).to_bits(), Ordering::Relaxed);
        for (slot, db) in self.bands.iter().zip(s.bands_db) {
            slot.store(db.clamp(-MAX_DB, MAX_DB).to_bits(), Ordering::Relaxed);
        }
        self.version.fetch_add(1, Ordering::Release);
    }

    pub fn get(&self) -> EqSettings {
        let f = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        EqSettings {
            enabled: self.enabled.load(Ordering::Relaxed),
            iso: self.iso.load(Ordering::Relaxed),
            preamp_db: f(&self.preamp),
            bands_db: std::array::from_fn(|i| f(&self.bands[i])),
        }
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Coeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coeffs {
    fn peaking(freq: f32, rate: f32, db: f32) -> Option<Self> {
        // Bands at or above Nyquist are skipped (e.g. 16 kHz on 22.05 kHz audio).
        if db.abs() < 0.01 || freq >= rate * 0.45 {
            return None;
        }
        let a = 10f32.powf(db / 40.0);
        let w0 = 2.0 * PI * freq / rate;
        let alpha = w0.sin() / (2.0 * Q);
        let cos = w0.cos();
        let a0 = 1.0 + alpha / a;
        Some(Self {
            b0: (1.0 + alpha * a) / a0,
            b1: -2.0 * cos / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha / a) / a0,
        })
    }
}

/// Per-channel filter state (transposed direct form II).
#[derive(Debug, Clone, Copy, Default)]
struct State {
    z1: f32,
    z2: f32,
}

const MAX_CHANNELS: usize = 8;

/// Runs in the audio callback; owns the filter state.
pub struct Equalizer {
    rate: f32,
    channels: usize,
    version: u64,
    enabled: bool,
    preamp: f32,
    /// Per band; `None` when the band is flat or above Nyquist.
    filters: [Option<Coeffs>; BANDS],
    state: [[State; MAX_CHANNELS]; BANDS],
}

impl Equalizer {
    pub fn new(rate: u32, channels: usize) -> Self {
        Self {
            rate: rate as f32,
            channels: channels.min(MAX_CHANNELS),
            version: u64::MAX,
            enabled: false,
            preamp: 1.0,
            filters: [None; BANDS],
            state: [[State::default(); MAX_CHANNELS]; BANDS],
        }
    }

    /// Pick up new settings if they changed. Cheap enough to call every callback.
    pub fn sync(&mut self, shared: &SharedEq) {
        let v = shared.version();
        if v == self.version {
            return;
        }
        self.version = v;
        self.configure(&shared.get());
    }

    pub fn configure(&mut self, s: &EqSettings) {
        let freqs = if s.iso { ISO_FREQS } else { WINAMP_FREQS };
        self.enabled = s.enabled;
        self.preamp = 10f32.powf(s.preamp_db / 20.0);
        for (slot, (&f, &db)) in self.filters.iter_mut().zip(freqs.iter().zip(&s.bands_db)) {
            *slot = Coeffs::peaking(f, self.rate, db);
        }
    }

    /// Filter interleaved samples in place.
    pub fn process(&mut self, samples: &mut [f32]) {
        if !self.enabled {
            return;
        }
        let ch = self.channels.max(1);
        for frame in samples.chunks_mut(ch) {
            for (c, s) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                let mut x = *s * self.preamp;
                for (f, st) in self.filters.iter().zip(self.state.iter_mut()) {
                    let Some(f) = f else { continue };
                    let st = &mut st[c];
                    let y = f.b0 * x + st.z1;
                    st.z1 = f.b1 * x - f.a1 * y + st.z2;
                    st.z2 = f.b2 * x - f.a2 * y;
                    x = y;
                }
                // Hard limit: boosted bands must not wrap integer output formats.
                *s = x.clamp(-1.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n).flat_map(|i| {
            let v = amp * (2.0 * PI * freq * i as f32 / rate).sin();
            [v, v]
        })
        .collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        // Skip the filter's settling time.
        samples[samples.len() / 2..].iter().fold(0f32, |m, s| m.max(s.abs()))
    }

    fn eq_with(bands_db: [f32; BANDS], preamp_db: f32) -> Equalizer {
        let mut eq = Equalizer::new(44_100, 2);
        eq.configure(&EqSettings { enabled: true, preamp_db, bands_db, iso: false });
        eq
    }

    #[test]
    fn flat_is_transparent() {
        let mut eq = eq_with([0.0; BANDS], 0.0);
        let input = sine(1000.0, 44_100.0, 4410, 0.5);
        let mut out = input.clone();
        eq.process(&mut out);
        assert!(input.iter().zip(&out).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn boost_raises_band_center() {
        let mut bands = [0.0; BANDS];
        bands[4] = 12.0; // 1 kHz
        let mut eq = eq_with(bands, 0.0);
        let mut out = sine(1000.0, 44_100.0, 8820, 0.1);
        eq.process(&mut out);
        let gain_db = 20.0 * (peak(&out) / 0.1).log10();
        assert!((gain_db - 12.0).abs() < 1.0, "gain {gain_db} dB");
    }

    #[test]
    fn cut_leaves_distant_frequencies_mostly_alone() {
        let mut bands = [0.0; BANDS];
        bands[0] = -12.0; // 60 Hz
        let mut eq = eq_with(bands, 0.0);
        let mut out = sine(6000.0, 44_100.0, 8820, 0.1);
        eq.process(&mut out);
        let gain_db = 20.0 * (peak(&out) / 0.1).log10();
        assert!(gain_db.abs() < 0.5, "gain {gain_db} dB");
    }

    #[test]
    fn preamp_scales_and_limits() {
        let mut eq = eq_with([0.0; BANDS], -6.0);
        let mut out = sine(440.0, 44_100.0, 4410, 0.5);
        eq.process(&mut out);
        assert!((peak(&out) - 0.25).abs() < 0.01);

        let mut eq = eq_with([0.0; BANDS], 12.0);
        let mut loud = sine(440.0, 44_100.0, 4410, 0.9);
        eq.process(&mut loud);
        assert!(peak(&loud) <= 1.0);
    }

    #[test]
    fn disabled_does_nothing_and_shared_round_trips() {
        let shared = SharedEq::default();
        let mut s = EqSettings { enabled: false, preamp_db: 20.0, ..Default::default() };
        s.bands_db[3] = 5.0;
        shared.set(&s);
        let got = shared.get();
        assert_eq!(got.preamp_db, MAX_DB); // clamped
        assert_eq!(got.bands_db[3], 5.0);

        let mut eq = Equalizer::new(44_100, 2);
        eq.sync(&shared);
        let input = sine(440.0, 44_100.0, 100, 0.5);
        let mut out = input.clone();
        eq.process(&mut out);
        assert_eq!(input, out);
    }
}
