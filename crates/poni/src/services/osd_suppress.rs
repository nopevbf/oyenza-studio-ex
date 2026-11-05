//! Interception of media and brightness keys to replace Windows default OSDs.
//!
//! Rule 2: Transient state only. Dropping this service unhooks cleanly.
//! Rule 3: Graceful UIPI handling. If the foreground window is elevated,
//! keys pass through to CallNextHookEx without flickering or hanging.

use crate::events::EventSink;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetWindowThreadProcessId, SetWindowsHookExW,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};
use windows::Win32::System::Threading::{
    OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};

// Virtual key codes for brightness up/down used on some keyboard drivers
const VK_BRIGHTNESS_DOWN: u32 = 0xD8;
const VK_BRIGHTNESS_UP: u32 = 0xD9;

static SUPPRESS_VOLUME: AtomicBool = AtomicBool::new(false);
static SUPPRESS_BRIGHTNESS: AtomicBool = AtomicBool::new(false);

static GLOBAL_HOOK: Mutex<Option<isize>> = Mutex::new(None);
static GLOBAL_SINK: Mutex<Option<EventSink>> = Mutex::new(None);

pub struct OsdSuppressService {
    _hook: HHOOK,
}

impl OsdSuppressService {
    pub fn start(sink: EventSink, suppress_volume: bool, suppress_brightness: bool) -> Option<Self> {
        SUPPRESS_VOLUME.store(suppress_volume, Ordering::Relaxed);
        SUPPRESS_BRIGHTNESS.store(suppress_brightness, Ordering::Relaxed);

        if let Ok(mut g_sink) = GLOBAL_SINK.lock() {
            *g_sink = Some(sink);
        }

        unsafe {
            let hook = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(low_level_keyboard_proc),
                HINSTANCE::default(),
                0,
            ).ok()?;

            if let Ok(mut g_hook) = GLOBAL_HOOK.lock() {
                *g_hook = Some(hook.0 as isize);
            }

            Some(Self { _hook: hook })
        }
    }

    pub fn set_suppress_volume(&self, enable: bool) {
        SUPPRESS_VOLUME.store(enable, Ordering::Relaxed);
    }

    pub fn set_suppress_brightness(&self, enable: bool) {
        SUPPRESS_BRIGHTNESS.store(enable, Ordering::Relaxed);
    }
}

impl Drop for OsdSuppressService {
    fn drop(&mut self) {
        unsafe {
            if let Ok(mut g_hook) = GLOBAL_HOOK.lock() {
                if let Some(h) = g_hook.take() {
                    let _ = UnhookWindowsHookEx(HHOOK(h as *mut core::ffi::c_void));
                }
            }
            if let Ok(mut g_sink) = GLOBAL_SINK.lock() {
                *g_sink = None;
            }
        }
    }
}

unsafe extern "system" fn low_level_keyboard_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code >= 0 && (w_param.0 as u32 == WM_KEYDOWN || w_param.0 as u32 == WM_SYSKEYDOWN) {
        let kb = &*(l_param.0 as *const KBDLLHOOKSTRUCT);
        let vk = kb.vkCode;

        let is_volume_key = vk == VK_VOLUME_UP.0 as u32
            || vk == VK_VOLUME_DOWN.0 as u32
            || vk == VK_VOLUME_MUTE.0 as u32;

        let is_brightness_key = vk == VK_BRIGHTNESS_UP || vk == VK_BRIGHTNESS_DOWN;

        // Rule 3: Check if foreground window is elevated
        if (is_volume_key && SUPPRESS_VOLUME.load(Ordering::Relaxed))
            || (is_brightness_key && SUPPRESS_BRIGHTNESS.load(Ordering::Relaxed))
        {
            if is_foreground_elevated() {
                // Let Windows handle the key normally under elevated apps to prevent UIPI conflict
                return CallNextHookEx(HHOOK::default(), n_code, w_param, l_param);
            }

            if is_volume_key && SUPPRESS_VOLUME.load(Ordering::Relaxed) {
                // Key intercepted: perform step programmatically
                if vk == VK_VOLUME_UP.0 as u32 {
                    step_master_volume(0.02);
                } else if vk == VK_VOLUME_DOWN.0 as u32 {
                    step_master_volume(-0.02);
                } else if vk == VK_VOLUME_MUTE.0 as u32 {
                    toggle_master_mute();
                }
                // Return 1 to swallow key so Windows native OSD does not render
                return LRESULT(1);
            }

            if is_brightness_key && SUPPRESS_BRIGHTNESS.load(Ordering::Relaxed) {
                let delta = if vk == VK_BRIGHTNESS_UP { 0.05 } else { -0.05 };
                let _ = crate::services::brightness::apply_system_brightness_delta(delta);
                return LRESULT(1);
            }
        }
    }

    CallNextHookEx(HHOOK::default(), n_code, w_param, l_param)
}

/// Helper to step master volume using Core Audio endpoints.
fn step_master_volume(delta: f32) {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    unsafe {
        if let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) {
            if let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
                if let Ok(vol) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    if delta > 0.0 {
                        let _ = vol.VolumeStepUp(std::ptr::null());
                    } else {
                        let _ = vol.VolumeStepDown(std::ptr::null());
                    }
                }
            }
        }
    }
}

/// Helper to toggle master mute.
fn toggle_master_mute() {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    unsafe {
        if let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) {
            if let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
                if let Ok(vol) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    if let Ok(muted) = vol.GetMute() {
                        let _ = vol.SetMute(!muted.as_bool(), std::ptr::null());
                    }
                }
            }
        }
    }
}

/// Rule 3: Inspect whether foreground process is running with elevated privileges.
fn is_foreground_elevated() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return false;
        }
        if let Ok(proc_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut token = Default::default();
            if OpenProcessToken(proc_handle, TOKEN_QUERY, &mut token).is_ok() {
                let mut elevation = TOKEN_ELEVATION::default();
                let mut size = 0u32;
                if GetTokenInformation(
                    token,
                    TokenElevation,
                    Some(&mut elevation as *mut _ as *mut _),
                    std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                    &mut size,
                ).is_ok() {
                    return elevation.TokenIsElevated != 0;
                }
            }
        }
    }
    false
}
