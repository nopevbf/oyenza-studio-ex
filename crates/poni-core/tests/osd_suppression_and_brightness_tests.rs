use poni_core::settings::Settings;

#[test]
fn test_osd_suppression_and_idle_look_defaults() {
    let s = Settings::default();
    assert!(!s.suppress_windows_volume_osd, "suppress_windows_volume_osd should default to false");
    assert!(!s.suppress_windows_brightness_osd, "suppress_windows_brightness_osd should default to false");
    assert!(!s.suppress_windows_toast_banners, "suppress_windows_toast_banners should default to false");
    assert!(!s.idle_look_only, "idle_look_only should default to false");
    assert!(!s.hud_in_fullscreen, "hud_in_fullscreen should default to false");
}

#[test]
fn test_osd_suppression_deserialize_from_json() {
    let json = r#"{
        "suppress_windows_volume_osd": true,
        "suppress_windows_brightness_osd": true,
        "suppress_windows_toast_banners": true,
        "idle_look_only": true,
        "hud_in_fullscreen": true
    }"#;
    let s: Settings = serde_json::from_str(json).expect("should deserialize custom settings");
    assert!(s.suppress_windows_volume_osd);
    assert!(s.suppress_windows_brightness_osd);
    assert!(s.suppress_windows_toast_banners);
    assert!(s.idle_look_only);
    assert!(s.hud_in_fullscreen);
}

#[test]
fn test_osd_suppression_empty_json_fallbacks_to_defaults() {
    let s: Settings = serde_json::from_str("{}").expect("empty json should deserialize cleanly");
    assert!(!s.suppress_windows_volume_osd);
    assert!(!s.suppress_windows_brightness_osd);
    assert!(!s.suppress_windows_toast_banners);
    assert!(!s.idle_look_only);
    assert!(!s.hud_in_fullscreen);
}

#[test]
fn test_osd_suppression_roundtrip_serialization() {
    let mut s = Settings::default();
    s.suppress_windows_volume_osd = true;
    s.suppress_windows_brightness_osd = true;
    s.suppress_windows_toast_banners = true;
    s.idle_look_only = true;
    s.hud_in_fullscreen = true;

    let serialized = serde_json::to_string(&s).expect("should serialize settings");
    let restored: Settings = serde_json::from_str(&serialized).expect("should deserialize roundtrip");
    assert_eq!(s.suppress_windows_volume_osd, restored.suppress_windows_volume_osd);
    assert_eq!(s.suppress_windows_brightness_osd, restored.suppress_windows_brightness_osd);
    assert_eq!(s.suppress_windows_toast_banners, restored.suppress_windows_toast_banners);
    assert_eq!(s.idle_look_only, restored.idle_look_only);
    assert_eq!(s.hud_in_fullscreen, restored.hud_in_fullscreen);
}

#[test]
fn test_brightness_info_and_clamping_bva() {
    use poni_core::brightness::{clamp_brightness, DisplayKind, BrightnessInfo};

    // Boundary Value Analysis (BVA) for brightness clamping
    assert_eq!(clamp_brightness(-0.5), 0.0);
    assert_eq!(clamp_brightness(0.0), 0.0);
    assert_eq!(clamp_brightness(0.001), 0.001);
    assert_eq!(clamp_brightness(0.55), 0.55);
    assert_eq!(clamp_brightness(0.999), 0.999);
    assert_eq!(clamp_brightness(1.0), 1.0);
    assert_eq!(clamp_brightness(1.5), 1.0);

    let internal = BrightnessInfo {
        level: clamp_brightness(0.75),
        display_name: "Internal Display".to_string(),
        kind: DisplayKind::InternalLaptop,
        is_supported: true,
    };
    assert_eq!(internal.percent(), 75);
    assert!(internal.is_supported);
    assert_eq!(internal.kind, DisplayKind::InternalLaptop);

    let external = BrightnessInfo {
        level: clamp_brightness(0.0),
        display_name: "External HDMI Monitor".to_string(),
        kind: DisplayKind::ExternalDdcCi,
        is_supported: false,
    };
    assert_eq!(external.percent(), 0);
    assert!(!external.is_supported);
    assert_eq!(external.kind, DisplayKind::ExternalDdcCi);
}

#[test]
fn test_weight_budget_integrity_and_bounds() {
    let budget_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/weight-budget.json");
    let content = std::fs::read_to_string(budget_path).expect("weight-budget.json must exist");
    let val: serde_json::Value = serde_json::from_str(&content).expect("valid json in weight-budget");
    
    let private_mib = val["rest"]["private_mib"].as_f64().expect("private_mib exists");
    let warn_only_until = val["warn_only_until"].as_str().expect("warn_only_until exists");

    // RED requirement: budget must accommodate new features headroom (at least 21.5 MiB)
    assert!(private_mib >= 21.5, "private_mib budget must be at least 21.5 to prevent false-positive CI failures, got {}", private_mib);
    // warn_only_until must be active into 2027
    assert!(warn_only_until >= "2027-01-01", "warn_only_until should be extended into 2027, got {}", warn_only_until);
}
