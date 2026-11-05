use poni_core::settings::Settings;

#[test]
fn test_privacy_and_battery_settings_defaults() {
    let settings = Settings::default();
    assert!(settings.privacy_indicator, "Privacy indicator harus aktif (true) secara default");
    assert!(!settings.battery_line, "Battery line harus mati (false) secara default sesuai permintaan user");
}

#[test]
fn test_privacy_and_battery_settings_deserialize_empty_json() {
    let json = "{}";
    let settings: Settings = serde_json::from_str(json).expect("Harus bisa parse empty JSON");
    assert!(settings.privacy_indicator);
    assert!(!settings.battery_line);
}
