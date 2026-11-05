//! Privacy service: detects Windows system-wide camera and microphone activity
//! using event-driven Windows Media Foundation Sensor Activity Monitor (IMFSensorActivityMonitor),
//! Core Audio WASAPI capture session notifications (IAudioSessionNotification / IAudioSessionEvents),
//! and fallback registry change notifications (RegNotifyChangeKeyValue).
//! Zero polling at rest: blocks on Win32 sync primitives until Windows signals a sensor status change.

use crate::events::{AppEvent, EventSink};
use poni_core::privacy::PrivacyState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use windows::core::{implement, w, Interface, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::MediaFoundation::{
    IMFSensorActivitiesReport, IMFSensorActivitiesReportCallback,
    IMFSensorActivitiesReportCallback_Impl, IMFSensorActivityMonitor,
    MFCreateSensorActivityMonitor, MFShutdown, MFStartup, MFSTARTUP_NOSOCKET, MF_SDK_VERSION,
};
use windows::Win32::System::Com::*;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegNotifyChangeKeyValue, RegOpenKeyExW, RegQueryValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, INFINITE};

const CAM_PATH: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\webcam");
const MIC_PATH: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone");

pub struct PrivacyService {
    stop: Arc<AtomicBool>,
    stop_event: HANDLE,
    worker: Option<JoinHandle<()>>,
}

// Media Foundation camera sensor streaming callback
#[implement(IMFSensorActivitiesReportCallback)]
struct SensorActivityCallback {
    streaming: Arc<AtomicBool>,
    notify_event: usize,
}

impl IMFSensorActivitiesReportCallback_Impl for SensorActivityCallback_Impl {
    fn OnActivitiesReport(&self, report: Option<&IMFSensorActivitiesReport>) -> windows::core::Result<()> {
        let is_any_streaming = is_report_streaming(report);
        let old = self.streaming.swap(is_any_streaming, Ordering::SeqCst);
        if old != is_any_streaming {
            crate::pinfo!("privacy: media foundation camera streaming changed -> {is_any_streaming}");
            unsafe {
                let _ = SetEvent(HANDLE(self.notify_event as *mut _));
            }
        }
        Ok(())
    }
}

pub fn is_report_streaming(report: Option<&IMFSensorActivitiesReport>) -> bool {
    let rep = match report {
        Some(r) => r,
        None => return false,
    };
    unsafe {
        let count = match rep.GetCount() {
            Ok(c) => c,
            Err(_) => return false,
        };
        for i in 0..count {
            if let Ok(item) = rep.GetActivityReport(i) {
                let proc_count = item.GetProcessCount().unwrap_or(0);
                for p in 0..proc_count {
                    if let Ok(p_act) = item.GetProcessActivity(p) {
                        if p_act.GetStreamingState().map(|b| b.as_bool()).unwrap_or(false) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

// Core Audio WASAPI microphone session events
#[implement(IAudioSessionEvents)]
struct MicSessionEvents {
    notify_event: usize,
}

#[allow(non_snake_case)]
impl IAudioSessionEvents_Impl for MicSessionEvents_Impl {
    fn OnDisplayNameChanged(&self, _: &PCWSTR, _: *const windows::core::GUID) -> windows::core::Result<()> { Ok(()) }
    fn OnIconPathChanged(&self, _: &PCWSTR, _: *const windows::core::GUID) -> windows::core::Result<()> { Ok(()) }
    fn OnSimpleVolumeChanged(&self, _: f32, _: BOOL, _: *const windows::core::GUID) -> windows::core::Result<()> { Ok(()) }
    fn OnChannelVolumeChanged(&self, _: u32, _: *const f32, _: u32, _: *const windows::core::GUID) -> windows::core::Result<()> { Ok(()) }
    fn OnGroupingParamChanged(&self, _: *const windows::core::GUID, _: *const windows::core::GUID) -> windows::core::Result<()> { Ok(()) }
    fn OnStateChanged(&self, _: AudioSessionState) -> windows::core::Result<()> {
        unsafe {
            let _ = SetEvent(HANDLE(self.notify_event as *mut _));
        }
        Ok(())
    }
    fn OnSessionDisconnected(&self, _: AudioSessionDisconnectReason) -> windows::core::Result<()> {
        unsafe {
            let _ = SetEvent(HANDLE(self.notify_event as *mut _));
        }
        Ok(())
    }
}

// Core Audio WASAPI microphone session creation notification
#[implement(IAudioSessionNotification)]
struct MicSessionNotification {
    notify_event: usize,
    retained_events: Arc<std::sync::Mutex<Vec<(IAudioSessionControl, IAudioSessionEvents)>>>,
}

#[allow(non_snake_case)]
impl IAudioSessionNotification_Impl for MicSessionNotification_Impl {
    fn OnSessionCreated(&self, newsession: Option<&IAudioSessionControl>) -> windows::core::Result<()> {
        if let Some(session) = newsession {
            let events: IAudioSessionEvents = MicSessionEvents {
                notify_event: self.notify_event,
            }.into();
            unsafe {
                let _ = session.RegisterAudioSessionNotification(&events);
            }
            if let Ok(mut lock) = self.retained_events.lock() {
                lock.push((session.clone(), events));
            }
            unsafe {
                let _ = SetEvent(HANDLE(self.notify_event as *mut _));
            }
        }
        Ok(())
    }
}

pub fn is_any_mic_active() -> bool {
    unsafe {
        let enumerator: IMMDeviceEnumerator = match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
            Ok(e) => e,
            Err(_) => return false,
        };
        let collection = match enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let dev_count = collection.GetCount().unwrap_or(0);
        for d in 0..dev_count {
            if let Ok(device) = collection.Item(d) {
                if let Ok(mgr) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) {
                    if let Ok(session_enum) = mgr.GetSessionEnumerator() {
                        let sess_count = session_enum.GetCount().unwrap_or(0);
                        for s in 0..sess_count {
                            if let Ok(session) = session_enum.GetSession(s) {
                                if let Ok(ctrl2) = session.cast::<IAudioSessionControl2>() {
                                    if let Ok(state) = ctrl2.GetState() {
                                        if state == AudioSessionStateActive {
                                            return true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

impl PrivacyService {
    pub fn start(sink: EventSink) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_event = unsafe { CreateEventW(None, true, false, None) }
            .map_err(|e| format!("CreateEventW failed: {e}"))?;

        let stop_clone = stop.clone();
        let stop_h = stop_event.0 as usize;

        let worker = std::thread::Builder::new()
            .name("privacy-watcher".into())
            .spawn(move || {
                run_watcher(sink, stop_clone, HANDLE(stop_h as *mut _));
            })
            .map_err(|e| format!("spawn privacy watcher failed: {e}"))?;

        Ok(Self {
            stop,
            stop_event,
            worker: Some(worker),
        })
    }
}

impl Drop for PrivacyService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        unsafe {
            let _ = SetEvent(self.stop_event);
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        unsafe {
            let _ = CloseHandle(self.stop_event);
        }
    }
}

fn run_watcher(sink: EventSink, stop: Arc<AtomicBool>, stop_event: HANDLE) {
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let mut cam_hkey = HKEY::default();
    let mut mic_hkey = HKEY::default();

    let cam_ok = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, CAM_PATH, 0, KEY_READ | KEY_NOTIFY, &mut cam_hkey) }.is_ok();
    let mic_ok = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, MIC_PATH, 0, KEY_READ | KEY_NOTIFY, &mut mic_hkey) }.is_ok();

    let cam_event = unsafe { CreateEventW(None, false, false, None) }.unwrap_or_default();
    let mic_event = unsafe { CreateEventW(None, false, false, None) }.unwrap_or_default();
    let mf_event = unsafe { CreateEventW(None, false, false, None) }.unwrap_or_default();
    let mic_wasapi_event = unsafe { CreateEventW(None, false, false, None) }.unwrap_or_default();

    let cam_streaming = Arc::new(AtomicBool::new(false));

    // 1. Try starting Media Foundation Sensor Activity Monitor for real-time camera streaming events
    let mut mf_started = false;
    let mut mf_monitor: Option<IMFSensorActivityMonitor> = None;

    if unsafe { MFStartup(MF_SDK_VERSION, MFSTARTUP_NOSOCKET) }.is_ok() {
        mf_started = true;
        let cb_impl = SensorActivityCallback {
            streaming: cam_streaming.clone(),
            notify_event: mf_event.0 as usize,
        };
        let cb: IMFSensorActivitiesReportCallback = cb_impl.into();
        match unsafe { MFCreateSensorActivityMonitor(&cb) } {
            Ok(mon) => {
                if unsafe { mon.Start() }.is_ok() {
                    crate::pinfo!("privacy: media foundation camera activity monitor active");
                    mf_monitor = Some(mon);
                } else {
                    crate::pwarn!("privacy: failed to start media foundation activity monitor");
                }
            }
            Err(e) => {
                crate::pwarn!("privacy: MFCreateSensorActivityMonitor not available: {e}");
            }
        }
    }

    // 2. Try registering Core Audio WASAPI listeners on all active microphone capture endpoints
    let mut wasapi_managers: Vec<(IAudioSessionManager2, IAudioSessionNotification)> = Vec::new();
    let retained_events = Arc::new(std::sync::Mutex::new(Vec::<(IAudioSessionControl, IAudioSessionEvents)>::new()));

    unsafe {
        if let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) {
            if let Ok(collection) = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) {
                let dev_count = collection.GetCount().unwrap_or(0);
                for d in 0..dev_count {
                    if let Ok(device) = collection.Item(d) {
                        if let Ok(mgr) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) {
                            let notifier: IAudioSessionNotification = MicSessionNotification {
                                notify_event: mic_wasapi_event.0 as usize,
                                retained_events: retained_events.clone(),
                            }.into();
                            if mgr.RegisterSessionNotification(&notifier).is_ok() {
                                // Register on existing sessions
                                if let Ok(session_enum) = mgr.GetSessionEnumerator() {
                                    let sess_count = session_enum.GetCount().unwrap_or(0);
                                    for s in 0..sess_count {
                                        if let Ok(session) = session_enum.GetSession(s) {
                                            let events: IAudioSessionEvents = MicSessionEvents {
                                                notify_event: mic_wasapi_event.0 as usize,
                                            }.into();
                                            let _ = session.RegisterAudioSessionNotification(&events);
                                            if let Ok(mut lock) = retained_events.lock() {
                                                lock.push((session, events));
                                            }
                                        }
                                    }
                                }
                                wasapi_managers.push((mgr, notifier));
                            }
                        }
                    }
                }
            }
        }
    }

    let wasapi_ok = !wasapi_managers.is_empty();
    crate::pinfo!("privacy: watcher started (cam_ok={cam_ok}, mic_ok={mic_ok}, mf={}, wasapi_mic={})", mf_monitor.is_some(), wasapi_ok);

    // Initial sensor state evaluation
    let mut last_state = evaluate_privacy(cam_hkey, mic_hkey, cam_streaming.load(Ordering::SeqCst));
    crate::pinfo!("privacy: initial state camera={}, mic={}", last_state.camera, last_state.microphone);
    sink.send(AppEvent::Privacy(last_state));

    while !stop.load(Ordering::SeqCst) {
        // Arm notifications (watching the whole subtree so subkey app changes and key additions trigger it)
        let filter = REG_NOTIFY_CHANGE_LAST_SET | REG_NOTIFY_CHANGE_NAME;
        if cam_ok {
            unsafe {
                let _ = RegNotifyChangeKeyValue(cam_hkey, true, filter, cam_event, true);
            }
        }
        if mic_ok {
            unsafe {
                let _ = RegNotifyChangeKeyValue(mic_hkey, true, filter, mic_event, true);
            }
        }

        // AGENTS.md rule: Zero polling at rest (INFINITE). When a sensor is active, poll every 1500 ms
        // so that if a process closes its stream or terminates without firing an event, the privacy
        // dot turns off cleanly within 1.5 seconds, then immediately resumes INFINITE sleep.
        let wait_timeout_ms = if last_state.camera || last_state.microphone {
            1500
        } else {
            INFINITE
        };

        let handles = [stop_event, cam_event, mic_event, mf_event, mic_wasapi_event];
        let wait_res = unsafe { WaitForMultipleObjects(&handles, false, wait_timeout_ms) };

        if stop.load(Ordering::SeqCst) || wait_res == WAIT_OBJECT_0 {
            break;
        }

        // State changed or periodic check while sensor active
        let new_state = evaluate_privacy(cam_hkey, mic_hkey, cam_streaming.load(Ordering::SeqCst));
        if new_state != last_state {
            crate::pinfo!("privacy: sensor status changed -> camera={}, mic={}", new_state.camera, new_state.microphone);
            last_state = new_state;
            sink.send(AppEvent::Privacy(new_state));
        }
    }

    // Teardown WASAPI listeners
    for (mgr, notifier) in wasapi_managers {
        unsafe {
            let _ = mgr.UnregisterSessionNotification(&notifier);
        }
    }
    if let Ok(mut lock) = retained_events.lock() {
        lock.clear();
    }

    // Teardown Media Foundation
    if let Some(mon) = mf_monitor {
        unsafe {
            let _ = mon.Stop();
        }
    }
    if mf_started {
        unsafe {
            let _ = MFShutdown();
        }
    }

    unsafe {
        if cam_ok {
            let _ = RegCloseKey(cam_hkey);
        }
        if mic_ok {
            let _ = RegCloseKey(mic_hkey);
        }
        if !cam_event.is_invalid() {
            let _ = CloseHandle(cam_event);
        }
        if !mic_event.is_invalid() {
            let _ = CloseHandle(mic_event);
        }
        if !mf_event.is_invalid() {
            let _ = CloseHandle(mf_event);
        }
        if !mic_wasapi_event.is_invalid() {
            let _ = CloseHandle(mic_wasapi_event);
        }
        CoUninitialize();
    }
}

fn evaluate_privacy(cam_hkey: HKEY, mic_hkey: HKEY, cam_streaming: bool) -> PrivacyState {
    let reg_camera = if !cam_hkey.is_invalid() {
        is_key_tree_active(cam_hkey)
    } else {
        false
    };
    let camera = cam_streaming || reg_camera;

    let wasapi_mic = is_any_mic_active();
    let reg_microphone = if !mic_hkey.is_invalid() {
        is_key_tree_active(mic_hkey)
    } else {
        false
    };
    let microphone = wasapi_mic || reg_microphone;

    PrivacyState::new(camera, microphone)
}

fn is_key_tree_active(root: HKEY) -> bool {
    let mut subkey_idx = 0;
    let mut name_buf = [0u16; 260];

    loop {
        let mut name_len = name_buf.len() as u32;
        let r = unsafe {
            RegEnumKeyExW(
                root,
                subkey_idx,
                windows::core::PWSTR(name_buf.as_mut_ptr()),
                &mut name_len,
                None,
                windows::core::PWSTR::null(),
                None,
                None,
            )
        };
        if r.is_err() {
            break;
        }
        subkey_idx += 1;

        let subkey_name = &name_buf[..name_len as usize];
        let is_non_packaged = is_utf16_equal(subkey_name, "NonPackaged");

        let mut child_hkey = HKEY::default();
        if unsafe {
            RegOpenKeyExW(
                root,
                PCWSTR(name_buf.as_ptr()),
                0,
                KEY_READ,
                &mut child_hkey,
            )
        }.is_ok() {
            if is_non_packaged {
                // Scan all apps inside NonPackaged
                let active = scan_app_keys(child_hkey);
                unsafe { let _ = RegCloseKey(child_hkey); }
                if active {
                    return true;
                }
            } else {
                // Direct subkey (UWP app)
                let active = is_app_active(child_hkey, None);
                unsafe { let _ = RegCloseKey(child_hkey); }
                if active {
                    return true;
                }
            }
        }
    }

    false
}

fn scan_app_keys(parent: HKEY) -> bool {
    let mut idx = 0;
    let mut name_buf = [0u16; 260];

    loop {
        let mut name_len = name_buf.len() as u32;
        let r = unsafe {
            RegEnumKeyExW(
                parent,
                idx,
                windows::core::PWSTR(name_buf.as_mut_ptr()),
                &mut name_len,
                None,
                windows::core::PWSTR::null(),
                None,
                None,
            )
        };
        if r.is_err() {
            break;
        }
        idx += 1;

        let mut app_hkey = HKEY::default();
        if unsafe {
            RegOpenKeyExW(
                parent,
                PCWSTR(name_buf.as_ptr()),
                0,
                KEY_READ,
                &mut app_hkey,
            )
        }.is_ok() {
            let subkey_slice = &name_buf[..name_len as usize];
            let active = is_app_active(app_hkey, Some(subkey_slice));
            unsafe { let _ = RegCloseKey(app_hkey); }
            if active {
                return true;
            }
        }
    }

    false
}

fn is_app_active(app_hkey: HKEY, key_name: Option<&[u16]>) -> bool {
    let mut start_time = 0u64;
    let mut start_size = std::mem::size_of::<u64>() as u32;
    let start_res = unsafe {
        RegQueryValueExW(
            app_hkey,
            w!("LastUsedTimeStart"),
            None,
            None,
            Some(&mut start_time as *mut u64 as *mut u8),
            Some(&mut start_size),
        )
    };

    let mut stop_time = 0u64;
    let mut stop_size = std::mem::size_of::<u64>() as u32;
    let stop_res = unsafe {
        RegQueryValueExW(
            app_hkey,
            w!("LastUsedTimeStop"),
            None,
            None,
            Some(&mut stop_time as *mut u64 as *mut u8),
            Some(&mut stop_size),
        )
    };

    let effective_start = if start_res.is_ok() { start_time } else { 0 };
    let effective_stop = if stop_res.is_ok() { stop_time } else { 0 };

    if poni_core::privacy::is_usage_active(effective_start, effective_stop) {
        // If key_name is provided (e.g. NonPackaged app path encoded with '#'),
        // verify that the process is actually running using ToolHelp32 snapshot.
        if let Some(name_slice) = key_name {
            if let Some(exe_name) = extract_exe_name(name_slice) {
                if !is_process_running(&exe_name) {
                    return false;
                }
            }
        }
        let label = key_name.map(String::from_utf16_lossy).unwrap_or_else(|| "UWP-App".into());
        crate::pinfo!("privacy: active sensor detected on key: {} (start={}, stop={})", label, effective_start, effective_stop);
        return true;
    }

    false
}

fn extract_exe_name(key_name: &[u16]) -> Option<String> {
    let s = String::from_utf16_lossy(key_name);
    let trimmed = s.trim_matches(['\0', ' ', '\t', '\r', '\n']);
    let last = trimmed.rsplit(['#', '\\', '/']).next()?.trim();
    if last.to_lowercase().ends_with(".exe") {
        Some(last.to_string())
    } else {
        None
    }
}

fn is_process_running(target_exe: &str) -> bool {
    unsafe {
        let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(s) => s,
            Err(_) => return true, // fallback to true if snapshot fails
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = false;
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let sz_exe = &entry.szExeFile;
                let len = sz_exe.iter().position(|&c| c == 0).unwrap_or(sz_exe.len());
                let exe_name = String::from_utf16_lossy(&sz_exe[..len]);
                if exe_name.eq_ignore_ascii_case(target_exe) {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        found
    }
}

fn is_utf16_equal(slice: &[u16], s: &str) -> bool {
    let mut s_chars = s.encode_utf16();
    for &ch in slice {
        if s_chars.next() != Some(ch) {
            return false;
        }
    }
    s_chars.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_exe_name_standard_hash_path() {
        let path: Vec<u16> = "C:#Program Files#Google#Chrome#Application#chrome.exe"
            .encode_utf16()
            .collect();
        assert_eq!(extract_exe_name(&path), Some("chrome.exe".to_string()));
    }

    #[test]
    fn test_extract_exe_name_with_spaces_and_uppercase() {
        let path: Vec<u16> = "C:#Users#furim#AppData#Roaming#Zoom#bin#Zoom.EXE"
            .encode_utf16()
            .collect();
        assert_eq!(extract_exe_name(&path), Some("Zoom.EXE".to_string()));
    }

    #[test]
    fn test_extract_exe_name_with_trailing_null_or_spaces() {
        let mut path: Vec<u16> = "C:#Program Files#Mozilla Firefox#firefox.exe "
            .encode_utf16()
            .collect();
        path.push(0);
        assert_eq!(extract_exe_name(&path), Some("firefox.exe".to_string()));
    }

    #[test]
    fn test_extract_exe_name_non_exe() {
        let path: Vec<u16> = "Microsoft.WindowsCamera_8wekyb3d8bbwe"
            .encode_utf16()
            .collect();
        assert_eq!(extract_exe_name(&path), None);
    }

    #[test]
    fn test_is_process_running_real_system_process() {
        assert!(is_process_running("explorer.exe"), "explorer.exe should always be running on Windows");
        assert!(!is_process_running("definitely_not_a_running_process_9999.exe"));
    }

    #[test]
    fn test_is_report_streaming_none() {
        assert!(!is_report_streaming(None));
    }

    #[test]
    fn test_is_any_mic_active_no_panic() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let _ = is_any_mic_active();
            CoUninitialize();
        }
    }

    #[test]
    fn test_mf_sensor_activity_monitor_lifecycle() {
        unsafe {
            if MFStartup(MF_SDK_VERSION, MFSTARTUP_NOSOCKET).is_ok() {
                let streaming = Arc::new(AtomicBool::new(false));
                let dummy_event = CreateEventW(None, false, false, None).unwrap_or_default();
                let cb_impl = SensorActivityCallback {
                    streaming: streaming.clone(),
                    notify_event: dummy_event.0 as usize,
                };
                let cb: IMFSensorActivitiesReportCallback = cb_impl.into();
                if let Ok(mon) = MFCreateSensorActivityMonitor(&cb) {
                    assert!(mon.Start().is_ok());
                    assert!(mon.Stop().is_ok());
                }
                let _ = CloseHandle(dummy_event);
                let _ = MFShutdown();
            }
        }
    }
}
