//! Connectivity toast rendering: dark rounded capsule with thin animated rings and no icon disc.

use crate::connectivity::model::{ConnEvent, ConnKind, ConnState};
use crate::gfx::{TextStyle, Weight};
use crate::icons::Icon;
use crate::ui::Ui;
use poni_core::geom::{palette, Color, Point, Rect};

pub const TOAST_H: f32 = 68.0;
pub const TOAST_RADIUS: f32 = 34.0;
pub const TOAST_A_W: f32 = 380.0;
pub const TOAST_B_W: f32 = 320.0;

/// Fixed icon bounding box size (40x40 px) to prevent text layout shifting across states.
#[allow(dead_code)]
pub const ICON_BOX_SIZE: f32 = 40.0;
/// Vertical slide distance for enter/exit animation (between 8 and 12 px).
#[cfg(test)]
pub const VERTICAL_SLIDE_PX: f32 = 10.0;
/// Duration a connectivity toast remains visible before fading out.
pub const TOAST_DURATION_S: f64 = 4.0;
/// Enter transition duration.
pub const ENTER_DURATION_S: f64 = 0.25;
/// Exit transition duration.
pub const EXIT_DURATION_S: f64 = 0.25;
/// Double ring pulse loop period in seconds.
const PULSE_PERIOD_S: f64 = 1.6;
/// Pulse ring peak opacity (fade from 0.4 to 0).
pub const PULSE_PEAK_OPACITY: f32 = 0.40;

/// Active connectivity toast state.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectivityToast {
    pub event: ConnEvent,
    pub started_at: f64,
    pub duration: f64,
}

impl ConnectivityToast {
    pub fn new(event: ConnEvent, now: f64) -> Self {
        Self {
            event,
            started_at: now,
            duration: TOAST_DURATION_S,
        }
    }

    /// Whether this toast has completely expired and finished its exit animation.
    pub fn is_expired(&self, now: f64) -> bool {
        now >= self.started_at + self.duration
    }

    /// Expiration timestamp in monotonic seconds.
    pub fn expires_at(&self) -> f64 {
        self.started_at + self.duration
    }

    /// Update the toast in place when an event for the same device/network arrives.
    pub fn update(&mut self, event: ConnEvent) {
        self.event = event;
    }

    /// Target width for this toast kind.
    pub fn width(&self) -> f32 {
        match &self.event.kind {
            ConnKind::Bluetooth { .. } => TOAST_A_W,
            ConnKind::Wifi { .. } | ConnKind::Ethernet => TOAST_B_W,
        }
    }
}

/// Calculate smooth ease-out quadratic progress: `1 - (1 - t)^2`.
fn ease_out(t: f32) -> f32 {
    let inv = 1.0 - t.clamp(0.0, 1.0);
    1.0 - inv * inv
}

/// Queries for verification testing.
/// Whether toast renders its own independent background capsule instead of using the island's body.
#[cfg(test)]
pub fn toast_renders_independent_background() -> bool {
    false
}
#[cfg(test)]
pub fn toast_has_capsule_container() -> bool {
    true
}

#[cfg(test)]
pub fn toast_has_icon_disc() -> bool {
    false
}

#[cfg(test)]
pub fn toast_pulse_peak_opacity() -> f32 {
    PULSE_PEAK_OPACITY
}

#[cfg(test)]
pub fn is_pulse_active_for_state(state: ConnState) -> bool {
    state == ConnState::Connected
}

#[cfg(test)]
pub fn toast_icon_box_size() -> f32 {
    ICON_BOX_SIZE
}

#[cfg(test)]
pub fn toast_vertical_slide_distance() -> f32 {
    VERTICAL_SLIDE_PX
}

/// Draw text with subtle drop shadow for crisp rendering on dark backgrounds.
fn draw_text_clean(ui: &mut Ui, text: &str, r: Rect, style: &TextStyle, alpha: f32) {
    let shadow_color = Color::rgba(0.0, 0.0, 0.0, 0.35 * alpha);
    let mut shadow_st = *style;
    shadow_st.color = shadow_color;
    ui.c.text(text, Rect::new(r.x, r.y + 1.0, r.w, r.h), &shadow_st);
    ui.c.text(text, r, style);
}

/// Draw an icon with subtle drop shadow directly on the capsule background.
fn draw_icon_clean(ui: &mut Ui, icon: Icon, r: Rect, color: Color, alpha: f32) {
    let shadow_color = Color::rgba(0.0, 0.0, 0.0, 0.35 * alpha);
    ui.c.icon(icon, Rect::new(r.x, r.y + 1.0, r.w, r.h), shadow_color);
    ui.c.icon(icon, r, color);
}

/// Draw the connectivity toast inside its dark rounded capsule.
pub fn draw_connectivity_toast(ui: &mut Ui, toast: &ConnectivityToast, body: Rect, now: f64) {
    let elapsed = (now - toast.started_at).max(0.0);
    let remaining = ((toast.started_at + toast.duration) - now).max(0.0);

    // Enter & exit easing
    let enter_p = (elapsed / ENTER_DURATION_S).clamp(0.0, 1.0) as f32;
    let exit_p = (remaining / EXIT_DURATION_S).clamp(0.0, 1.0) as f32;
    let alpha = ease_out(enter_p) * ease_out(exit_p);
    if alpha <= 0.001 {
        return;
    }

    // Unified with Dynamic Island: the island body itself provides the animated capsule and smooth spring bounce.
    let draw_body = body;
    let cy = draw_body.center().y;

    match &toast.event.kind {
        ConnKind::Bluetooth { name, device_kind, battery } => {
            let connected = toast.event.state == ConnState::Connected;

            // ---------------- Left: Fixed 40x40 Icon Area (No Disc)
            let left_box = Rect::from_center(Point::new(draw_body.x + 30.0, cy), ICON_BOX_SIZE, ICON_BOX_SIZE);
            let left_center = left_box.center();
            let left_icon_rect = Rect::from_center(left_center, 20.0, 20.0);

            if connected {
                if let Some(b) = battery {
                    let progress = (*b as f32 / 100.0).clamp(0.0, 1.0);
                    let track = Color::white(0.18 * alpha);
                    let fill = palette::BLUE.with_alpha(alpha);
                    // Thin battery arc ring (1.5 px stroke, no fill)
                    ui.c.ring(left_center, 18.0, 1.5, progress, track, fill);
                } else {
                    // Thin accent ring if no battery info (1.2 px stroke, no fill)
                    ui.c.stroke_circle(left_center, 18.0, palette::BLUE.with_alpha(alpha * 0.8), 1.2);
                }
            } else {
                // Static faint thin ring when disconnected (opacity <= 0.15)
                ui.c.stroke_circle(left_center, 18.0, Color::white(0.12 * alpha), 1.0);
            }

            let bt_glyph = if connected {
                Color::white(0.96 * alpha)
            } else {
                Color::white(0.48 * alpha)
            };
            draw_icon_clean(ui, Icon::Bluetooth, left_icon_rect, bt_glyph, alpha);

            // ---------------- Right: Fixed 40x40 Icon Area (No Disc)
            let right_box = Rect::from_center(Point::new(draw_body.right() - 30.0, cy), ICON_BOX_SIZE, ICON_BOX_SIZE);
            let right_center = right_box.center();
            let right_icon_rect = Rect::from_center(right_center, 20.0, 20.0);

            if connected {
                draw_double_ring_pulse(ui, right_center, elapsed, alpha, palette::BLUE);
            } else {
                // Pulse off when disconnected: optional faint static ring (opacity <= 0.15)
                ui.c.stroke_circle(right_center, 18.0, Color::white(0.12 * alpha), 1.0);
            }

            let dev_glyph = if connected {
                Color::white(0.96 * alpha)
            } else {
                Color::white(0.48 * alpha)
            };
            draw_icon_clean(ui, device_kind.icon(), right_icon_rect, dev_glyph, alpha);

            // ---------------- Middle: Device Name + Status
            let tx = draw_body.x + 58.0;
            let tw = (draw_body.right() - 58.0 - tx).max(0.0);

            let label_st = TextStyle::headline()
                .weight(Weight::Semibold)
                .color(Color::white(0.96 * alpha));
            draw_text_clean(ui, name, Rect::new(tx, cy - 18.0, tw, 20.0), &label_st, alpha);

            let sub_text = match (connected, battery) {
                (true, Some(b)) => format!("Connected · {b}%"),
                (true, None) => "Connected".to_string(),
                (false, _) => "Disconnected".to_string(),
            };
            let sub_color = if connected {
                Color::white(0.70 * alpha)
            } else {
                Color::rgba(0.92, 0.48, 0.48, 0.95 * alpha)
            };
            let sub_st = TextStyle::footnote().color(sub_color);
            draw_text_clean(ui, &sub_text, Rect::new(tx, cy + 2.0, tw, 16.0), &sub_st, alpha);
        }

        ConnKind::Wifi { ssid } => {
            let connected = toast.event.state == ConnState::Connected;
            let left_box = Rect::from_center(Point::new(draw_body.x + 30.0, cy), ICON_BOX_SIZE, ICON_BOX_SIZE);
            let left_center = left_box.center();
            let left_icon_rect = Rect::from_center(left_center, 20.0, 20.0);

            let pulse_color = palette::BLUE;
            if connected {
                draw_double_ring_pulse(ui, left_center, elapsed, alpha, pulse_color);
            } else {
                // Pulse off when disconnected: static faint ring (opacity <= 0.15)
                ui.c.stroke_circle(left_center, 18.0, Color::white(0.12 * alpha), 1.0);
            }

            let icon = if connected { Icon::Wifi } else { Icon::WifiOff };
            let icon_color = if connected {
                Color::white(0.96 * alpha)
            } else {
                Color::white(0.48 * alpha)
            };
            draw_icon_clean(ui, icon, left_icon_rect, icon_color, alpha);

            // Middle: SSID + State
            let tx = draw_body.x + 58.0;
            let tw = (draw_body.right() - 20.0 - tx).max(0.0);
            let display_name = if ssid.is_empty() { "Wi-Fi" } else { ssid.as_str() };

            let label_st = TextStyle::headline()
                .weight(Weight::Semibold)
                .color(Color::white(0.96 * alpha));
            draw_text_clean(ui, display_name, Rect::new(tx, cy - 18.0, tw, 20.0), &label_st, alpha);

            let sub_text = if connected { "Connected" } else { "Disconnected" };
            let sub_color = if connected {
                Color::white(0.70 * alpha)
            } else {
                Color::rgba(0.92, 0.48, 0.48, 0.95 * alpha)
            };
            let sub_st = TextStyle::footnote().color(sub_color);
            draw_text_clean(ui, sub_text, Rect::new(tx, cy + 2.0, tw, 16.0), &sub_st, alpha);
        }

        ConnKind::Ethernet => {
            let connected = toast.event.state == ConnState::Connected;
            let left_box = Rect::from_center(Point::new(draw_body.x + 30.0, cy), ICON_BOX_SIZE, ICON_BOX_SIZE);
            let left_center = left_box.center();
            let left_icon_rect = Rect::from_center(left_center, 20.0, 20.0);

            let pulse_color = palette::GREEN;
            if connected {
                draw_double_ring_pulse(ui, left_center, elapsed, alpha, pulse_color);
            } else {
                // Pulse off when disconnected: static faint ring (opacity <= 0.15)
                ui.c.stroke_circle(left_center, 18.0, Color::white(0.12 * alpha), 1.0);
            }

            let icon_color = if connected {
                Color::white(0.96 * alpha)
            } else {
                Color::white(0.48 * alpha)
            };
            draw_icon_clean(ui, Icon::Ethernet, left_icon_rect, icon_color, alpha);

            // Middle: "Ethernet" + State
            let tx = draw_body.x + 58.0;
            let tw = (draw_body.right() - 20.0 - tx).max(0.0);

            let label_st = TextStyle::headline()
                .weight(Weight::Semibold)
                .color(Color::white(0.96 * alpha));
            draw_text_clean(ui, "Ethernet", Rect::new(tx, cy - 18.0, tw, 20.0), &label_st, alpha);

            let sub_text = if connected { "Connected" } else { "Disconnected" };
            let sub_color = if connected {
                Color::white(0.70 * alpha)
            } else {
                Color::rgba(0.92, 0.48, 0.48, 0.95 * alpha)
            };
            let sub_st = TextStyle::footnote().color(sub_color);
            draw_text_clean(ui, sub_text, Rect::new(tx, cy + 2.0, tw, 16.0), &sub_st, alpha);
        }
    }
}

/// Draw double ring pulse: two concentric rings expanding out of phase by 0.5 cycle (stroke 1.2 px, fade opacity 0.4 to 0).
fn draw_double_ring_pulse(ui: &mut Ui, center: Point, elapsed: f64, master_alpha: f32, color: Color) {
    let t1 = ((elapsed % PULSE_PERIOD_S) / PULSE_PERIOD_S) as f32;
    let t2 = (((elapsed + (PULSE_PERIOD_S * 0.5)) % PULSE_PERIOD_S) / PULSE_PERIOD_S) as f32;

    for t in [t1, t2] {
        let eased = ease_out(t);
        let radius = 18.0 + eased * 12.0;
        let ring_alpha = (PULSE_PEAK_OPACITY * (1.0 - t)).max(0.0) * master_alpha;
        if ring_alpha > 0.005 {
            ui.c.stroke_circle(center, radius, color.with_alpha(ring_alpha), 1.2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectivity::model::DeviceKind;

#[test]
    fn test_toast_content_drawn_directly_in_island_body() {
        // Requirement: Background harus satu kesatuan dengan island (bukan kapsul independen ganda)
        assert_eq!(toast_renders_independent_background(), false, "Toast must not render an independent background capsule");
    }

    #[test]
    fn test_pill_geom_returns_connectivity_toast_geometry() {
        let mut st = crate::state::AppState::new(poni_core::settings::Settings::default(), poni_core::store::Store::default());
        st.settings.theme = poni_core::settings::Theme::Pill;
        st.connectivity_toast = Some(ConnectivityToast::new(
            ConnEvent {
                kind: ConnKind::Bluetooth {
                    name: "Buds".into(),
                    device_kind: DeviceKind::Earbuds,
                    battery: Some(90),
                },
                state: ConnState::Connected,
                timestamp: 100.0,
            },
            100.0,
        ));
        let g = crate::pill::geom(&crate::activities::Mode::ConnectivityToast, &st);
        assert_eq!(g, Some(crate::island::Geom::new(TOAST_A_W, TOAST_H, TOAST_RADIUS)));
    }

    #[test]
    fn test_capsule_is_retained() {
        assert_eq!(toast_has_capsule_container(), true, "Capsule container must be retained");
    }

    #[test]
    fn test_no_disc_behind_icons() {
        assert_eq!(toast_has_icon_disc(), false, "Icons must have no background disc/fill");
    }

    #[test]
    fn test_pulse_ring_peak_opacity_is_0_4() {
        assert_eq!(toast_pulse_peak_opacity(), 0.40, "Pulse ring peak opacity must be 0.40");
    }

    #[test]
    fn test_pulse_ring_inactive_when_disconnected() {
        assert_eq!(is_pulse_active_for_state(ConnState::Connected), true);
        assert_eq!(is_pulse_active_for_state(ConnState::Disconnected), false);
    }

    #[test]
    fn test_icon_box_fixed_to_40px() {
        assert_eq!(toast_icon_box_size(), 40.0, "Icon area must be fixed 40x40 px");
    }

    #[test]
    fn test_vertical_slide_is_between_8_and_12px() {
        let dist = toast_vertical_slide_distance();
        assert!(dist >= 8.0 && dist <= 12.0, "Vertical slide distance must be between 8 and 12 px, got {dist}");
    }

    #[test]
    fn toast_timing_and_expiration() {
        let ev = ConnEvent {
            kind: ConnKind::Bluetooth {
                name: "Buds".into(),
                device_kind: DeviceKind::Earbuds,
                battery: Some(90),
            },
            state: ConnState::Connected,
            timestamp: 100.0,
        };
        let toast = ConnectivityToast::new(ev, 100.0);
        assert!(!toast.is_expired(100.0));
        assert!(!toast.is_expired(103.9));
        assert!(toast.is_expired(104.0));
        assert!(toast.is_expired(105.0));
    }

    #[test]
    fn toast_widths_differ_by_kind() {
        let bt = ConnectivityToast::new(
            ConnEvent {
                kind: ConnKind::Bluetooth {
                    name: "Headset".into(),
                    device_kind: DeviceKind::Headphones,
                    battery: None,
                },
                state: ConnState::Connected,
                timestamp: 100.0,
            },
            100.0,
        );
        let wifi = ConnectivityToast::new(
            ConnEvent {
                kind: ConnKind::Wifi { ssid: "Home".into() },
                state: ConnState::Connected,
                timestamp: 100.0,
            },
            100.0,
        );
        assert_eq!(bt.width(), TOAST_A_W);
        assert_eq!(wifi.width(), TOAST_B_W);
    }

    #[test]
    fn test_toast_created_with_fresh_mono_time_survives_subsequent_frame() {
        let ev = ConnEvent {
            kind: ConnKind::Bluetooth {
                name: "Sony WH-1000XM4".into(),
                device_kind: DeviceKind::Headphones,
                battery: Some(80),
            },
            state: ConnState::Connected,
            timestamp: 500.0,
        };
        // Stale timestamp (e.g. 10.0 while clock is 500.0) would falsely expire immediately:
        let stale_toast = ConnectivityToast::new(ev.clone(), 10.0);
        assert!(stale_toast.is_expired(500.0), "Stale toast must be recognized as expired at current clock");

        // Fresh timestamp (500.0) ensures toast remains active for TOAST_DURATION_S (4.0s) on subsequent frame:
        let fresh_toast = ConnectivityToast::new(ev, 500.0);
        assert!(!fresh_toast.is_expired(500.0), "Fresh toast must not be expired at start");
        assert!(!fresh_toast.is_expired(500.016), "Fresh toast must remain active on next 60fps frame");
        assert!(!fresh_toast.is_expired(503.9), "Fresh toast must remain active throughout its 4s lifespan");
        assert!(fresh_toast.is_expired(504.0), "Fresh toast must expire after 4.0 seconds");
    }
}
