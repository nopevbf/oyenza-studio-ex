use poni_core::eyes::{EyeMood, IdleTracker, WAVE_DURATION, WAVE_IDLE_THRESHOLD};

#[test]
fn test_eye_mood_excludes_love() {
    let moods = EyeMood::ALL;
    assert!(!moods.iter().any(|m| m.as_str() == "love"));
    assert!(moods.contains(&EyeMood::Blink));
    assert!(moods.contains(&EyeMood::Flutter));
    assert!(moods.contains(&EyeMood::Squint));
    assert!(moods.contains(&EyeMood::Smile));
    assert!(moods.contains(&EyeMood::Sleep));
    assert!(moods.contains(&EyeMood::Sly));
    assert!(moods.contains(&EyeMood::SlyLeft));
    assert!(moods.contains(&EyeMood::Read));
    assert!(moods.contains(&EyeMood::Think));
}

#[test]
fn test_idle_tracker_initial_state() {
    let tracker = IdleTracker::new();
    assert_eq!(tracker.idle_secs(), 0.0);
    assert!(!tracker.is_waving());
}

#[test]
fn test_idle_tracker_before_threshold() {
    let mut tracker = IdleTracker::new();
    tracker.tick(25.0, false);
    assert_eq!(tracker.idle_secs(), 25.0);
    assert!(!tracker.is_waving(), "Belum 30 detik idle, tidak boleh waving");
}

#[test]
fn test_idle_tracker_triggers_wave_at_threshold() {
    let mut tracker = IdleTracker::new();
    tracker.tick(WAVE_IDLE_THRESHOLD, false);
    assert!(tracker.is_waving(), "Tepat 30 detik idle harus memicu waving");

    // Tetap waving selama rentang durasi wave
    tracker.tick(1.0, false);
    assert!(tracker.is_waving(), "Masih dalam durasi waving");
}

#[test]
fn test_idle_tracker_stops_wave_after_duration() {
    let mut tracker = IdleTracker::new();
    tracker.tick(WAVE_IDLE_THRESHOLD + WAVE_DURATION + 0.1, false);
    assert!(!tracker.is_waving(), "Setelah durasi wave lewat, wave harus berhenti (istirahat hemat CPU)");
}

#[test]
fn test_idle_tracker_mouse_movement_cancels_wave_immediately() {
    let mut tracker = IdleTracker::new();
    tracker.tick(WAVE_IDLE_THRESHOLD + 1.0, false);
    assert!(tracker.is_waving());

    // User bergerak kursor
    tracker.tick(0.016, true);
    assert!(!tracker.is_waving(), "Gerakan kursor harus langsung menghentikan waving");
    assert_eq!(tracker.idle_secs(), 0.0, "Timer idle harus di-reset ke 0");
}


#[test]
fn test_glove_svg_path_parses_cleanly() {
    let glove_d = "M26 78L18 61Q4 51 8 43Q10 35 17 41L22 46L13 17Q12 8 19 7Q26 6 28 16L33 34L30 10Q30 2 37 2Q44 2 44 10L45 33L48 13Q50 5 57 8Q63 10 61 18L58 39L65 25Q69 18 74 23Q79 28 74 35L66 55Q66 67 56 78Z";
    let cuff_d = "M24 76H57V86H24Z";
    assert!(poni_core::svgpath::parse(glove_d).is_ok());
    assert!(poni_core::svgpath::parse(cuff_d).is_ok());
}
