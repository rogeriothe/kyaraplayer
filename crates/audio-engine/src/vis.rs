//! Visualization: a lock-free tap on the device output and the analyzer that turns
//! it into the classic 75-column spectrum and oscilloscope.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use serde::Serialize;

/// Columns of the classic visualizer (76 px wide, last column unused).
pub const COLUMNS: usize = 75;
const TAP_LEN: usize = 4096;
const FFT_LEN: usize = 1024;
/// Samples shown by the oscilloscope (about 13 ms at 44.1 kHz).
const SCOPE_LEN: usize = 576;

/// Mono ring of the most recent output, written by the audio callback.
pub struct VisTap {
    buf: Box<[AtomicU32]>,
    write: AtomicUsize,
}

impl Default for VisTap {
    fn default() -> Self {
        Self { buf: (0..TAP_LEN).map(|_| AtomicU32::new(0)).collect(), write: AtomicUsize::new(0) }
    }
}

impl VisTap {
    /// Mix interleaved frames to mono and append. Allocation- and lock-free.
    pub fn push(&self, samples: &[f32], channels: usize) {
        let ch = channels.max(1);
        let mut w = self.write.load(Ordering::Relaxed);
        for frame in samples.chunks_exact(ch) {
            let mono = frame.iter().sum::<f32>() / ch as f32;
            self.buf[w % TAP_LEN].store(mono.to_bits(), Ordering::Relaxed);
            w = w.wrapping_add(1);
        }
        self.write.store(w, Ordering::Release);
    }

    /// Copy the newest `out.len()` samples, oldest first.
    pub fn snapshot(&self, out: &mut [f32]) {
        let w = self.write.load(Ordering::Acquire);
        let n = out.len().min(TAP_LEN);
        for (i, o) in out.iter_mut().take(n).enumerate() {
            let idx = w.wrapping_sub(n).wrapping_add(i) % TAP_LEN;
            *o = f32::from_bits(self.buf[idx].load(Ordering::Relaxed));
        }
    }
}

/// One visualizer frame. Spectrum values are 0..=255 (bar height); wave values are
/// 0..=255 with 128 as the center line.
#[derive(Debug, Clone, Serialize)]
pub struct VisFrame {
    pub spectrum: Vec<u8>,
    pub wave: Vec<u8>,
}

pub struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    samples: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
}

impl Default for Analyzer {
    fn default() -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FFT_LEN);
        // Hann window.
        let window = (0..FFT_LEN)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_LEN - 1) as f32).cos())
            .collect();
        Self { fft, window, samples: vec![0.0; FFT_LEN], spectrum: vec![Complex::default(); FFT_LEN] }
    }
}

impl Analyzer {
    pub fn frame(&mut self, tap: &VisTap, rate: u32) -> VisFrame {
        tap.snapshot(&mut self.samples);
        VisFrame { spectrum: self.spectrum(rate), wave: self.wave() }
    }

    fn wave(&self) -> Vec<u8> {
        let scope = &self.samples[FFT_LEN - SCOPE_LEN..];
        (0..COLUMNS)
            .map(|c| {
                let s = scope[c * SCOPE_LEN / COLUMNS];
                (128.0 + s.clamp(-1.0, 1.0) * 127.0).round() as u8
            })
            .collect()
    }

    fn spectrum(&mut self, rate: u32) -> Vec<u8> {
        for ((out, s), w) in self.spectrum.iter_mut().zip(&self.samples).zip(&self.window) {
            *out = Complex::new(s * w, 0.0);
        }
        self.fft.process(&mut self.spectrum);

        let rate = rate.max(8000) as f32;
        let bin_hz = rate / FFT_LEN as f32;
        let lo = 40.0f32;
        let hi = (rate * 0.45).min(16_000.0);
        // Amplitude of a full-scale sine after the Hann window (coherent gain 0.5).
        let norm = FFT_LEN as f32 / 4.0;
        (0..COLUMNS)
            .map(|c| {
                // Log-spaced columns; each takes the strongest bin in its range.
                let f0 = lo * (hi / lo).powf(c as f32 / COLUMNS as f32);
                let f1 = lo * (hi / lo).powf((c + 1) as f32 / COLUMNS as f32);
                let b0 = ((f0 / bin_hz) as usize).max(1);
                let b1 = ((f1 / bin_hz).ceil() as usize).clamp(b0 + 1, FFT_LEN / 2);
                let mag = self.spectrum[b0..b1].iter().map(|z| z.norm()).fold(0.0f32, f32::max) / norm;
                // dB with a +3 dB/octave tilt so treble is visible, mapped from -66..0 dB.
                let center = (f0 * f1).sqrt();
                let db = 20.0 * mag.max(1e-9).log10() + 3.0 * (center / 1000.0).log2();
                (((db + 66.0) / 66.0).clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap_with_sine(freq: f32, rate: f32, amp: f32) -> VisTap {
        let tap = VisTap::default();
        let stereo: Vec<f32> = (0..TAP_LEN)
            .flat_map(|i| {
                let v = amp * (2.0 * std::f32::consts::PI * freq * i as f32 / rate).sin();
                [v, v]
            })
            .collect();
        tap.push(&stereo, 2);
        tap
    }

    #[test]
    fn snapshot_returns_newest_samples_in_order() {
        let tap = VisTap::default();
        let data: Vec<f32> = (0..TAP_LEN + 10).map(|i| i as f32).collect();
        tap.push(&data, 1);
        let mut out = [0.0; 3];
        tap.snapshot(&mut out);
        assert_eq!(out, [(TAP_LEN + 7) as f32, (TAP_LEN + 8) as f32, (TAP_LEN + 9) as f32]);
    }

    #[test]
    fn sine_peaks_in_the_matching_column() {
        let mut a = Analyzer::default();
        let low = a.frame(&tap_with_sine(100.0, 44_100.0, 0.8), 44_100).spectrum;
        let high = a.frame(&tap_with_sine(5000.0, 44_100.0, 0.8), 44_100).spectrum;
        let argmax = |v: &[u8]| v.iter().enumerate().max_by_key(|(_, x)| **x).unwrap().0;
        assert!(argmax(&low) < 20, "100 Hz peak at column {}", argmax(&low));
        assert!(argmax(&high) > 45, "5 kHz peak at column {}", argmax(&high));
        assert!(low[argmax(&low)] > 200);
    }

    #[test]
    fn silence_is_flat_and_centered() {
        let mut a = Analyzer::default();
        let f = a.frame(&VisTap::default(), 44_100);
        assert!(f.spectrum.iter().all(|&v| v == 0));
        assert!(f.wave.iter().all(|&v| v == 128));
        assert_eq!((f.spectrum.len(), f.wave.len()), (COLUMNS, COLUMNS));
    }
}
