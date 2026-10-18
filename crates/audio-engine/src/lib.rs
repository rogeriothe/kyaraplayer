//! Playback engine: decode (symphonia) → convert → device output (cpal).

mod convert;
mod decoder;
mod engine;
pub mod eq;
pub mod eqf;
mod output;
pub mod stream;
pub mod vis;

pub use engine::{Engine, Event, PlayerState, Status, TrackInfo};
pub use eq::EqSettings;
pub use vis::VisFrame;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported media: {0}")]
    Unsupported(String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("audio output error: {0}")]
    Output(String),
    #[error("stream error: {0}")]
    Stream(String),
}
