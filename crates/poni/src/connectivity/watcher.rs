//! OS event watchers for Ethernet and event adapters for Bluetooth and Wi-Fi.

use crate::connectivity::model::{ConnEvent, ConnKind, ConnState, Debouncer, DeviceKind};
use crate::events::{AppEvent, EventSink};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    BOOLEAN, ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS, HANDLE, NO_ERROR, WIN32_ERROR,
};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW,
    CM_Locate_DevNodeW, CM_LOCATE_DEVNODE_FLAGS, CM_GETIDLIST_FILTER_ENUMERATOR, CM_GETIDLIST_FILTER_PRESENT, CR_SUCCESS,
};
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_FriendlyName, DEVPKEY_NAME, DEVPROPKEY, DEVPROPTYPE,
};
use windows::Win32::NetworkManagement::IpHelper::{
    CancelMibChangeNotify2, GetAdaptersAddresses, NotifyIpInterfaceChange, GAA_FLAG_SKIP_ANYCAST,
    GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE,
};
use windows::Win32::Networking::WinSock::AF_UNSPEC;

const IF_TYPE_ETHERNET: u32 = 6;

/// DEVPKEY_Bluetooth_Battery ({104EA319-6EE2-4701-BD47-8DDBF425BBE5}, 2)
pub const DEVPKEY_BLUETOOTH_BATTERY: DEVPROPKEY = DEVPROPKEY {
    fmtid: windows::core::GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 2,
};

enum Msg {
    Changed,
    Stop,
}

// ---------------------------------------------------------------------------
// Bluetooth PnP Watcher (cfgmgr32 BTHENUM PRESENT) - Reflection from Nusa
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct BluetoothDeviceSnapshot {
    pub mac: String,
    pub name: String,
    pub battery: Option<u8>,
    pub audio: bool,
}

pub struct BluetoothPnpWatcher {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl BluetoothPnpWatcher {
    pub fn start(sink: EventSink) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("poni-bt-pnp".into())
            .spawn(move || worker_bluetooth_pnp(rx, sink))
            .map_err(|e| format!("bluetooth pnp thread spawn failed: {e}"))?;
        Ok(Self { tx, thread: Some(thread) })
    }
}

impl Drop for BluetoothPnpWatcher {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn worker_bluetooth_pnp(rx: Receiver<Msg>, sink: EventSink) {
    // Initial scan is silent so existing connections are not treated as new connects
    let mut known: HashMap<String, BluetoothDeviceSnapshot> = HashMap::new();
    for dev in query_present_bluetooth_devices() {
        known.insert(dev.mac.clone(), dev);
    }
    crate::pinfo!("bluetooth-pnp: initialized with {} connected device(s)", known.len());

    let interval = Duration::from_millis(1500);
    loop {
        match rx.recv_timeout(interval) {
            Ok(Msg::Stop) => break,
            Ok(Msg::Changed) | Err(mpsc::RecvTimeoutError::Timeout) => {
                let current_list = query_present_bluetooth_devices();
                let mut current_map: HashMap<String, BluetoothDeviceSnapshot> = HashMap::new();
                for dev in current_list {
                    current_map.insert(dev.mac.clone(), dev);
                }

                // Check for newly connected devices or battery updates
                for (mac, curr) in &current_map {
                    match known.get(mac) {
                        None => {
                            crate::pinfo!("bluetooth-pnp: device connected: {} ({mac}, audio {})", curr.name, curr.audio);
                            sink.send(AppEvent::Bluetooth {
                                name: curr.name.clone(),
                                connected: true,
                                battery: curr.battery,
                                audio: curr.audio,
                            });
                        }
                        Some(prev) => {
                            if prev.battery != curr.battery && curr.battery.is_some() {
                                crate::pinfo!("bluetooth-pnp: device battery updated: {} -> {:?}", curr.name, curr.battery);
                                sink.send(AppEvent::Bluetooth {
                                    name: curr.name.clone(),
                                    connected: true,
                                    battery: curr.battery,
                                    audio: curr.audio,
                                });
                            }
                        }
                    }
                }

                // Check for disconnected devices
                for (mac, prev) in &known {
                    if !current_map.contains_key(mac) {
                        crate::pinfo!("bluetooth-pnp: device disconnected: {} ({mac})", prev.name);
                        sink.send(AppEvent::Bluetooth {
                            name: prev.name.clone(),
                            connected: false,
                            battery: None,
                            audio: prev.audio,
                        });
                    }
                }

                known = current_map;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Parse double-null-terminated REG_MULTI_SZ buffer into a list of strings.
pub fn parse_multi_sz(buf: &[u16]) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = Vec::new();
    for &ch in buf {
        if ch == 0 {
            if !current.is_empty() {
                result.push(String::from_utf16_lossy(&current));
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }
    result
}

/// Extract normalized Bluetooth MAC address from a device ID string.
pub fn extract_bt_mac(device_id: &str) -> Option<String> {
    let upper = device_id.to_uppercase();
    // Format 1: BTHENUM\Dev_1E928008E018\...
    if let Some(idx) = upper.find("DEV_") {
        let rest = &upper[idx + 4..];
        let mac: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if mac.len() >= 12 {
            return Some(mac);
        }
    }
    // Format 2: BTHENUM\{guid}...&...&1E928008E018_...
    for part in upper.split(['\\', '&', '_']) {
        if part.len() == 12 && part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(part.to_string());
        }
    }
    None
}

/// Query currently PRESENT BTHENUM device IDs using cfgmgr32.
pub fn query_present_bthenum_device_ids() -> Vec<String> {
    let filter: Vec<u16> = "BTHENUM\0".encode_utf16().collect();
    let mut len: u32 = 0;
    unsafe {
        let res = CM_Get_Device_ID_List_SizeW(
            &mut len,
            PCWSTR::from_raw(filter.as_ptr()),
            CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT,
        );
        if res != CR_SUCCESS || len <= 1 {
            return Vec::new();
        }
        let mut buf: Vec<u16> = vec![0; len as usize];
        let res2 = CM_Get_Device_ID_ListW(
            PCWSTR::from_raw(filter.as_ptr()),
            &mut buf,
            CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT,
        );
        if res2 != CR_SUCCESS {
            return Vec::new();
        }
        parse_multi_sz(&buf)
    }
}

/// Query string property of a devnode.
pub fn query_devnode_property_string(devnode: u32, key: &DEVPROPKEY) -> Option<String> {
    unsafe {
        let mut prop_type = DEVPROPTYPE::default();
        let mut size: u32 = 0;
        let _ = CM_Get_DevNode_PropertyW(devnode, key, &mut prop_type, None, &mut size, 0);
        if size == 0 {
            return None;
        }
        let mut buf: Vec<u8> = vec![0; size as usize];
        let res = CM_Get_DevNode_PropertyW(
            devnode,
            key,
            &mut prop_type,
            Some(buf.as_mut_ptr()),
            &mut size,
            0,
        );
        if res != CR_SUCCESS || buf.is_empty() {
            return None;
        }
        // UTF-16LE string: size in bytes
        let u16_slice = core::slice::from_raw_parts(buf.as_ptr() as *const u16, (size as usize) / 2);
        let s = String::from_utf16_lossy(u16_slice).trim_end_matches('\0').trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }
}

/// Query battery percentage of a devnode.
pub fn query_devnode_battery(devnode: u32) -> Option<u8> {
    unsafe {
        let mut prop_type = DEVPROPTYPE::default();
        let mut size: u32 = 0;
        let _ = CM_Get_DevNode_PropertyW(devnode, &DEVPKEY_BLUETOOTH_BATTERY, &mut prop_type, None, &mut size, 0);
        if size == 0 {
            return None;
        }
        let mut buf: Vec<u8> = vec![0; size as usize];
        let res = CM_Get_DevNode_PropertyW(
            devnode,
            &DEVPKEY_BLUETOOTH_BATTERY,
            &mut prop_type,
            Some(buf.as_mut_ptr()),
            &mut size,
            0,
        );
        if res == CR_SUCCESS && !buf.is_empty() {
            let batt = buf[0];
            if batt <= 100 {
                return Some(batt);
            }
        }
        None
    }
}

/// Query all currently connected Bluetooth devices with name and battery.
pub fn query_present_bluetooth_devices() -> Vec<BluetoothDeviceSnapshot> {
    let ids = query_present_bthenum_device_ids();
    if ids.is_empty() {
        return Vec::new();
    }

    let mut map: HashMap<String, BluetoothDeviceSnapshot> = HashMap::new();

    for id in &ids {
        let mac = match extract_bt_mac(id) {
            Some(m) => m,
            None => continue,
        };

        let mut devnode: u32 = 0;
        let id_u16: Vec<u16> = format!("{id}\0").encode_utf16().collect();
        let res = unsafe { CM_Locate_DevNodeW(&mut devnode, PCWSTR::from_raw(id_u16.as_ptr()), CM_LOCATE_DEVNODE_FLAGS(0)) };
        if res != CR_SUCCESS {
            continue;
        }

        let name = query_devnode_property_string(devnode, &DEVPKEY_Device_FriendlyName)
            .or_else(|| query_devnode_property_string(devnode, &DEVPKEY_NAME))
            .unwrap_or_default();

        let battery = query_devnode_battery(devnode);
        let id_upper = id.to_uppercase();
        // A2DP Audio Sink: 0000110b
        // AVRCP Transport: 0000110e
        // Handsfree / Headset: 0000111e, 00001108
        let is_audio = id_upper.contains("0000110B")
            || id_upper.contains("0000110E")
            || id_upper.contains("0000111E")
            || id_upper.contains("00001108")
            || is_bluetooth_audio_endpoint(id, &name);

        map.entry(mac.clone())
            .and_modify(|existing| {
                if existing.name.is_empty() || (!name.is_empty() && !name.contains("Transport") && !name.contains("Avrcp")) {
                    existing.name = name.clone();
                }
                if existing.battery.is_none() && battery.is_some() {
                    existing.battery = battery;
                }
                existing.audio = existing.audio || is_audio;
            })
            .or_insert_with(|| BluetoothDeviceSnapshot {
                mac,
                name: if name.is_empty() { "Bluetooth Device".to_string() } else { name },
                battery,
                audio: is_audio,
            });
    }

    map.into_values().collect()
}

// ---------------------------------------------------------------------------
// Ethernet Watcher (NotifyIpInterfaceChange)
// ---------------------------------------------------------------------------

/// Ethernet connection watcher using `NotifyIpInterfaceChange`.
pub struct EthernetWatcher {
    handle: HANDLE,
    tx: Sender<Msg>,
    callback_ctx: *mut core::ffi::c_void,
    thread: Option<JoinHandle<()>>,
}

unsafe impl Send for EthernetWatcher {}
unsafe impl Sync for EthernetWatcher {}

impl EthernetWatcher {
    pub fn start(sink: EventSink) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<Msg>();
        let tx_cb = tx.clone();

        let tx_raw = Box::into_raw(Box::new(tx_cb)) as *mut core::ffi::c_void;
        let mut handle = HANDLE::default();

        let rc = unsafe {
            NotifyIpInterfaceChange(
                AF_UNSPEC,
                Some(ip_change_callback),
                Some(tx_raw as *const core::ffi::c_void),
                BOOLEAN::from(false),
                &mut handle,
            )
        };

        if rc != NO_ERROR && rc != WIN32_ERROR(ERROR_SUCCESS.0) {
            drop(unsafe { Box::from_raw(tx_raw as *mut Sender<Msg>) });
            return Err(format!("NotifyIpInterfaceChange failed: {rc:?}"));
        }

        let thread = std::thread::Builder::new()
            .name("poni-ethernet".into())
            .spawn(move || worker(rx, sink))
            .map_err(|e| {
                unsafe {
                    let _ = CancelMibChangeNotify2(handle);
                    drop(Box::from_raw(tx_raw as *mut Sender<Msg>));
                }
                format!("ethernet thread spawn failed: {e}")
            })?;

        Ok(Self {
            handle,
            tx,
            callback_ctx: tx_raw,
            thread: Some(thread),
        })
    }
}

impl Drop for EthernetWatcher {
    fn drop(&mut self) {
        unsafe {
            let _ = CancelMibChangeNotify2(self.handle);
        }
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        unsafe {
            drop(Box::from_raw(self.callback_ctx as *mut Sender<Msg>));
        }
    }
}

unsafe extern "system" fn ip_change_callback(
    callercontext: *const core::ffi::c_void,
    _row: *const MIB_IPINTERFACE_ROW,
    _notificationtype: MIB_NOTIFICATION_TYPE,
) {
    if callercontext.is_null() {
        return;
    }
    let tx = &*(callercontext as *const Sender<Msg>);
    let _ = tx.send(Msg::Changed);
}

fn worker(rx: Receiver<Msg>, sink: EventSink) {
    let mut debouncer = Debouncer::new(2.5);
    let mut last_state = is_ethernet_connected();

    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Stop => break,
            Msg::Changed => {
                std::thread::sleep(std::time::Duration::from_millis(2500));
                let curr_state = is_ethernet_connected();
                if curr_state != last_state {
                    last_state = curr_state;
                    let state = if curr_state {
                        ConnState::Connected
                    } else {
                        ConnState::Disconnected
                    };
                    let now = crate::clock::mono();
                    if debouncer.should_emit("ethernet", state, now) {
                        sink.send(AppEvent::Connectivity(ConnEvent {
                            kind: ConnKind::Ethernet,
                            state,
                            timestamp: now,
                        }));
                    }
                }
            }
        }
    }
}

/// Check if an adapter name or description corresponds to a virtual/tunnel interface.
pub fn is_virtual_adapter(friendly: &str, desc: &str) -> bool {
    let lower_f = friendly.to_lowercase();
    let lower_d = desc.to_lowercase();
    let virtual_keywords = [
        "vethernet", "wsl", "docker", "tailscale", "tap-", "virtual",
        "hyper-v", "vmware", "host-only", "wireguard", "vpn", "loopback",
    ];
    for kw in virtual_keywords {
        if lower_f.contains(kw) || lower_d.contains(kw) {
            return true;
        }
    }
    false
}

#[cfg(windows)]
unsafe fn has_valid_unicast_ip(mut addr_ptr: *const windows::Win32::NetworkManagement::IpHelper::IP_ADAPTER_UNICAST_ADDRESS_LH) -> bool {
    while !addr_ptr.is_null() {
        let addr = &*addr_ptr;
        let lp_sockaddr = addr.Address.lpSockaddr;
        if !lp_sockaddr.is_null() {
            let family = (*lp_sockaddr).sa_family;
            if family == windows::Win32::Networking::WinSock::AF_INET {
                let sin = &*(lp_sockaddr as *const windows::Win32::Networking::WinSock::SOCKADDR_IN);
                let ip_bytes = sin.sin_addr.S_un.S_addr.to_ne_bytes();
                // Check not APIPA (169.254.x.x) and not 0.0.0.0
                if ip_bytes[0] != 0 && !(ip_bytes[0] == 169 && ip_bytes[1] == 254) {
                    return true;
                }
            } else if family == windows::Win32::Networking::WinSock::AF_INET6 {
                return true;
            }
        }
        addr_ptr = addr.Next;
    }
    false
}

/// Query whether any physical Ethernet adapter is currently connected (`OperStatus == IfOperStatusUp`).
pub fn is_ethernet_connected() -> bool {
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size: u32 = 16 * 1024;
    for _ in 0..3 {
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let head = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        let r = unsafe { GetAdaptersAddresses(AF_UNSPEC.0 as u32, flags, None, Some(head), &mut size) };
        if r == ERROR_BUFFER_OVERFLOW.0 {
            continue;
        }
        if r != NO_ERROR.0 {
            return false;
        }
        let mut p = head as *const IP_ADAPTER_ADDRESSES_LH;
        while !p.is_null() {
            let a = unsafe { &*p };
            if a.IfType == IF_TYPE_ETHERNET && a.OperStatus.0 == 1 /* IfOperStatusUp */ {
                let friendly = if !a.FriendlyName.is_null() {
                    unsafe { a.FriendlyName.to_string().unwrap_or_default() }
                } else {
                    String::new()
                };
                let desc = if !a.Description.is_null() {
                    unsafe { a.Description.to_string().unwrap_or_default() }
                } else {
                    String::new()
                };
                if !is_virtual_adapter(&friendly, &desc) && unsafe { has_valid_unicast_ip(a.FirstUnicastAddress) } {
                    return true;
                }
            }
            p = a.Next;
        }
        break;
    }
    false
}

/// Helper to build a Bluetooth `ConnEvent`.
pub fn build_bluetooth_conn_event(
    name: String,
    connected: bool,
    battery: Option<u8>,
    audio: bool,
    now: f64,
) -> ConnEvent {
    let device_kind = if audio {
        let lower = name.to_lowercase();
        if lower.contains("buds") || lower.contains("earphone") || lower.contains("airpod") {
            DeviceKind::Earbuds
        } else if lower.contains("speaker") || lower.contains("soundbar") || lower.contains("stage") {
            DeviceKind::Speaker
        } else {
            DeviceKind::Headphones
        }
    } else {
        let lower = name.to_lowercase();
        if lower.contains("keyboard") || lower.contains("kb") {
            DeviceKind::Keyboard
        } else if lower.contains("mouse") {
            DeviceKind::Mouse
        } else if lower.contains("controller") || lower.contains("gamepad") || lower.contains("xbox") {
            DeviceKind::Gamepad
        } else if lower.contains("phone") || lower.contains("iphone") || lower.contains("android") {
            DeviceKind::Phone
        } else {
            DeviceKind::Generic
        }
    };

    ConnEvent {
        kind: ConnKind::Bluetooth {
            name,
            device_kind,
            battery,
        },
        state: if connected {
            ConnState::Connected
        } else {
            ConnState::Disconnected
        },
        timestamp: now,
    }
}

/// Helper to build a Wi-Fi `ConnEvent`.
pub fn build_wifi_conn_event(ssid: String, connected: bool, now: f64) -> ConnEvent {
    ConnEvent {
        kind: ConnKind::Wifi { ssid },
        state: if connected {
            ConnState::Connected
        } else {
            ConnState::Disconnected
        },
        timestamp: now,
    }
}

/// Determine if an audio endpoint represents a Bluetooth audio device (headphones, earbuds, speaker, etc.).
pub fn is_bluetooth_audio_endpoint(id: &str, name: &str) -> bool {
    let id_upper = id.to_uppercase();
    if id_upper.contains("BTHENUM") || id_upper.contains("BTHHFENUM") || id_upper.contains("BLUETOOTH") {
        return true;
    }
    let name_lower = name.to_lowercase();
    if name_lower.contains("bluetooth")
        || name_lower.contains("airpod")
        || name_lower.contains("buds")
        || name_lower.contains("wh-1000")
        || name_lower.contains("wf-1000")
        || name_lower.contains("freebuds")
        || name_lower.contains("wireless headset")
        || name_lower.contains("bt audio")
        || name_lower.contains("stage")
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_bluetooth_audio_endpoint() {
        assert!(is_bluetooth_audio_endpoint(r"SWD\MMDEVAPI\{0.0.0.00000000}.{123}\BTHENUM#...", "Headphones"));
        assert!(is_bluetooth_audio_endpoint(r"BTHHFENUM\...", "Hands-Free AG Audio"));
        assert!(is_bluetooth_audio_endpoint("{0.0.0.00000000}.{abc}", "Sony WH-1000XM4"));
        assert!(is_bluetooth_audio_endpoint("{0.0.0.00000000}.{abc}", "AirPods Pro - Find My"));
        assert!(is_bluetooth_audio_endpoint("{0.0.0.00000000}.{abc}", "Galaxy Buds2 Pro"));
        assert!(is_bluetooth_audio_endpoint("{0.0.0.00000000}.{abc}", "Bluetooth Audio Device"));
        assert!(is_bluetooth_audio_endpoint("{0.0.0.00000000}.{abc}", "HUAWEI FreeBuds Pro"));
        assert!(is_bluetooth_audio_endpoint("{0.0.0.00000000}.{abc}", "Xbox Wireless Headset"));
        assert!(is_bluetooth_audio_endpoint("{0.0.0.00000000}.{abc}", "Stage"));

        assert!(!is_bluetooth_audio_endpoint("{0.0.0.00000000}.{abc}", "Realtek High Definition Audio"));
        assert!(!is_bluetooth_audio_endpoint("{0.0.0.00000000}.{abc}", "Speakers / Headphones (Conexant ISST Audio)"));
        assert!(!is_bluetooth_audio_endpoint("{0.0.0.00000000}.{abc}", "27GL850 (NVIDIA High Definition Audio)"));
    }

    #[test]
    fn test_parse_multi_sz() {
        let input: Vec<u16> = "Dev1\0Dev2\0Dev3\0\0".encode_utf16().collect();
        let list = parse_multi_sz(&input);
        assert_eq!(list, vec!["Dev1", "Dev2", "Dev3"]);

        let empty: Vec<u16> = vec![0, 0];
        assert!(parse_multi_sz(&empty).is_empty());
    }

    #[test]
    fn test_extract_bt_mac() {
        let id1 = r"BTHENUM\Dev_1E928008E018&225d8a66&0&BluetoothDevice_1E928008E018";
        assert_eq!(extract_bt_mac(id1), Some("1E928008E018".to_string()));

        let id2 = r"BTHENUM\{0000110b-0000-1000-8000-00805f9b34fb}_LOCALMFG&0002&225d8a66&0&1E928008E018_C00000000";
        assert_eq!(extract_bt_mac(id2), Some("1E928008E018".to_string()));

        let invalid = r"PCI\VEN_8086&DEV_1234";
        assert_eq!(extract_bt_mac(invalid), None);
    }

    #[test]
    fn test_is_virtual_adapter() {
        assert!(is_virtual_adapter("vEthernet (Default Switch)", "Hyper-V Virtual Ethernet"));
        assert!(is_virtual_adapter("vEthernet (WSL)", "Hyper-V Virtual Ethernet"));
        assert!(is_virtual_adapter("Tailscale", "Tailscale Tunnel"));
        assert!(is_virtual_adapter("Ethernet 2", "TAP-Windows Adapter V9"));
        assert!(is_virtual_adapter("DockerNAT", "Hyper-V"));
        assert!(is_virtual_adapter("VMware Network Adapter VMnet1", "VMware Virtual"));

        assert!(!is_virtual_adapter("Ethernet", "Realtek PCIe GbE Family Controller"));
        assert!(!is_virtual_adapter("Ethernet", "Intel(R) Ethernet Connection (2) I219-V"));
        assert!(!is_virtual_adapter("LAN", "Killer E2500 Gigabit Ethernet Controller"));
    }

    #[test]
    fn test_stage_mapped_to_speaker() {
        let ev = build_bluetooth_conn_event("Stage".into(), true, None, true, 100.0);
        match ev.kind {
            ConnKind::Bluetooth { device_kind, .. } => {
                assert_eq!(device_kind, DeviceKind::Speaker, "Stage must be recognized as Speaker");
            }
            _ => panic!("Expected Bluetooth ConnKind"),
        }
    }

    #[test]
    fn test_query_present_bluetooth_devices_runs_without_panic() {
        let devices = query_present_bluetooth_devices();
        println!("query_present_bluetooth_devices found: {:?}", devices);
        // On this machine, Stage should be found:
        let stage = devices.iter().find(|d| d.name.to_lowercase().contains("stage"));
        assert!(stage.is_some(), "Stage Bluetooth device should be found via cfgmgr32 query");
    }
}

#[cfg(not(windows))]
pub struct BluetoothPnpWatcher;
#[cfg(not(windows))]
impl BluetoothPnpWatcher {
    pub fn start(_sink: EventSink) -> Result<Self, String> {
        Ok(Self)
    }
}

#[cfg(not(windows))]
pub struct EthernetWatcher;
#[cfg(not(windows))]
impl EthernetWatcher {
    pub fn start(_sink: EventSink) -> Result<Self, String> {
        Ok(Self)
    }
}

#[cfg(not(windows))]
pub fn is_ethernet_connected() -> bool {
    false
}
