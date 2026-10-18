//! Engine thread: owns the decoder and the output device, driven by commands.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use ringbuf::traits::{Observer, Producer};
use serde::Serialize;

use crate::convert::{remap_channels, Resampler};
use crate::decoder::Source;
use crate::output::{Output, Shared};
use crate::stream::StreamState;
use crate::EngineError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayerState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackInfo {
    pub path: PathBuf,
    pub duration: Option<f64>,
    /// Average bitrate in kbps.
    pub bitrate: Option<u32>,
    pub sample_rate: u32,
    pub channels: usize,
    /// Station name, for network streams.
    pub stream_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub state: PlayerState,
    pub position: f64,
    pub volume: f32,
    pub balance: f32,
    pub track: Option<TrackInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    StateChanged { state: PlayerState },
    TrackStarted { track: TrackInfo },
    /// Opening a network stream; `TrackStarted` or `Error` follows.
    Connecting { path: PathBuf },
    /// A network stream announced a new song title (ICY `StreamTitle`).
    StreamTitle { title: String },
    /// Current track reached its end and the buffer drained.
    TrackEnded,
    Error { message: String },
}

enum Command {
    Play(PathBuf),
    Pause,
    Resume,
    Stop,
    Seek(f64),
    Shutdown,
}

struct Inner {
    state: PlayerState,
    track: Option<TrackInfo>,
    /// Track position (seconds) that corresponds to `played_frames == 0`.
    base_position: f64,
    out_rate: u32,
}

/// Handle to the playback engine. Cheap to share behind an `Arc`.
pub struct Engine {
    commands: Sender<Command>,
    events: Receiver<Event>,
    shared: Arc<Shared>,
    status: Arc<Mutex<Inner>>,
    analyzer: Mutex<crate::vis::Analyzer>,
    /// Network state of the stream being opened or played, aborted on stop/play so a
    /// stalled server never holds up the next command.
    stream: Arc<Mutex<Arc<StreamState>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Engine {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = unbounded();
        let (evt_tx, evt_rx) = unbounded();
        let shared = Arc::new(Shared::new());
        let status = Arc::new(Mutex::new(Inner {
            state: PlayerState::Stopped,
            track: None,
            base_position: 0.0,
            out_rate: 44_100,
        }));
        let stream = Arc::new(Mutex::new(Arc::new(StreamState::default())));

        let worker = Worker {
            commands: cmd_rx,
            events: evt_tx,
            shared: shared.clone(),
            status: status.clone(),
            output: None,
            source: None,
            resampler: None,
            mixed: Vec::new(),
            converted: Vec::new(),
            pending: 0,
            draining: false,
            stream: stream.clone(),
        };
        let thread = thread::Builder::new()
            .name("audio-engine".into())
            .spawn(move || worker.run())
            .expect("spawn audio engine thread");

        Self {
            commands: cmd_tx,
            events: evt_rx,
            shared,
            status,
            analyzer: Mutex::new(crate::vis::Analyzer::default()),
            stream,
            thread: Some(thread),
        }
    }

    fn abort_stream(&self) {
        self.stream.lock().unwrap().abort.store(true, Ordering::Relaxed);
    }

    /// Play a file or an `http(s)://` stream.
    pub fn play(&self, path: impl Into<PathBuf>) {
        self.abort_stream();
        self.send(Command::Play(path.into()));
    }
    pub fn pause(&self) {
        self.send(Command::Pause);
    }
    pub fn resume(&self) {
        self.send(Command::Resume);
    }
    /// Winamp semantics: pause button toggles between paused and playing.
    pub fn toggle_pause(&self) {
        match self.state() {
            PlayerState::Playing => self.pause(),
            PlayerState::Paused => self.resume(),
            PlayerState::Stopped => {}
        }
    }
    pub fn stop(&self) {
        self.abort_stream();
        self.send(Command::Stop);
    }
    pub fn seek(&self, secs: f64) {
        self.send(Command::Seek(secs));
    }
    pub fn set_volume(&self, v: f32) {
        self.shared.set_volume(v);
    }
    pub fn set_balance(&self, b: f32) {
        self.shared.set_balance(b);
    }
    /// Takes effect on the next device callback.
    pub fn set_eq(&self, settings: &crate::EqSettings) {
        self.shared.eq.set(settings);
    }
    pub fn eq(&self) -> crate::EqSettings {
        self.shared.eq.get()
    }

    pub fn state(&self) -> PlayerState {
        self.status.lock().unwrap().state
    }

    pub fn status(&self) -> Status {
        let s = self.status.lock().unwrap();
        let played = self.shared.played_frames.load(Ordering::Relaxed) as f64;
        let position = match s.state {
            PlayerState::Stopped => 0.0,
            _ => s.base_position + played / s.out_rate as f64,
        };
        Status {
            state: s.state,
            position,
            volume: self.shared.volume(),
            balance: self.shared.balance(),
            track: s.track.clone(),
        }
    }

    /// Current visualizer frame computed from the audio being played.
    pub fn vis_frame(&self) -> crate::VisFrame {
        let rate = self.status.lock().unwrap().out_rate;
        self.analyzer.lock().unwrap().frame(&self.shared.vis, rate)
    }

    /// Engine events. Each event is delivered to exactly one receiver.
    pub fn events(&self) -> Receiver<Event> {
        self.events.clone()
    }

    fn send(&self, cmd: Command) {
        // The worker only exits on Shutdown, so a send failure means it panicked.
        if self.commands.send(cmd).is_err() {
            log::error!("audio engine thread is gone");
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.abort_stream();
        let _ = self.commands.send(Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Worker {
    commands: Receiver<Command>,
    events: Sender<Event>,
    shared: Arc<Shared>,
    status: Arc<Mutex<Inner>>,
    output: Option<Output>,
    source: Option<Source>,
    resampler: Option<Resampler>,
    /// Decoded audio remapped to the device channel count.
    mixed: Vec<f32>,
    /// Audio ready for the device, not yet pushed to the ring buffer.
    converted: Vec<f32>,
    /// Read offset into `converted`.
    pending: usize,
    /// Source ended; waiting for the device to play out the buffer.
    draining: bool,
    stream: Arc<Mutex<Arc<StreamState>>>,
}

impl Worker {
    fn run(mut self) {
        loop {
            let busy = self.source.is_some() && self.state() == PlayerState::Playing;
            let cmd = if busy {
                self.commands.try_recv().ok()
            } else {
                match self.commands.recv_timeout(Duration::from_millis(50)) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            };

            if let Some(cmd) = cmd {
                if !self.handle(cmd) {
                    return;
                }
                continue;
            }
            if busy {
                self.pump();
            }
        }
    }

    /// Returns false on shutdown.
    fn handle(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Play(path) => {
                if let Err(e) = self.start(path) {
                    // A stream cancelled by stop/play while connecting is not an error.
                    if self.stream.lock().unwrap().abort.load(Ordering::Relaxed) {
                        self.stop();
                    } else {
                        self.fail(e);
                    }
                }
            }
            Command::Pause => {
                if self.state() == PlayerState::Playing {
                    self.shared.paused.store(true, Ordering::Relaxed);
                    self.set_state(PlayerState::Paused);
                }
            }
            Command::Resume => {
                if self.state() == PlayerState::Paused {
                    self.shared.paused.store(false, Ordering::Relaxed);
                    self.set_state(PlayerState::Playing);
                }
            }
            Command::Stop => self.stop(),
            Command::Seek(secs) => {
                if let Some(src) = self.source.as_mut() {
                    // Seeking past the end finishes the track, like Winamp does.
                    if src.duration().is_some_and(|d| secs >= d) {
                        let end = src.duration().unwrap_or(secs);
                        self.flush(end);
                        self.draining = true;
                        return true;
                    }
                    match src.seek(secs) {
                        Ok(pos) => {
                            self.flush(pos);
                            self.draining = false;
                        }
                        Err(e) => log::warn!("seek failed: {e}"),
                    }
                }
            }
            Command::Shutdown => {
                self.stop();
                return false;
            }
        }
        true
    }

    fn start(&mut self, path: PathBuf) -> Result<(), EngineError> {
        self.stop();
        let stream = Arc::new(StreamState::default());
        *self.stream.lock().unwrap() = stream.clone();
        if path.to_str().is_some_and(crate::stream::is_url) {
            self.emit(Event::Connecting { path: path.clone() });
        }
        let source = Source::open(&path, &stream)?;
        let stream_name = source.stream_info().and_then(|i| i.name.clone());
        let (rate, channels) = source.format_hint();

        let reuse = self.output.as_ref().is_some_and(|o| o.matches(rate, channels));
        if !reuse {
            self.output = None; // close the old device stream before opening a new one
            self.output = Some(Output::open(self.shared.clone(), rate, channels)?);
        }
        let out = self.output.as_ref().expect("output just opened");
        self.status.lock().unwrap().out_rate = out.rate;

        let track = TrackInfo {
            path,
            duration: source.duration(),
            bitrate: source.bitrate(),
            sample_rate: rate,
            channels,
            stream_name,
        };
        self.source = Some(source);
        self.resampler = None;
        self.draining = false;
        self.shared.paused.store(false, Ordering::Relaxed);
        self.flush(0.0);
        self.status.lock().unwrap().track = Some(track.clone());
        self.emit(Event::TrackStarted { track });
        self.set_state(PlayerState::Playing);
        Ok(())
    }

    fn stop(&mut self) {
        if self.source.take().is_some() || self.state() != PlayerState::Stopped {
            self.flush(0.0);
            self.shared.paused.store(false, Ordering::Relaxed);
            self.draining = false;
            self.status.lock().unwrap().track = None;
            self.set_state(PlayerState::Stopped);
        }
    }

    /// Drop everything buffered and restart the position clock at `position`.
    fn flush(&mut self, position: f64) {
        self.converted.clear();
        self.pending = 0;
        if let Some(r) = self.resampler.as_mut() {
            r.reset();
        }
        self.status.lock().unwrap().base_position = position;
        if self.output.is_none() {
            self.shared.played_frames.store(0, Ordering::Relaxed);
            return;
        }
        self.shared.flush.store(true, Ordering::Release);
        // Wait for the callback to acknowledge so stale audio is not mixed with new audio.
        let deadline = Instant::now() + Duration::from_millis(200);
        while self.shared.flush.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        if self.shared.flush.swap(false, Ordering::AcqRel) {
            log::warn!("output did not acknowledge flush");
            self.shared.played_frames.store(0, Ordering::Relaxed);
        }
    }

    /// Move audio from the decoder into the device ring buffer.
    fn pump(&mut self) {
        let Some(out) = self.output.as_mut() else { return };

        // Push leftovers first.
        if self.pending < self.converted.len() {
            let n = out.producer.push_slice(&self.converted[self.pending..]);
            self.pending += n;
            if self.pending < self.converted.len() {
                thread::sleep(Duration::from_millis(5)); // ring full
                return;
            }
        }
        self.converted.clear();
        self.pending = 0;

        if self.draining {
            if out.producer.occupied_len() == 0 {
                self.draining = false;
                self.source = None;
                self.emit(Event::TrackEnded);
                self.stop();
            } else {
                thread::sleep(Duration::from_millis(10));
            }
            return;
        }

        let Some(src) = self.source.as_mut() else { return };
        if src.stream_info().is_some() {
            let stream = self.stream.lock().unwrap().clone();
            let title = stream.title.lock().unwrap().take();
            if let Some(title) = title {
                let _ = self.events.send(Event::StreamTitle { title });
            }
        }
        let (out_rate, out_channels) = (out.rate, out.channels);
        match src.next_chunk() {
            Ok(Some(chunk)) => {
                self.mixed.clear();
                remap_channels(chunk.samples, chunk.channels, out_channels, &mut self.mixed);
                if chunk.rate == out_rate {
                    std::mem::swap(&mut self.converted, &mut self.mixed);
                } else {
                    let rs = self
                        .resampler
                        .get_or_insert_with(|| Resampler::new(chunk.rate, out_rate, out_channels));
                    if !rs.matches(chunk.rate, out_rate, out_channels) {
                        *rs = Resampler::new(chunk.rate, out_rate, out_channels);
                    }
                    rs.process(&self.mixed, &mut self.converted);
                }
            }
            Ok(None) => {
                // Play out what the resampler still holds.
                if let Some(rs) = self.resampler.as_mut() {
                    rs.finish(&mut self.converted);
                }
                self.draining = true;
            }
            Err(e) => {
                log::error!("{e}");
                self.emit(Event::Error { message: e.to_string() });
                self.draining = true;
            }
        }
    }

    fn fail(&mut self, e: EngineError) {
        log::error!("{e}");
        self.stop();
        self.emit(Event::Error { message: e.to_string() });
    }

    fn state(&self) -> PlayerState {
        self.status.lock().unwrap().state
    }

    fn set_state(&self, state: PlayerState) {
        let changed = {
            let mut s = self.status.lock().unwrap();
            std::mem::replace(&mut s.state, state) != state
        };
        if changed {
            self.emit(Event::StateChanged { state });
        }
    }

    fn emit(&self, e: Event) {
        let _ = self.events.send(e);
    }
}
