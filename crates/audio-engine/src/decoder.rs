//! File decoding on top of symphonia. Produces interleaved f32 frames.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase, Timestamp};

use crate::stream::{self, StreamInfo, StreamState};
use crate::EngineError;

/// Decoded chunk of audio.
pub struct Chunk<'a> {
    pub samples: &'a [f32],
    pub rate: u32,
    pub channels: usize,
}

pub struct Source {
    format: Box<dyn FormatReader + 'static>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    duration: Option<f64>,
    bitrate: Option<u32>,
    rate: u32,
    channels: usize,
    buf: Vec<f32>,
    /// Frames to drop from the next decoded output (sample-accurate seek).
    skip_frames: usize,
    /// Set for network streams.
    stream: Option<StreamInfo>,
}

impl Source {
    /// Open a local file, or an `http(s)://` stream (aborted through `stream_state`).
    pub fn open(path: &Path, stream_state: &Arc<StreamState>) -> Result<Self, EngineError> {
        if let Some(url) = path.to_str().filter(|p| stream::is_url(p)) {
            let (reader, info) = stream::open(url, stream_state.clone())?;
            let mut hint = Hint::new();
            if let Some(ext) = info.hint {
                hint.with_extension(ext);
            }
            let mut source = Self::probe(Box::new(ReadOnlySource::new(reader)), hint, None)?;
            source.bitrate = info.bitrate;
            // A live stream has no end; headers copied from a file (Xing) say otherwise.
            source.duration = None;
            source.stream = Some(info);
            return Ok(source);
        }

        let file = File::open(path)?;
        let file_len = file.metadata().map(|m| m.len()).ok();
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        Self::probe(Box::new(file), hint, file_len)
    }

    fn probe(source: Box<dyn MediaSource>, hint: Hint, file_len: Option<u64>) -> Result<Self, EngineError> {
        let mss = MediaSourceStream::new(source, Default::default());

        let format = symphonia::default::get_probe()
            .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
            .map_err(|e| EngineError::Unsupported(e.to_string()))?;

        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| EngineError::Unsupported("no audio track".into()))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| EngineError::Unsupported("missing audio codec parameters".into()))?;

        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|e| EngineError::Unsupported(e.to_string()))?;

        let rate = params.sample_rate.unwrap_or(44_100);
        let channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(2);
        let time_base = track.time_base;
        let duration = match (time_base, track.duration, track.num_frames) {
            (Some(tb), Some(dur), _) => tb.calc_duration(dur).map(|t| t.as_secs_f64()),
            (_, _, Some(frames)) => Some(frames as f64 / rate as f64),
            _ => None,
        };
        // Average bitrate in kbps; containers rarely store it, so derive it from the size.
        let bitrate = match (file_len, duration) {
            (Some(len), Some(d)) if d > 0.0 => Some((len as f64 * 8.0 / d / 1000.0).round() as u32),
            _ => None,
        };

        Ok(Self {
            track_id: track.id,
            format,
            decoder,
            time_base,
            duration,
            bitrate,
            rate,
            channels,
            buf: Vec::new(),
            skip_frames: 0,
            stream: None,
        })
    }

    /// Server-provided details when this is a network stream.
    pub fn stream_info(&self) -> Option<&StreamInfo> {
        self.stream.as_ref()
    }

    pub fn duration(&self) -> Option<f64> {
        self.duration
    }

    /// Average bitrate in kbps, if the duration is known.
    pub fn bitrate(&self) -> Option<u32> {
        self.bitrate
    }

    /// Sample rate and channel count advertised by the container. Real values
    /// come with each decoded [`Chunk`].
    pub fn format_hint(&self) -> (u32, usize) {
        (self.rate, self.channels)
    }

    /// Decode the next chunk. `Ok(None)` means end of stream.
    pub fn next_chunk(&mut self) -> Result<Option<Chunk<'_>>, EngineError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(None),
                Err(SymError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                // End of file, or a stream aborted by stop/play (the command is already queued).
                Err(SymError::IoError(e))
                    if matches!(e.kind(), std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionAborted) =>
                {
                    return Ok(None)
                }
                Err(e) => return Err(EngineError::Decode(e.to_string())),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(audio) => {
                    let spec = audio.spec();
                    self.rate = spec.rate();
                    self.channels = spec.channels().count().max(1);
                    audio.copy_to_vec_interleaved(&mut self.buf);
                }
                // Corrupt packets are skipped, like every common player does.
                Err(SymError::DecodeError(e)) => {
                    log::warn!("decode error: {e}");
                    continue;
                }
                Err(SymError::IoError(_)) => continue,
                Err(e) => return Err(EngineError::Decode(e.to_string())),
            }

            let mut start = 0;
            if self.skip_frames > 0 {
                let frames = self.buf.len() / self.channels;
                let skip = self.skip_frames.min(frames);
                self.skip_frames -= skip;
                start = skip * self.channels;
            }
            if start >= self.buf.len() {
                continue;
            }
            return Ok(Some(Chunk {
                samples: &self.buf[start..],
                rate: self.rate,
                channels: self.channels,
            }));
        }
    }

    /// Seek to `secs`. Returns the position actually reached in seconds.
    pub fn seek(&mut self, secs: f64) -> Result<f64, EngineError> {
        let secs = secs.max(0.0);
        let time = Time::try_from_secs_f64(secs)
            .ok_or_else(|| EngineError::Decode(format!("invalid seek time {secs}")))?;
        let seeked = self
            .format
            .seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(self.track_id) })
            .map_err(|e| EngineError::Decode(e.to_string()))?;
        self.decoder.reset();

        let Some(tb) = self.time_base else {
            self.skip_frames = 0;
            return Ok(secs);
        };
        let to_secs = |ts: Timestamp| tb.calc_time(ts).map(|t| t.as_secs_f64());
        let required = to_secs(seeked.required_ts).unwrap_or(secs);
        let actual = to_secs(seeked.actual_ts).unwrap_or(required);
        // Accurate seek lands at or before the target; drop the difference.
        self.skip_frames = ((required - actual).max(0.0) * self.rate as f64).round() as usize;
        Ok(required)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
    }

    /// Decode everything; returns (frames, peak amplitude).
    fn decode_all(src: &mut Source) -> (usize, f32) {
        let (mut frames, mut peak) = (0, 0.0f32);
        while let Some(chunk) = src.next_chunk().unwrap() {
            frames += chunk.samples.len() / chunk.channels;
            peak = chunk.samples.iter().fold(peak, |p, s| p.max(s.abs()));
        }
        (frames, peak)
    }

    fn open(name: &str) -> Source {
        Source::open(&fixture(name), &Arc::new(StreamState::default())).unwrap()
    }

    #[test]
    fn flac_16_bit_decodes_with_duration() {
        let mut src = open("sine.flac");
        assert_eq!(src.format_hint(), (44_100, 2));
        assert!(src.duration().is_some_and(|d| (d - 0.5).abs() < 0.01));
        let (frames, peak) = decode_all(&mut src);
        assert_eq!(frames, 22_050);
        // Peak measured by ffmpeg's astats on the fixture: -21.07 dBFS.
        assert!((peak - 0.0884).abs() < 0.001, "peak {peak}");
    }

    #[test]
    fn flac_24_bit_96k_decodes_at_full_scale() {
        let mut src = open("hires.flac");
        assert_eq!(src.format_hint(), (96_000, 2));
        let (frames, peak) = decode_all(&mut src);
        assert_eq!(frames, 48_000);
        assert!((peak - 0.0884).abs() < 0.001, "24-bit samples mis-scaled: peak {peak}");
    }

    #[test]
    fn flac_seeks_sample_accurately() {
        let mut src = open("sine.flac");
        let pos = src.seek(0.25).unwrap();
        assert!((pos - 0.25).abs() < 0.001);
        let (frames, _) = decode_all(&mut src);
        assert!((frames as i64 - 11_025).abs() <= 1, "frames after seek: {frames}");
    }
}
