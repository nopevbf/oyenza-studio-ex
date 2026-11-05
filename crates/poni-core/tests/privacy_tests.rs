use poni_core::privacy::{is_usage_active, PrivacyColor, PrivacyState};

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

#[test]
fn test_is_usage_active_conditions() {
    // 1. Kasus standar: stop_time == 0 dan start_time > 0
    assert!(is_usage_active(134363312738940877, 0), "Sesi aktif dengan stop_time 0 harus terdeteksi");

    // 2. Kasus webcam eksternal / Windows update: start_time > stop_time > 0 (sesi baru tanpa reset 0)
    assert!(is_usage_active(134363312848293138, 134363312848293137), "Sesi aktif dengan start_time > stop_time harus terdeteksi");

    // 3. Kasus inaktif: stop_time >= start_time
    assert!(!is_usage_active(134363312738940877, 134363312848293137), "Sesi selesai dengan stop_time > start_time harus inaktif");
    assert!(!is_usage_active(134363312848293137, 134363312848293137), "stop_time == start_time harus inaktif");

    // 4. Kasus key kosong: start_time == 0 dan stop_time == 0
    assert!(!is_usage_active(0, 0), "start_time 0 dan stop_time 0 harus inaktif");
}
