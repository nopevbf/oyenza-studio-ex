//! Dynamic eyes moods and idle glove wave tracking.

use serde::{Deserialize, Serialize};

/// Threshold in seconds of continuous cursor inactivity before waving starts.
pub const WAVE_IDLE_THRESHOLD: f64 = 30.0;
/// Duration in seconds of the waving glove animation before resting.
pub const WAVE_DURATION: f64 = 2.5;

/// Available eye expressions for the dynamic eyes look.
/// Note: 'Love' mood is explicitly excluded per design requirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EyeMood {
    Blink,
    Flutter,
    Squint,
    Smile,
    Sleep,
    Sly,
    SlyLeft,
    Read,
    Think,
    Wave,
}

impl EyeMood {
    pub const ALL: &'static [EyeMood] = &[
        EyeMood::Blink,
        EyeMood::Flutter,
        EyeMood::Squint,
        EyeMood::Smile,
        EyeMood::Sleep,
        EyeMood::Sly,
        EyeMood::SlyLeft,
        EyeMood::Read,
        EyeMood::Think,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Blink => "blink",
            Self::Flutter => "flutter",
            Self::Squint => "squint",
            Self::Smile => "smile",
            Self::Sleep => "sleep",
            Self::Sly => "sly",
            Self::SlyLeft => "sly_left",
            Self::Read => "read",
            Self::Think => "think",
            Self::Wave => "wave",
        }
    }

    /// Selects a pseudo-random mood based on an integer seed.
    pub fn from_seed(seed: u64) -> Self {
        let idx = (seed as usize) % Self::ALL.len();
        Self::ALL[idx]
    }
}

/// Tracks cursor inactivity and controls the special idle glove wave.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IdleTracker {
    idle_secs: f64,
    wave_triggered: bool,
}

impl IdleTracker {
    pub const fn new() -> Self {
        Self {
            idle_secs: 0.0,
            wave_triggered: false,
        }
    }

    pub fn idle_secs(&self) -> f64 {
        self.idle_secs
    }

    /// Advances the idle timer by `dt` seconds, resetting if `mouse_moved` is true.
    pub fn tick(&mut self, dt: f64, mouse_moved: bool) {
        if mouse_moved {
            self.idle_secs = 0.0;
            self.wave_triggered = false;
        } else {
            self.idle_secs += dt;
        }
    }

    /// True if the island should currently be rendering the waving gloves.
    pub fn is_waving(&self) -> bool {
        self.idle_secs >= WAVE_IDLE_THRESHOLD && self.idle_secs < (WAVE_IDLE_THRESHOLD + WAVE_DURATION)
    }

    /// Reset idle tracker directly.
    pub fn reset(&mut self) {
        self.idle_secs = 0.0;
        self.wave_triggered = false;
    }
}
