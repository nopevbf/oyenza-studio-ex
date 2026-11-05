use poni_core::battery_line::{battery_bar_color, battery_bar_width, battery_contour_segments, should_show_battery_line};
use poni_core::geom::Rect;
use poni_core::svgpath::Seg;

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

#[test]
fn test_battery_contour_segments_zero_percent_empty() {
    let body = Rect::new(100.0, 0.0, 160.0, 32.0);
    let segs = battery_contour_segments(body, 14.0, 0);
    assert!(segs.is_empty(), "0% baterai tidak menghasilkan segmen garis");
}

#[test]
fn test_battery_contour_segments_partial_curve_left() {
    let body = Rect::new(100.0, 0.0, 160.0, 32.0);
    // Radius 14.0 -> keliling kurva kiri = pi/2 * 14 = ~22.0 DIP
    // Total panjang = 22.0 + (160 - 28) + 22.0 = 176.0 DIP.
    // 5% dari 176 = 8.8 DIP (masih di dalam kurva kiri)
    let segs = battery_contour_segments(body, 14.0, 5);
    assert_eq!(segs.len(), 2, "Harus MoveTo dan ArcTo parsial");
    match segs[0] {
        Seg::MoveTo(x, y) => {
            assert_eq!(x, 100.0);
            assert_eq!(y, 32.0 - 14.0);
        }
        _ => panic!("Segmen pertama harus MoveTo"),
    }
    match segs[1] {
        Seg::ArcTo(rx, ry, _, false, false, x, y) => {
            assert_eq!(rx, 14.0);
            assert_eq!(ry, 14.0);
            assert!(x > 100.0 && x < 100.0 + 14.0);
            assert!(y > 18.0 && y <= 32.0);
        }
        _ => panic!("Segmen kedua harus ArcTo parsial"),
    }
}

#[test]
fn test_battery_contour_segments_middle_flat() {
    let body = Rect::new(100.0, 0.0, 160.0, 32.0);
    // 50% dari 176 = 88 DIP (melewati kurva kiri 22.0 DIP, sepanjang garis tengah)
    let segs = battery_contour_segments(body, 14.0, 50);
    assert_eq!(segs.len(), 3, "Harus MoveTo, ArcTo penuh sudut kiri, dan LineTo tengah");
    assert_eq!(segs[0], Seg::MoveTo(100.0, 18.0));
    assert_eq!(segs[1], Seg::ArcTo(14.0, 14.0, 0.0, false, false, 114.0, 32.0));
    match segs[2] {
        Seg::LineTo(x, y) => {
            assert_eq!(y, 32.0);
            assert!(x > 114.0 && x < 260.0 - 14.0);
        }
        _ => panic!("Segmen ketiga harus LineTo"),
    }
}

#[test]
fn test_battery_contour_segments_full_100_percent() {
    let body = Rect::new(100.0, 0.0, 160.0, 32.0);
    let segs = battery_contour_segments(body, 14.0, 100);
    assert_eq!(segs.len(), 4, "100% baterai: MoveTo, ArcTo kiri, LineTo tengah, ArcTo kanan penuh");
    assert_eq!(segs[0], Seg::MoveTo(100.0, 18.0));
    assert_eq!(segs[1], Seg::ArcTo(14.0, 14.0, 0.0, false, false, 114.0, 32.0));
    assert_eq!(segs[2], Seg::LineTo(246.0, 32.0));
    assert_eq!(segs[3], Seg::ArcTo(14.0, 14.0, 0.0, false, false, 260.0, 18.0));
}

#[test]
fn test_battery_contour_segments_zero_radius_fallback() {
    let body = Rect::new(100.0, 0.0, 160.0, 32.0);
    let segs = battery_contour_segments(body, 0.0, 50);
    assert_eq!(segs.len(), 2, "Radius 0 harus menghasilkan garis lurus datar (MoveTo + LineTo)");
    assert_eq!(segs[0], Seg::MoveTo(100.0, 32.0));
    assert_eq!(segs[1], Seg::LineTo(180.0, 32.0));
}
