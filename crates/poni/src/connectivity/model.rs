//! Domain model and types for connectivity events, device kinds, and debouncing.

use crate::icons::Icon;
use std::collections::HashMap;

/// Detected Bluetooth device kind based on Class of Device or GATT Appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Headphones,
    Earbuds,
    Speaker,
    Keyboard,
    Mouse,
    Gamepad,
    Phone,
    Generic,
}

impl DeviceKind {
    /// Return the corresponding icon for this device kind.
    pub fn icon(self) -> Icon {
        match self {
            Self::Headphones => Icon::Headphones,
            Self::Earbuds => Icon::Earbuds,
            Self::Speaker => Icon::Speaker,
            Self::Keyboard => Icon::Keyboard,
            Self::Mouse => Icon::Mouse,
            Self::Gamepad => Icon::Gamepad,
            Self::Phone => Icon::Phone,
            Self::Generic => Icon::Bluetooth,
        }
    }

    /// Decode Classic Bluetooth Class of Device (major, minor).
    pub fn from_cod(major: u32, minor: u32) -> Self {
        match major {
            // Phone (cellular, cordless, smart phone)
            0x02 => Self::Phone,
            // Audio / Video
            0x04 => match minor {
                1 => Self::Earbuds,     // Wearable Headset
                2 => Self::Headphones,  // Hands-free
                5 => Self::Speaker,     // Loudspeaker
                6 => Self::Headphones,  // Headphones
                7 => Self::Earbuds,     // Portable Audio
                8 => Self::Speaker,     // Car Audio
                _ => Self::Headphones,
            },
            // Peripheral (HID)
            0x05 => {
                // Bits 4/5 or 6/7 indicate keyboard / pointing device
                if minor & 0x10 != 0 || minor & 0x40 != 0 {
                    Self::Keyboard
                } else if minor & 0x20 != 0 || minor & 0x80 != 0 {
                    Self::Mouse
                } else if minor & 0x01 != 0 || minor & 0x02 != 0 {
                    Self::Gamepad
                } else {
                    Self::Generic
                }
            }
            _ => Self::Generic,
        }
    }

    /// Decode raw 24-bit Class of Device integer.
    #[allow(dead_code)]
    pub fn from_raw_cod(cod: u32) -> Self {
        let major = (cod >> 8) & 0x1F;
        let minor = (cod >> 2) & 0x3F;
        Self::from_cod(major, minor)
    }

    /// Decode Bluetooth Low Energy Appearance category (and subcategory if present).
    #[allow(dead_code)]
    pub fn from_ble_appearance(category: u32, subcategory: u32) -> Self {
        match category {
            0x01 => Self::Phone,
            // HID category (960 >> 6 = 15 = 0x0F)
            0x0F => match subcategory {
                1 => Self::Keyboard,
                2 => Self::Mouse,
                3 | 4 => Self::Gamepad,
                _ => Self::Generic,
            },
            // Audio sink / AV
            0x21 | 0x25 => Self::Speaker,
            // Wearable audio (category 0x27)
            0x27 => match subcategory {
                1 => Self::Earbuds,
                2 => Self::Headphones,
                _ => Self::Earbuds,
            },
            // Hearing Aid
            0x29 => Self::Earbuds,
            _ => Self::Generic,
        }
    }
}

/// Connection state (Connected or Disconnected).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnState {
    Connected,
    Disconnected,
}

/// Connection target kind.
#[derive(Clone, Debug, PartialEq)]
pub enum ConnKind {
    Bluetooth {
        name: String,
        device_kind: DeviceKind,
        battery: Option<u8>,
    },
    Wifi {
        ssid: String,
    },
    Ethernet,
}

/// A connectivity state change event.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnEvent {
    pub kind: ConnKind,
    pub state: ConnState,
    pub timestamp: f64,
}

impl ConnEvent {
    pub fn name(&self) -> &str {
        match &self.kind {
            ConnKind::Bluetooth { name, .. } => name.as_str(),
            ConnKind::Wifi { ssid } => ssid.as_str(),
            ConnKind::Ethernet => "Ethernet",
        }
    }

    pub fn key(&self) -> &str {
        self.name()
    }
}

/// Debouncer to prevent rapid flapping and duplicate event spam.
pub struct Debouncer {
    last_events: HashMap<String, (ConnState, f64)>,
    interval_s: f64,
}

impl Debouncer {
    pub fn new(interval_s: f64) -> Self {
        Self {
            last_events: HashMap::new(),
            interval_s,
        }
    }

    /// Check if an event should be emitted based on its key and state.
    pub fn should_emit_event(&mut self, ev: &ConnEvent, now: f64) -> bool {
        self.should_emit(ev.key(), ev.state, now)
    }

    /// Check if event should be emitted. Returns true if state changed or debounce window passed.
    pub fn should_emit(&mut self, key: &str, state: ConnState, now: f64) -> bool {
        if let Some((last_state, last_time)) = self.last_events.get_mut(key) {
            if *last_state == state && (now - *last_time) < self.interval_s {
                return false;
            }
            *last_state = state;
            *last_time = now;
            true
        } else {
            self.last_events.insert(key.to_string(), (state, now));
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cod_maps_to_correct_device_kinds() {
        // Phone
        assert_eq!(DeviceKind::from_cod(0x02, 0), DeviceKind::Phone);

        // Audio: Wearable headset -> Earbuds
        assert_eq!(DeviceKind::from_cod(0x04, 1), DeviceKind::Earbuds);

        // Audio: Loudspeaker -> Speaker
        assert_eq!(DeviceKind::from_cod(0x04, 5), DeviceKind::Speaker);

        // Audio: Headphones -> Headphones
        assert_eq!(DeviceKind::from_cod(0x04, 6), DeviceKind::Headphones);

        // HID: Keyboard
        assert_eq!(DeviceKind::from_cod(0x05, 0x10), DeviceKind::Keyboard);

        // HID: Mouse
        assert_eq!(DeviceKind::from_cod(0x05, 0x20), DeviceKind::Mouse);

        // HID: Gamepad
        assert_eq!(DeviceKind::from_cod(0x05, 0x02), DeviceKind::Gamepad);

        // Unknown major -> Generic
        assert_eq!(DeviceKind::from_cod(0x1F, 0), DeviceKind::Generic);
    }

    #[test]
    fn ble_appearance_maps_correctly() {
        assert_eq!(DeviceKind::from_ble_appearance(0x01, 0), DeviceKind::Phone);
        assert_eq!(DeviceKind::from_ble_appearance(0x0F, 1), DeviceKind::Keyboard);
        assert_eq!(DeviceKind::from_ble_appearance(0x0F, 2), DeviceKind::Mouse);
        assert_eq!(DeviceKind::from_ble_appearance(0x0F, 3), DeviceKind::Gamepad);
        assert_eq!(DeviceKind::from_ble_appearance(0x21, 0), DeviceKind::Speaker);
        assert_eq!(DeviceKind::from_ble_appearance(0x27, 1), DeviceKind::Earbuds);
        assert_eq!(DeviceKind::from_ble_appearance(0x27, 2), DeviceKind::Headphones);
    }

    #[test]
    fn debouncer_suppresses_rapid_duplicates() {
        let mut d = Debouncer::new(1.5);
        let key = "bt:AirBuds";

        // First event is emitted
        assert!(d.should_emit(key, ConnState::Connected, 10.0));

        // Duplicate event within 1.5s is suppressed
        assert!(!d.should_emit(key, ConnState::Connected, 10.5));
        assert!(!d.should_emit(key, ConnState::Connected, 11.2));

        // State change is emitted even within debounce interval
        assert!(d.should_emit(key, ConnState::Disconnected, 11.3));

        // Duplicate disconnected within 1.5s is suppressed
        assert!(!d.should_emit(key, ConnState::Disconnected, 11.4));

        // After debounce interval expires (11.3 + 1.5 = 12.8), same event is allowed
        assert!(d.should_emit(key, ConnState::Disconnected, 13.0));
    }

    #[test]
    fn device_kind_icons_match() {
        assert_eq!(DeviceKind::Headphones.icon(), Icon::Headphones);
        assert_eq!(DeviceKind::Earbuds.icon(), Icon::Earbuds);
        assert_eq!(DeviceKind::Speaker.icon(), Icon::Speaker);
        assert_eq!(DeviceKind::Keyboard.icon(), Icon::Keyboard);
        assert_eq!(DeviceKind::Mouse.icon(), Icon::Mouse);
        assert_eq!(DeviceKind::Gamepad.icon(), Icon::Gamepad);
        assert_eq!(DeviceKind::Phone.icon(), Icon::Phone);
        assert_eq!(DeviceKind::Generic.icon(), Icon::Bluetooth);
    }
}
