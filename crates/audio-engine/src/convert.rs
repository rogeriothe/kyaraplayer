//! Channel remapping and sample-rate conversion between decoder and device.

/// Remap interleaved `input` with `src` channels to `dst` channels, appending to `out`.
pub fn remap_channels(input: &[f32], src: usize, dst: usize, out: &mut Vec<f32>) {
    if src == dst {
        out.extend_from_slice(input);
        return;
    }
    for frame in input.chunks_exact(src) {
        match (src, dst) {
            (1, _) => out.extend(std::iter::repeat_n(frame[0], dst)),
            (_, 1) => out.push(frame.iter().sum::<f32>() / src as f32),
            _ => {
                // Keep the front channels in order, pad with silence if the device has more.
                for c in 0..dst {
                    out.push(frame.get(c).copied().unwrap_or(0.0));
                }
            }
        }
    }
}

/// Streaming sample-rate converter between decoder and device.
///
/// Band-limited FFT resampling (rubato); falls back to linear interpolation if the
/// rate pair cannot be set up.
pub enum Resampler {
    Fft(Box<FftResampler>),
    Linear(LinearResampler),
}

impl Resampler {
    pub fn new(from_rate: u32, to_rate: u32, channels: usize) -> Self {
        match FftResampler::new(from_rate, to_rate, channels) {
            Ok(r) => Self::Fft(Box::new(r)),
            Err(e) => {
                log::warn!("band-limited resampler unavailable ({from_rate} -> {to_rate} Hz): {e}");
                Self::Linear(LinearResampler::new(from_rate, to_rate, channels))
            }
        }
    }

    pub fn matches(&self, from_rate: u32, to_rate: u32, channels: usize) -> bool {
        match self {
            Self::Fft(r) => (r.from, r.to, r.channels) == (from_rate, to_rate, channels),
            Self::Linear(r) => r.matches(from_rate, to_rate, channels),
        }
    }

    /// Drop buffered audio (seek, flush).
    pub fn reset(&mut self) {
        match self {
            Self::Fft(r) => r.reset(),
            Self::Linear(r) => r.reset(),
        }
    }

    /// Resample interleaved `input`, appending to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        match self {
            Self::Fft(r) => r.process(input, out),
            Self::Linear(r) => r.process(input, out),
        }
    }

    /// End of input: emit whatever is still buffered.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        if let Self::Fft(r) = self {
            r.finish(out);
        }
    }
}

/// Fixed-input-chunk FFT resampler with a FIFO in front, so callers can feed packets
/// of any size. The filter delay is trimmed from the output to keep positions exact.
pub struct FftResampler {
    inner: rubato::Fft<f32>,
    from: u32,
    to: u32,
    channels: usize,
    /// Interleaved input not yet consumed.
    fifo: Vec<f32>,
    /// Scratch output for one chunk.
    scratch: Vec<f32>,
    /// Output frames still to drop (filter delay).
    skip: usize,
    /// Output frames owed for the input fed so far (exact ratio), used when flushing.
    fed_frames: u64,
    emitted_frames: u64,
}

const FFT_CHUNK: usize = 1024;

impl FftResampler {
    pub fn new(from: u32, to: u32, channels: usize) -> Result<Self, rubato::ResamplerConstructionError> {
        use rubato::Resampler as _;
        let inner = rubato::Fft::<f32>::new(
            from as usize,
            to as usize,
            FFT_CHUNK,
            channels.max(1),
            rubato::FixedSync::Input,
        )?;
        let skip = inner.output_delay();
        let scratch = vec![0.0; inner.output_frames_max() * channels.max(1)];
        Ok(Self { inner, from, to, channels: channels.max(1), fifo: Vec::new(), scratch, skip, fed_frames: 0, emitted_frames: 0 })
    }

    fn reset(&mut self) {
        use rubato::Resampler as _;
        self.inner.reset();
        self.fifo.clear();
        self.skip = self.inner.output_delay();
        self.fed_frames = 0;
        self.emitted_frames = 0;
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.fifo.extend_from_slice(input);
        self.fed_frames += (input.len() / self.channels) as u64;
        self.run(false, out);
    }

    fn finish(&mut self, out: &mut Vec<f32>) {
        self.run(true, out);
    }

    /// Process full chunks; with `flush`, pad the tail with silence until every output
    /// frame owed for the real input has been produced.
    fn run(&mut self, flush: bool, out: &mut Vec<f32>) {
        use rubato::audioadapter_buffers::direct::InterleavedSlice;
        use rubato::{Indexing, Resampler as _};
        let ch = self.channels;
        let owed = |s: &Self| s.fed_frames * u64::from(s.to) / u64::from(s.from);
        loop {
            let need = self.inner.input_frames_next();
            let have = self.fifo.len() / ch;
            if have < need && !(flush && self.emitted_frames < owed(self)) {
                return;
            }
            let partial = (have < need).then_some(have);
            let frames_in = partial.unwrap_or(need);
            let frames_out = self.inner.output_frames_next();
            let indexing = Indexing { input_offset: 0, output_offset: 0, partial_len: partial, active_channels_mask: None };
            // Partial input still needs a buffer of the full chunk length.
            if partial.is_some() {
                self.fifo.resize(need * ch, 0.0);
            }
            let result = {
                let Ok(input) = InterleavedSlice::new(&self.fifo[..], ch, need) else { return };
                let Ok(mut output) = InterleavedSlice::new_mut(&mut self.scratch[..], ch, frames_out) else { return };
                self.inner.process_into_buffer(&input, &mut output, Some(&indexing))
            };
            let (_, written) = match result {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("resampler: {e}");
                    self.fifo.clear();
                    return;
                }
            };
            self.fifo.drain(..frames_in * ch);
            if partial.is_some() {
                self.fifo.clear();
            }
            let mut produced = &self.scratch[..written * ch];
            let skip = self.skip.min(written);
            self.skip -= skip;
            produced = &produced[skip * ch..];
            let mut frames = produced.len() / ch;
            if flush {
                // Do not emit the silent padding past the real end.
                frames = frames.min(owed(self).saturating_sub(self.emitted_frames) as usize);
            }
            out.extend_from_slice(&produced[..frames * ch]);
            self.emitted_frames += frames as u64;
            if flush && self.emitted_frames >= owed(self) {
                return;
            }
        }
    }
}

/// Streaming linear-interpolation resampler (fallback).
pub struct LinearResampler {
    channels: usize,
    step: f64,
    pos: f64,
    prev: Vec<f32>,
}

impl LinearResampler {
    pub fn new(from_rate: u32, to_rate: u32, channels: usize) -> Self {
        Self {
            channels,
            step: from_rate as f64 / to_rate as f64,
            pos: 0.0,
            prev: vec![0.0; channels],
        }
    }

    pub fn matches(&self, from_rate: u32, to_rate: u32, channels: usize) -> bool {
        self.channels == channels && self.step == from_rate as f64 / to_rate as f64
    }

    pub fn reset(&mut self) {
        self.pos = 0.0;
        self.prev.fill(0.0);
    }

    /// Resample interleaved `input`, appending to `out`.
    ///
    /// `pos` is a fractional frame index into `input`; `-1` refers to the last frame
    /// of the previous call, so interpolation is continuous across chunks.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let ch = self.channels;
        let n = input.len() / ch;
        if n == 0 {
            return;
        }
        let frame = |i: isize| -> &[f32] {
            if i < 0 {
                &self.prev
            } else {
                let i = i as usize;
                &input[i * ch..(i + 1) * ch]
            }
        };
        while self.pos < (n - 1) as f64 {
            let i = self.pos.floor();
            let t = (self.pos - i) as f32;
            let (a, b) = (frame(i as isize), frame(i as isize + 1));
            for c in 0..ch {
                out.push(a[c] + (b[c] - a[c]) * t);
            }
            self.pos += self.step;
        }
        self.pos -= n as f64;
        self.prev.copy_from_slice(&input[(n - 1) * ch..n * ch]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_to_stereo_duplicates() {
        let mut out = Vec::new();
        remap_channels(&[0.5, -0.5], 1, 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn stereo_to_mono_averages() {
        let mut out = Vec::new();
        remap_channels(&[1.0, 0.0, 0.5, 0.5], 2, 1, &mut out);
        assert_eq!(out, vec![0.5, 0.5]);
    }

    #[test]
    fn fft_resampler_keeps_length_and_frequency() {
        // 1 s of a 1 kHz sine at 44.1 kHz, fed in odd-sized packets, to 48 kHz.
        let input: Vec<f32> =
            (0..44_100).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44_100.0).sin() * 0.5).collect();
        let mut rs = Resampler::new(44_100, 48_000, 1);
        assert!(matches!(rs, Resampler::Fft(_)));
        let mut out = Vec::new();
        for chunk in input.chunks(1152) {
            rs.process(chunk, &mut out);
        }
        rs.finish(&mut out);
        assert!((out.len() as i64 - 48_000).abs() <= 2, "got {} frames", out.len());
        // Count zero crossings in the steady middle: ~2 per cycle, 1 kHz.
        let mid = &out[4_800..43_200]; // 0.8 s
        let crossings = mid.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((crossings as i64 - 1600).abs() <= 4, "crossings {crossings}");
        // No leading silence: the delay is trimmed.
        assert!(out[..40].iter().any(|s| s.abs() > 0.05));
    }

    #[test]
    fn resample_ratio_is_kept_across_chunks() {
        let mut rs = LinearResampler::new(44_100, 48_000, 1);
        let input: Vec<f32> = (0..441).map(|i| i as f32).collect();
        let mut out = Vec::new();
        for chunk in input.chunks(100) {
            rs.process(chunk, &mut out);
        }
        // 441 frames at 44.1k = 10 ms = 480 frames at 48k (±1 for the held-back tail).
        assert!((out.len() as i64 - 480).abs() <= 1, "got {}", out.len());
        // Ramp input stays a monotonic ramp after interpolation.
        assert!(out.windows(2).all(|w| w[1] >= w[0]));
    }

    #[test]
    fn same_rate_is_identity() {
        let mut rs = LinearResampler::new(48_000, 48_000, 2);
        let input = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let mut out = Vec::new();
        rs.process(&input, &mut out);
        rs.process(&[6.0, 7.0], &mut out);
        assert_eq!(out, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
    }
}
