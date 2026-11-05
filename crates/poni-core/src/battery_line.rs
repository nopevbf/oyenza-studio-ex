//! Battery line indicator math and color styling for the Dynamic Island.

use crate::geom::Rect;
use crate::svgpath::Seg;

/// Determines if the 2px battery indicator line should be drawn.
pub fn should_show_battery_line(has_battery: bool, setting_enabled: bool) -> bool {
    has_battery && setting_enabled
}

/// Calculates the width of the active battery line in DIPs.
pub fn battery_bar_width(percent: u8, total_width: f32) -> f32 {
    let pct = percent.min(100) as f32 / 100.0;
    (total_width * pct).max(0.0)
}

/// Generates SVG path segments for the dynamic battery line indicator conforming to the bottom notch contour.
/// Follows the bottom curvature: left arc, flat bottom line, and right arc.
pub fn battery_contour_segments(body: Rect, radius: f32, percent: u8) -> Vec<Seg> {
    let p = (percent.min(100) as f32 / 100.0).max(0.0);
    if p <= 0.0 {
        return Vec::new();
    }

    let r = radius.min(body.h).min(body.w / 2.0).max(0.0);
    let (x0, x1, y1) = (body.x, body.right(), body.bottom());

    if r <= 0.0 {
        let active_w = body.w * p;
        return vec![
            Seg::MoveTo(x0, y1),
            Seg::LineTo(x0 + active_w, y1),
        ];
    }

    // Arc length = pi/2 * r
    let l_arc = std::f32::consts::FRAC_PI_2 * r;
    let l_mid = (body.w - 2.0 * r).max(0.0);
    let l_total = 2.0 * l_arc + l_mid;
    let l_active = p * l_total;

    let mut segs = Vec::with_capacity(4);
    // Start at bottom of left shoulder / top of left corner curve
    segs.push(Seg::MoveTo(x0, y1 - r));

    if l_active <= l_arc {
        // Partial left curve: angle theta from 0 to pi/2
        let theta = (l_active / r).min(std::f32::consts::FRAC_PI_2);
        let x = (x0 + r) - r * theta.cos();
        let y = (y1 - r) + r * theta.sin();
        segs.push(Seg::ArcTo(r, r, 0.0, false, false, x, y));
    } else if l_active <= l_arc + l_mid {
        // Full left curve + partial flat bottom
        segs.push(Seg::ArcTo(r, r, 0.0, false, false, x0 + r, y1));
        let d = l_active - l_arc;
        segs.push(Seg::LineTo(x0 + r + d, y1));
    } else {
        // Full left curve + full flat bottom + partial/full right curve
        segs.push(Seg::ArcTo(r, r, 0.0, false, false, x0 + r, y1));
        segs.push(Seg::LineTo(x1 - r, y1));
        let d = l_active - (l_arc + l_mid);
        let theta = (d / r).min(std::f32::consts::FRAC_PI_2);
        let x = (x1 - r) + r * theta.sin();
        let y = (y1 - r) + r * theta.cos();
        segs.push(Seg::ArcTo(r, r, 0.0, false, false, x, y));
    }

    segs
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
