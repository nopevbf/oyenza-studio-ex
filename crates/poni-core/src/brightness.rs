//! Screen brightness definitions, models, and boundary calculations.

use serde::{Deserialize, Serialize};

/// Clamps a brightness floating point value strictly to the range `[0.0, 1.0]`.
pub fn clamp_brightness(val: f32) -> f32 {
    val.clamp(0.0, 1.0)
}

/// Identifies whether the display is the laptop's internal panel or an external DDC/CI monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayKind {
    InternalLaptop,
    ExternalDdcCi,
}

/// Information describing the active display's brightness state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrightnessInfo {
    /// Brightness level between 0.0 and 1.0.
    pub level: f32,
    /// Friendly name or device path of the display.
    pub display_name: String,
    /// Kind of display (internal vs external DDC/CI).
    pub kind: DisplayKind,
    /// Whether hardware brightness control is supported by the display.
    pub is_supported: bool,
}

impl BrightnessInfo {
    /// Returns the integer percentage from 0 to 100.
    pub fn percent(&self) -> u32 {
        (clamp_brightness(self.level) * 100.0).round() as u32
    }
}
