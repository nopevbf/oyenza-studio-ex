//! Wi-Fi connect/disconnect HUD notifications.
//!
//! Uses `WlanRegisterNotification` to receive ACM notifications when the
//! current interface connects to or disconnects from a network. On each
//! notification the worker reads the current connection state with
//! `WlanQueryInterface` and dispatches `AppEvent::Wifi`.
//!
//! Idle cost: the callback runs on the WLAN API's internal thread pool and
//! only sends a small message. The worker thread sleeps in a blocking `recv`;
//! nothing polls.

use crate::events::{AppEvent, EventSink};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use windows::Win32::Foundation::{BOOL, ERROR_SUCCESS, HANDLE};
use windows::Win32::NetworkManagement::WiFi::{
    wlan_interface_state_connected, wlan_intf_opcode_current_connection,
    wlan_notification_acm_connection_complete, wlan_notification_acm_disconnected,
    L2_NOTIFICATION_DATA, WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory,
    WlanOpenHandle, WlanQueryInterface, WlanRegisterNotification, DOT11_SSID,
    WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO_LIST,
    WLAN_NOTIFICATION_SOURCE_ACM, WLAN_NOTIFICATION_SOURCES,
};

/// A message the WLAN callback sends to the worker thread.
enum Msg {
    /// ACM connected or disconnected notification arrived.
    Changed,
    /// Sent by `Drop` to stop the worker.
    Stop,
}

/// Wi-Fi connect/disconnect watcher. Dropping it stops the thread.
pub struct WifiService {
    handle: HANDLE,
    tx: Sender<Msg>,
    callback_ctx: *mut core::ffi::c_void,
    thread: Option<JoinHandle<()>>,
}

unsafe impl Send for WifiService {}
unsafe impl Sync for WifiService {}

impl WifiService {
    /// Start the Wi-Fi watcher. Returns an error if `WlanOpenHandle` fails
    /// (e.g. the WLAN service is not running — normal on desktops without Wi-Fi).
    pub fn start(sink: EventSink) -> windows::core::Result<Self> {
        let (tx, rx) = mpsc::channel::<Msg>();
        let tx_cb = tx.clone();

        // Open a WLAN client handle.
        let mut negotiated = 0u32;
        let mut handle = HANDLE::default();
        let rc = unsafe { WlanOpenHandle(2, None, &mut negotiated, &mut handle) };
        if rc != ERROR_SUCCESS.0 {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(-1i32),
                format!("WlanOpenHandle failed: {rc}"),
            ));
        }

        // Register for ACM (auto-config manager) notifications: connect/disconnect.
        let tx_raw = Box::into_raw(Box::new(tx_cb)) as *mut core::ffi::c_void;
        let rc = unsafe {
            WlanRegisterNotification(
                handle,
                WLAN_NOTIFICATION_SOURCE_ACM,
                BOOL::from(false),
                Some(wlan_callback),
                Some(tx_raw as *const core::ffi::c_void),
                None,
                None,
            )
        };
        if rc != ERROR_SUCCESS.0 {
            unsafe { WlanCloseHandle(handle, None) };
            drop(unsafe { Box::from_raw(tx_raw as *mut Sender<Msg>) });
            return Err(windows::core::Error::new(
                windows::core::HRESULT(-1i32),
                format!("WlanRegisterNotification failed: {rc}"),
            ));
        }

        let handle_raw = handle.0 as isize;
        let thread = std::thread::Builder::new()
            .name("poni-wifi".into())
            .spawn(move || {
                let handle = HANDLE(handle_raw as *mut core::ffi::c_void);
                worker(handle, rx, sink)
            })
            .map_err(|e| {
                unsafe {
                    let _ = WlanRegisterNotification(
                        handle,
                        WLAN_NOTIFICATION_SOURCES(0),
                        BOOL::from(false),
                        None,
                        None,
                        None,
                        None,
                    );
                    WlanCloseHandle(handle, None);
                    drop(Box::from_raw(tx_raw as *mut Sender<Msg>));
                }
                windows::core::Error::new(
                    windows::core::HRESULT(-1i32),
                    format!("wifi thread: {e}"),
                )
            })?;

        Ok(Self {
            handle,
            tx,
            callback_ctx: tx_raw,
            thread: Some(thread),
        })
    }
}

impl Drop for WifiService {
    fn drop(&mut self) {
        // Unregister notifications first so wlan_callback won't be invoked.
        unsafe {
            let _ = WlanRegisterNotification(
                self.handle,
                WLAN_NOTIFICATION_SOURCES(0),
                BOOL::from(false),
                None,
                None,
                None,
                None,
            );
        }
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            if t.join().is_err() {
                crate::pwarn!("wifi: worker thread panicked");
            }
        }
        unsafe {
            WlanCloseHandle(self.handle, None);
            drop(Box::from_raw(self.callback_ctx as *mut Sender<Msg>));
        }
    }
}

/// WLAN callback — runs on WLAN API's internal thread pool.
/// Sends a lightweight message to the worker; does nothing expensive.
unsafe extern "system" fn wlan_callback(
    data: *mut L2_NOTIFICATION_DATA,
    context: *mut core::ffi::c_void,
) {
    if context.is_null() || data.is_null() {
        return;
    }
    let d = &*data;
    if d.NotificationSource != WLAN_NOTIFICATION_SOURCE_ACM {
        return;
    }
    if d.NotificationCode != wlan_notification_acm_connection_complete.0 as u32
        && d.NotificationCode != wlan_notification_acm_disconnected.0 as u32
    {
        return;
    }
    let tx = &*(context as *const Sender<Msg>);
    let _ = tx.send(Msg::Changed);
}

/// Determine if a Wi-Fi state transition should emit an event.
/// On initial startup (`is_initial: true`), returns `None` so the app starts silently as the normal notch.
/// On subsequent changes, returns `Some(AppEvent::Wifi)` only when connection state or SSID changes.
pub fn transition_event(last: &Option<(String, bool)>, curr: &Option<(String, bool)>, is_initial: bool) -> Option<AppEvent> {
    if is_initial {
        return None;
    }
    if curr != last {
        if let Some((ssid, connected)) = curr {
            return Some(AppEvent::Wifi {
                ssid: ssid.clone(),
                connected: *connected,
            });
        }
    }
    None
}

/// Worker thread: reads connection state and dispatches AppEvent::Wifi.
fn worker(handle: HANDLE, rx: Receiver<Msg>, sink: EventSink) {
    // Initial state check is silent: existing connection is recorded so startup
    // stays as the normal notch and never triggers a connection toast.
    let mut last_state = query_connection(handle);

    loop {
        match rx.recv() {
            Ok(Msg::Stop) | Err(_) => break,
            Ok(Msg::Changed) => {
                // Brief pause so Windows finishes updating the connection state.
                std::thread::sleep(std::time::Duration::from_millis(150));
                let curr = query_connection(handle);
                if let Some(ev) = transition_event(&last_state, &curr, false) {
                    sink.send(ev);
                }
                last_state = curr;
            }
        }
    }
}

/// Query the first connected Wi-Fi interface. Returns `(ssid, connected)`.
/// Returns `None` if WlanEnumInterfaces fails or has no Wi-Fi adapters.
fn query_connection(handle: HANDLE) -> Option<(String, bool)> {
    let mut iface_list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
    let rc = unsafe { WlanEnumInterfaces(handle, None, &mut iface_list) };
    if rc != ERROR_SUCCESS.0 || iface_list.is_null() {
        return None;
    }
    let list = unsafe { &*iface_list };
    if list.dwNumberOfItems == 0 {
        unsafe { WlanFreeMemory(iface_list as *const core::ffi::c_void) };
        return None;
    }
    let guid = unsafe { (*iface_list).InterfaceInfo[0].InterfaceGuid };
    unsafe { WlanFreeMemory(iface_list as *const core::ffi::c_void) };

    let mut data_size = 0u32;
    let mut data_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
    let rc = unsafe {
        WlanQueryInterface(
            handle,
            &guid,
            wlan_intf_opcode_current_connection,
            None,
            &mut data_size,
            &mut data_ptr,
            None,
        )
    };
    if rc != ERROR_SUCCESS.0 || data_ptr.is_null() {
        return Some(("Wi-Fi".into(), false));
    }

    let attrs = unsafe { &*(data_ptr as *const WLAN_CONNECTION_ATTRIBUTES) };
    let connected = attrs.isState == wlan_interface_state_connected;
    let ssid = ssid_to_string(&attrs.wlanAssociationAttributes.dot11Ssid);
    unsafe { WlanFreeMemory(data_ptr as *const core::ffi::c_void) };

    Some((ssid, connected))
}

/// Convert a `DOT11_SSID` (raw bytes, length-prefixed) to a UTF-8 `String`.
fn ssid_to_string(ssid: &DOT11_SSID) -> String {
    let len = ssid.uSSIDLength as usize;
    let bytes = &ssid.ucSSID[..len.min(32)];
    if bytes.is_empty() {
        "Wi-Fi".into()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wifi_startup_is_silent() {
        // Initial state query with active connection must produce None (no event emitted).
        let initial = Some(("Office_5G".to_string(), true));
        let ev = transition_event(&None, &initial, true);
        assert!(ev.is_none(), "Startup must be completely silent so notch is displayed immediately");
    }

    #[test]
    fn test_wifi_transition_emits_on_change() {
        let last = Some(("Office_5G".to_string(), true));
        let curr = Some(("Office_5G".to_string(), false));
        let ev = transition_event(&last, &curr, false);
        assert!(
            matches!(ev, Some(AppEvent::Wifi { ref ssid, connected: false }) if ssid == "Office_5G"),
            "Event must be emitted on connection state change"
        );
    }

    #[test]
    fn test_wifi_transition_ignores_same_state() {
        let last = Some(("Office_5G".to_string(), true));
        let curr = Some(("Office_5G".to_string(), true));
        let ev = transition_event(&last, &curr, false);
        assert!(ev.is_none(), "No event should be emitted when connection state has not changed");
    }
}
