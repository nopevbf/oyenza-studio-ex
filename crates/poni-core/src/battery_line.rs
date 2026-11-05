//! Battery line indicator math and color styling for the Dynamic Island.

/// Determines if the 2px battery indicator line should be drawn.
pub fn should_show_battery_line(has_battery: bool, setting_enabled: bool) -> bool {
    has_battery && setting_enabled
}

/// Calculates the width of the active battery line in DIPs.
pub fn battery_bar_width(percent: u8, total_width: f32) -> f32 {
    let pct = percent.min(100) as f32 / 100.0;
    (total_width * pct).max(0.0)
}

/// Resolves the RGB color of the battery line based on battery percentage.
/// Full (> 50%): Green (#22C55E)
/// Medium (16% - 50%): Yellow (#EAB308)
/// Low (<= 15%): Red (#EF4444)
pub fn battery_bar_color(percent: u8, _charging: bool) -> (u8, u8, u8) {
    if percent > 50 {
        (0x22, 0xc5, 0x5e)
    } else if percent > 15 {
        (0xea, 0xb3, 0x08)
    } else {
        (0xef, 0x44, 0x44)
    }
}
