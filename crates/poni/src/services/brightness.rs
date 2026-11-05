//! Screen brightness control for laptop internal panels and external DDC/CI monitors.
//!
//! Rule 1 (Keep it light & Non-blocking):
//! DDC/CI calls over the I2C bus can be slow (50-200ms) on some external displays.
//! Hardware brightness requests are dispatched on-demand to a transient worker thread
//! so the UI thread remains responsive at 60 FPS, and NO thread runs at rest while idle.

use crate::events::{AppEvent, EventSink};
use poni_core::brightness::clamp_brightness;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use windows::core::Result;
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, SetMonitorBrightness, PHYSICAL_MONITOR,
};
use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};

pub struct BrightnessService {
    sink: EventSink,
    cached_level: Arc<AtomicU32>,
}

#[allow(dead_code)]
impl BrightnessService {
    pub fn start(sink: EventSink) -> Result<Self> {
        let cached_level = Arc::new(AtomicU32::new((0.70_f32).to_bits()));
        Ok(Self { sink, cached_level })
    }

    pub fn read(&self) {
        let sink = self.sink.clone();
        let cached = Arc::clone(&self.cached_level);
        let _ = std::thread::Builder::new()
            .name("poni-brightness".into())
            .spawn(move || {
                let level = read_system_brightness().unwrap_or_else(|| f32::from_bits(cached.load(Ordering::Relaxed)));
                cached.store(level.to_bits(), Ordering::Relaxed);
                sink.send(AppEvent::Brightness { level });
            });
    }

    pub fn step(&self, delta: f32) {
        let sink = self.sink.clone();
        let cached = Arc::clone(&self.cached_level);
        let _ = std::thread::Builder::new()
            .name("poni-brightness".into())
            .spawn(move || {
                let curr = f32::from_bits(cached.load(Ordering::Relaxed));
                let target = clamp_brightness(curr + delta);
                cached.store(target.to_bits(), Ordering::Relaxed);
                let _ = apply_system_brightness(target);
                sink.send(AppEvent::Brightness { level: target });
            });
    }

    pub fn set(&self, level: f32) {
        let sink = self.sink.clone();
        let cached = Arc::clone(&self.cached_level);
        let _ = std::thread::Builder::new()
            .name("poni-brightness".into())
            .spawn(move || {
                let target = clamp_brightness(level);
                cached.store(target.to_bits(), Ordering::Relaxed);
                let _ = apply_system_brightness(target);
                sink.send(AppEvent::Brightness { level: target });
            });
    }
}

/// Reads the brightness level (0.0 ..= 1.0) from the active physical monitor.
pub fn read_system_brightness() -> Option<f32> {
    let monitors = collect_physical_monitors();
    for mon in &monitors {
        let mut min = 0u32;
        let mut cur = 0u32;
        let mut max = 0u32;
        unsafe {
            if GetMonitorBrightness(mon.hPhysicalMonitor, &mut min, &mut cur, &mut max) != 0 {
                if max > min {
                    let level = (cur - min) as f32 / (max - min) as f32;
                    destroy_physical_monitors(monitors);
                    return Some(clamp_brightness(level));
                }
            }
        }
    }
    destroy_physical_monitors(monitors);
    None
}

/// Sets the brightness level (0.0 ..= 1.0) on physical monitors.
pub fn apply_system_brightness(level: f32) -> bool {
    let monitors = collect_physical_monitors();
    let mut success = false;
    for mon in &monitors {
        let mut min = 0u32;
        let mut cur = 0u32;
        let mut max = 0u32;
        unsafe {
            if GetMonitorBrightness(mon.hPhysicalMonitor, &mut min, &mut cur, &mut max) != 0 {
                let target_val = min + ((max - min) as f32 * clamp_brightness(level)).round() as u32;
                if SetMonitorBrightness(mon.hPhysicalMonitor, target_val) != 0 {
                    success = true;
                }
            }
        }
    }
    destroy_physical_monitors(monitors);
    success
}

/// Applies a relative brightness step.
pub fn apply_system_brightness_delta(delta: f32) -> bool {
    let cur = read_system_brightness().unwrap_or(0.70);
    apply_system_brightness(clamp_brightness(cur + delta))
}

struct MonitorCollector {
    monitors: Vec<PHYSICAL_MONITOR>,
}

unsafe extern "system" fn enum_monitor_proc(
    hmonitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let collector = &mut *(lparam.0 as *mut MonitorCollector);
    let mut count = 0u32;
    if GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count).is_ok() && count > 0 {
        let mut phys: Vec<PHYSICAL_MONITOR> = vec![PHYSICAL_MONITOR::default(); count as usize];
        if GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut phys).is_ok() {
            collector.monitors.extend(phys);
        }
    }
    BOOL(1)
}

fn collect_physical_monitors() -> Vec<PHYSICAL_MONITOR> {
    let mut collector = MonitorCollector { monitors: Vec::new() };
    unsafe {
        let _ = EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(enum_monitor_proc),
            LPARAM(&mut collector as *mut MonitorCollector as isize),
        );
    }
    collector.monitors
}

fn destroy_physical_monitors(monitors: Vec<PHYSICAL_MONITOR>) {
    if !monitors.is_empty() {
        unsafe {
            let _ = DestroyPhysicalMonitors(&monitors);
        }
    }
}
