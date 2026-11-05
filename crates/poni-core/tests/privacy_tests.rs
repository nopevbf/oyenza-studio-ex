use poni_core::privacy::{PrivacyColor, PrivacyState};

#[test]
fn test_privacy_state_neither_active() {
    let state = PrivacyState { camera: false, microphone: false };
    assert_eq!(state.active_dot(), None);
}

#[test]
fn test_privacy_state_mic_only_shows_orange() {
    let state = PrivacyState { camera: false, microphone: true };
    assert_eq!(state.active_dot(), Some(PrivacyColor::Orange));
}

#[test]
fn test_privacy_state_camera_only_shows_green() {
    let state = PrivacyState { camera: true, microphone: false };
    assert_eq!(state.active_dot(), Some(PrivacyColor::Green));
}

#[test]
fn test_privacy_state_both_active_prioritizes_camera_green() {
    // Sesuai keputusan interview: jika kamera dan mic aktif bersamaan, kamera (Green) diprioritaskan.
    let state = PrivacyState { camera: true, microphone: true };
    assert_eq!(state.active_dot(), Some(PrivacyColor::Green));
}

#[test]
fn test_privacy_color_rgb_values() {
    let green = PrivacyColor::Green;
    let (r, g, b) = green.rgb();
    assert_eq!((r, g, b), (0x4a, 0xde, 0x80)); // #4ADE80

    let orange = PrivacyColor::Orange;
    let (r, g, b) = orange.rgb();
    assert_eq!((r, g, b), (0xfb, 0x92, 0x3c)); // #FB923C
}
