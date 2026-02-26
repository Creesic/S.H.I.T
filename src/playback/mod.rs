pub mod engine;

pub use engine::PlaybackEngine;

use crate::core::CanMessage;
use chrono::{DateTime, Utc};

/// Playback state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// Playback configuration
#[derive(Debug, Clone)]
pub struct PlaybackConfig {
    pub speed: f64,  // 1.0 = real-time, 2.0 = 2x speed
    pub loop_playback: bool,
}
