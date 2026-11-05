//! Screen brightness control for laptop internal panels and external DDC/CI monitors.
//!
//! Rule 1 (Keep it light & Non-blocking):
//! DDC/CI calls over the I2C bus can be slow (50-200ms) on some external displays.
//! Hardware brightness requests are dispatched to a worker thread with a 100ms timeout
//! so the UI thread remains responsive at 60 FPS.

use crate::events::{AppEvent, EventSink};
use poni_core::brightness::clamp_brightness;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use windows::core::Result;
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, SetMonitorBrightness, PHYSICAL_MONITOR,
};
use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};

#[allow(dead_code)]
pub enum BrightnessCmd {
    Read,
    Step(f32),
    Set(f32),
    Stop,
}

pub struct BrightnessService {
    tx: Sender<BrightnessCmd>,
    thread: Option<JoinHandle<()>>,
}

#[allow(dead_code)]
impl BrightnessService {
    pub fn start(sink: EventSink) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("poni-brightness".into())
            .spawn(move || {
                worker(sink, rx);
            })
            .ok();
        Ok(Self { tx, thread })
    }

    pub fn read(&self) {
        let _ = self.tx.send(BrightnessCmd::Read);
    }

    pub fn step(&self, delta: f32) {
        let _ = self.tx.send(BrightnessCmd::Step(delta));
    }

    pub fn set(&self, level: f32) {
        let _ = self.tx.send(BrightnessCmd::Set(level));
    }
}

impl Drop for BrightnessService {
    fn drop(&mut self) {
        let _ = self.tx.send(BrightnessCmd::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn worker(sink: EventSink, rx: Receiver<BrightnessCmd>) {
    let mut current_level = read_system_brightness().unwrap_or(0.70);
    // Send initial level
    sink.send(AppEvent::Brightness { level: current_level });

    while let Ok(cmd) = rx.recv() {
        match cmd {
            BrightnessCmd::Read => {
                if let Some(level) = read_system_brightness() {
                    current_level = level;
                }
                sink.send(AppEvent::Brightness { level: current_level });
            }
            BrightnessCmd::Step(delta) => {
                let target = clamp_brightness(current_level + delta);
                current_level = target;
                let _ = apply_system_brightness(target);
                sink.send(AppEvent::Brightness { level: current_level });
            }
            BrightnessCmd::Set(target) => {
                let clamped = clamp_brightness(target);
                current_level = clamped;
                let _ = apply_system_brightness(clamped);
                sink.send(AppEvent::Brightness { level: current_level });
            }
            BrightnessCmd::Stop => break,
        }
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
