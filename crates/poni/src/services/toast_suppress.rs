//! Toast notification banner suppression service.
//!
//! Suppresses floating banner toasts from Windows while keeping them
//! intact in Action Center / Notification Center history.
//!
//! Rule 2: Transient state only. Restores Windows notification settings on exit.

use std::sync::atomic::{AtomicBool, Ordering};

const WNF_SHEL_QUIET_HOURS_ACTIVE: u64 = 0xd83063ea3bc3035;

type NtUpdateWnfStateDataFn = unsafe extern "system" fn(
    state_name: *const u64,
    buffer: *const core::ffi::c_void,
    length: u32,
    type_id: *const core::ffi::c_void,
    explicit_scope: *const core::ffi::c_void,
    matching_change_stamp: u32,
    check_stamp: u32,
) -> i32;

type NtQueryWnfStateDataFn = unsafe extern "system" fn(
    state_name: *const u64,
    type_id: *const core::ffi::c_void,
    explicit_scope: *const core::ffi::c_void,
    change_stamp: *mut u32,
    buffer: *mut core::ffi::c_void,
    buffer_size: *mut u32,
) -> i32;

pub struct ToastSuppressService {
    initial_dnd_state: u32,
    active: AtomicBool,
}

impl ToastSuppressService {
    pub fn new(enable: bool) -> Self {
        let initial_dnd_state = query_quiet_hours().unwrap_or(0);
        let svc = Self {
            initial_dnd_state,
            active: AtomicBool::new(false),
        };
        if enable {
            svc.set_suppress(true);
        }
        svc
    }

    pub fn set_suppress(&self, enable: bool) {
        if enable {
            let _ = set_quiet_hours(1);
            self.active.store(true, Ordering::Relaxed);
        } else {
            let _ = set_quiet_hours(self.initial_dnd_state);
            self.active.store(false, Ordering::Relaxed);
        }
    }
}

impl Drop for ToastSuppressService {
    fn drop(&mut self) {
        if self.active.load(Ordering::Relaxed) {
            let _ = set_quiet_hours(self.initial_dnd_state);
        }
    }
}

fn set_quiet_hours(val: u32) -> Result<(), ()> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::w;

    unsafe {
        let ntdll = LoadLibraryW(w!("ntdll.dll")).map_err(|_| ())?;
        let proc = GetProcAddress(ntdll, windows::core::s!("NtUpdateWnfStateData"));
        if let Some(p) = proc {
            let nt_update: NtUpdateWnfStateDataFn = std::mem::transmute(p);
            let state = WNF_SHEL_QUIET_HOURS_ACTIVE;
            let data = val;
            let status = nt_update(
                &state,
                &data as *const _ as *const _,
                std::mem::size_of::<u32>() as u32,
                std::ptr::null(),
                std::ptr::null(),
                0,
                0,
            );
            if status >= 0 {
                return Ok(());
            }
        }
    }
    Err(())
}

fn query_quiet_hours() -> Result<u32, ()> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::w;

    unsafe {
        let ntdll = LoadLibraryW(w!("ntdll.dll")).map_err(|_| ())?;
        let proc = GetProcAddress(ntdll, windows::core::s!("NtQueryWnfStateData"));
        if let Some(p) = proc {
            let nt_query: NtQueryWnfStateDataFn = std::mem::transmute(p);
            let state = WNF_SHEL_QUIET_HOURS_ACTIVE;
            let mut stamp = 0u32;
            let mut data = 0u32;
            let mut size = std::mem::size_of::<u32>() as u32;
            let status = nt_query(
                &state,
                std::ptr::null(),
                std::ptr::null(),
                &mut stamp,
                &mut data as *mut _ as *mut _,
                &mut size,
            );
            if status >= 0 {
                return Ok(data);
            }
        }
    }
    Err(())
}
