//! Privacy indicator models and colors (camera & microphone activity).

use serde::{Deserialize, Serialize};

/// Privacy dot color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivacyColor {
    /// Emerald green for active camera: #4ADE80
    Green,
    /// Amber orange for active microphone: #FB923C
    Orange,
}

impl PrivacyColor {
    /// RGB components (0..=255)
    pub const fn rgb(self) -> (u8, u8, u8) {
        match self {
            Self::Green => (0x4a, 0xde, 0x80),
            Self::Orange => (0xfb, 0x92, 0x3c),
        }
    }

    /// Hex color value as 0xRRGGBB
    pub const fn hex(self) -> u32 {
        let (r, g, b) = self.rgb();
        ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
    }
}

/// Determines whether a hardware sensor (camera/mic) capability session is actively in use.
/// Evaluates both traditional active sessions (stop_time == 0) and modern sessions
/// where a new session start timestamp exceeds the previous stop timestamp (start_time > stop_time).
pub const fn is_usage_active(start_time: u64, stop_time: u64) -> bool {
    (stop_time == 0 && start_time > 0) || (start_time > stop_time)
}

/// The state of active privacy sensors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyState {
    pub camera: bool,
    pub microphone: bool,
}

impl PrivacyState {
    pub const fn new(camera: bool, microphone: bool) -> Self {
        Self { camera, microphone }
    }

    /// Resolves which dot color to display, with Camera taking priority over Microphone.
    pub const fn active_dot(self) -> Option<PrivacyColor> {
        if self.camera {
            Some(PrivacyColor::Green)
        } else if self.microphone {
            Some(PrivacyColor::Orange)
        } else {
            None
        }
    }
}
