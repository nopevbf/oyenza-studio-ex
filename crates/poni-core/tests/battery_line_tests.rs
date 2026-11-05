use poni_core::battery_line::{battery_bar_color, battery_bar_width, should_show_battery_line};

#[test]
fn test_should_show_battery_line() {
    assert!(!should_show_battery_line(false, true), "Desktop tanpa baterai tidak boleh menampilkan garis baterai");
    assert!(!should_show_battery_line(true, false), "Jika toggle setting dimatikan, tidak boleh ditampilkan");
    assert!(!should_show_battery_line(false, false), "Tanpa baterai dan setting off -> tidak ditampilkan");
    assert!(should_show_battery_line(true, true), "Laptop dengan baterai dan setting on -> ditampilkan");
}

#[test]
fn test_battery_bar_width_calculation() {
    let total_width = 160.0;
    assert_eq!(battery_bar_width(100, total_width), 160.0);
    assert_eq!(battery_bar_width(50, total_width), 80.0);
    assert_eq!(battery_bar_width(0, total_width), 0.0);
}

#[test]
fn test_battery_bar_color_thresholds() {
    // > 50%: Hijau (#22C55E)
    let (r, g, b) = battery_bar_color(100, false);
    assert_eq!((r, g, b), (0x22, 0xc5, 0x5e));

    let (r, g, b) = battery_bar_color(51, false);
    assert_eq!((r, g, b), (0x22, 0xc5, 0x5e));

    // 16% - 50%: Kuning (#EAB308)
    let (r, g, b) = battery_bar_color(50, false);
    assert_eq!((r, g, b), (0xea, 0xb3, 0x08));

    let (r, g, b) = battery_bar_color(16, false);
    assert_eq!((r, g, b), (0xea, 0xb3, 0x08));

    // <= 15%: Merah (#EF4444)
    let (r, g, b) = battery_bar_color(15, false);
    assert_eq!((r, g, b), (0xef, 0x44, 0x44));

    let (r, g, b) = battery_bar_color(5, false);
    assert_eq!((r, g, b), (0xef, 0x44, 0x44));
}
