//! Audio device output via cpal. The device callback pulls from a lock-free ring buffer
//! and applies volume/balance, so those controls react without buffer latency.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig, SupportedStreamConfig};
use ringbuf::traits::{Consumer, Split};
use ringbuf::{HeapProd, HeapRb};

use crate::eq::{Equalizer, SharedEq};
use crate::vis::VisTap;
use crate::EngineError;

/// Seconds of audio kept between decoder and device.
const BUFFER_SECS: f32 = 0.5;

/// State shared between the engine thread, the device callback and API callers.
#[derive(Default)]
pub struct Shared {
    volume: AtomicU32,
    balance: AtomicU32,
    pub paused: AtomicBool,
    /// Set by the engine to ask the callback to drop buffered audio; cleared by the callback.
    pub flush: AtomicBool,
    /// Frames sent to the device since the last flush.
    pub played_frames: AtomicU64,
    pub eq: SharedEq,
    pub vis: VisTap,
}

impl Shared {
    pub fn new() -> Self {
        let s = Self::default();
        s.set_volume(1.0);
        s
    }
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }
    pub fn set_volume(&self, v: f32) {
        self.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn balance(&self) -> f32 {
        f32::from_bits(self.balance.load(Ordering::Relaxed))
    }
    pub fn set_balance(&self, b: f32) {
        self.balance.store(b.clamp(-1.0, 1.0).to_bits(), Ordering::Relaxed);
    }
}

pub struct Output {
    _stream: cpal::Stream,
    pub producer: HeapProd<f32>,
    pub rate: u32,
    pub channels: usize,
}

impl Output {
    /// Open the default device, preferring `rate`/`channels` so no conversion is needed.
    pub fn open(shared: Arc<Shared>, rate: u32, channels: usize) -> Result<Self, EngineError> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| EngineError::Output("no output device".into()))?;
        let supported = pick_config(&device, rate, channels.min(2) as u16)?;
        let config: StreamConfig = supported.config();
        let out_rate = config.sample_rate;
        let out_channels = config.channels as usize;

        let capacity = (out_rate as f32 * BUFFER_SECS) as usize * out_channels;
        let (producer, consumer) = HeapRb::<f32>::new(capacity).split();

        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, consumer, shared, out_channels),
            SampleFormat::I16 => build::<i16>(&device, config, consumer, shared, out_channels),
            SampleFormat::I32 => build::<i32>(&device, config, consumer, shared, out_channels),
            SampleFormat::U16 => build::<u16>(&device, config, consumer, shared, out_channels),
            SampleFormat::U8 => build::<u8>(&device, config, consumer, shared, out_channels),
            other => return Err(EngineError::Output(format!("unsupported sample format {other}"))),
        }?;
        stream.play().map_err(|e| EngineError::Output(e.to_string()))?;
        log::info!("output opened: {out_rate} Hz, {out_channels} ch");

        Ok(Self { _stream: stream, producer, rate: out_rate, channels: out_channels })
    }

    pub fn matches(&self, rate: u32, channels: usize) -> bool {
        self.rate == rate && self.channels == channels.min(2)
    }
}

fn pick_config(
    device: &cpal::Device,
    rate: u32,
    channels: u16,
) -> Result<SupportedStreamConfig, EngineError> {
    let err = |e: cpal::Error| EngineError::Output(e.to_string());
    let rank = |f: SampleFormat| match f {
        SampleFormat::F32 => 0,
        SampleFormat::I32 => 1,
        SampleFormat::I16 => 2,
        _ => 3,
    };
    let best = device
        .supported_output_configs()
        .map_err(err)?
        .filter(|r| r.channels() == channels && r.contains_rate(rate))
        .min_by_key(|r| rank(r.sample_format()));
    match best {
        Some(range) => Ok(range.with_sample_rate(rate)),
        None => device.default_output_config().map_err(err),
    }
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut consumer: ringbuf::HeapCons<f32>,
    shared: Arc<Shared>,
    channels: usize,
) -> Result<cpal::Stream, EngineError>
where
    T: SizedSample + FromSample<f32>,
{
    let mut scratch: Vec<f32> = Vec::new();
    let mut eq = Equalizer::new(config.sample_rate, channels);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                if shared.flush.load(Ordering::Acquire) {
                    consumer.clear();
                    shared.played_frames.store(0, Ordering::Relaxed);
                    shared.flush.store(false, Ordering::Release);
                }
                if shared.paused.load(Ordering::Relaxed) {
                    data.fill(T::EQUILIBRIUM);
                    return;
                }

                if scratch.len() < data.len() {
                    scratch.resize(data.len(), 0.0);
                }
                let buf = &mut scratch[..data.len()];
                let got = consumer.pop_slice(buf);
                buf[got..].fill(0.0); // underrun: pad with silence
                eq.sync(&shared.eq);
                eq.process(&mut buf[..got]);
                // Tap before volume/balance, like the original's visualizer.
                shared.vis.push(&buf[..got], channels);

                let vol = shared.volume();
                let (gl, gr) = if channels == 2 {
                    let b = shared.balance();
                    (vol * (1.0 - b).min(1.0), vol * (1.0 + b).min(1.0))
                } else {
                    (vol, vol)
                };
                for (frame_out, frame_in) in data.chunks_mut(channels).zip(buf.chunks(channels)) {
                    for (c, (o, s)) in frame_out.iter_mut().zip(frame_in).enumerate() {
                        let g = if c == 1 { gr } else { gl };
                        *o = T::from_sample(s * g);
                    }
                }
                shared.played_frames.fetch_add((got / channels) as u64, Ordering::Relaxed);
            },
            |e| log::error!("output stream error: {e}"),
            None,
        )
        .map_err(|e| EngineError::Output(e.to_string()))
}
