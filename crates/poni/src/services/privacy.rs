//! Privacy service: detects Windows system-wide camera and microphone activity
//! using event-driven registry change notifications (RegNotifyChangeKeyValue).
//! Zero polling: blocks on Win32 sync primitives until Windows signals a sensor status change.

use crate::events::{AppEvent, EventSink};
use poni_core::privacy::PrivacyState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegNotifyChangeKeyValue, RegOpenKeyExW, RegQueryValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET,
};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, INFINITE};

const CAM_PATH: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\webcam");
const MIC_PATH: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone");

pub struct PrivacyService {
    stop: Arc<AtomicBool>,
    stop_event: HANDLE,
    worker: Option<JoinHandle<()>>,
}

// HANDLE wrapper for Send + Sync across thread bounds safely

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
    let mut cam_hkey = HKEY::default();
    let mut mic_hkey = HKEY::default();

    let cam_ok = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, CAM_PATH, 0, KEY_READ | KEY_NOTIFY, &mut cam_hkey) }.is_ok();
    let mic_ok = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, MIC_PATH, 0, KEY_READ | KEY_NOTIFY, &mut mic_hkey) }.is_ok();

    if !cam_ok && !mic_ok {
        crate::pwarn!("privacy watcher: failed to open consent store registry keys");
        return;
    }

    let cam_event = unsafe { CreateEventW(None, false, false, None) }.unwrap_or_default();
    let mic_event = unsafe { CreateEventW(None, false, false, None) }.unwrap_or_default();

    // Initial sensor state evaluation
    let mut last_state = evaluate_privacy(cam_hkey, mic_hkey);
    sink.send(AppEvent::Privacy(last_state));

    while !stop.load(Ordering::SeqCst) {
        // Arm notifications (watching the whole subtree so subkey app changes trigger it)
        if cam_ok {
            unsafe {
                let _ = RegNotifyChangeKeyValue(cam_hkey, true, REG_NOTIFY_CHANGE_LAST_SET, cam_event, true);
            }
        }
        if mic_ok {
            unsafe {
                let _ = RegNotifyChangeKeyValue(mic_hkey, true, REG_NOTIFY_CHANGE_LAST_SET, mic_event, true);
            }
        }

        let handles = [stop_event, cam_event, mic_event];
        let wait_res = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };

        if stop.load(Ordering::SeqCst) || wait_res == WAIT_OBJECT_0 {
            break;
        }

        // State changed on camera or mic
        let new_state = evaluate_privacy(cam_hkey, mic_hkey);
        if new_state != last_state {
            last_state = new_state;
            sink.send(AppEvent::Privacy(new_state));
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
    }
}

fn evaluate_privacy(cam_hkey: HKEY, mic_hkey: HKEY) -> PrivacyState {
    let camera = if !cam_hkey.is_invalid() {
        is_key_tree_active(cam_hkey)
    } else {
        false
    };
    let microphone = if !mic_hkey.is_invalid() {
        is_key_tree_active(mic_hkey)
    } else {
        false
    };
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
    // Check LastUsedTimeStop: if 0 and LastUsedTimeStart > 0, it is actively in use!
    let mut stop_time = 0u64;
    let mut size = std::mem::size_of::<u64>() as u32;

    let res = unsafe {
        RegQueryValueExW(
            app_hkey,
            w!("LastUsedTimeStop"),
            None,
            None,
            Some(&mut stop_time as *mut u64 as *mut u8),
            Some(&mut size),
        )
    };

    if res.is_ok() && stop_time == 0 {
        // Double check LastUsedTimeStart to ensure it's not an unused stub
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
        if start_res.is_ok() && start_time > 0 {
            // Rule 1: If key_name is provided (e.g. NonPackaged app path encoded with '#'),
            // verify that the process is actually running using ToolHelp32 snapshot.
            if let Some(name_slice) = key_name {
                if let Some(exe_name) = extract_exe_name(name_slice) {
                    if !is_process_running(&exe_name) {
                        return false;
                    }
                }
            }
            return true;
        }
    }

    false
}

fn extract_exe_name(key_name: &[u16]) -> Option<String> {
    let s = String::from_utf16_lossy(key_name);
    let last = s.rsplit(['#', '\\', '/']).next()?;
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
