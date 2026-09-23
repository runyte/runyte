// SPDX-License-Identifier: MPL-2.0

//! Operating-system account paths used for owner-scoped state.

#[cfg(unix)]
use std::{ffi::CStr, os::unix::ffi::OsStringExt};
use std::{path::PathBuf, sync::OnceLock};

/// The effective operating-system account's home directory.
///
/// `$HOME` may belong to the invoking account rather than the process's
/// effective user, most notably when a command is run through `sudo`. State
/// owned by the effective user must therefore resolve the account database
/// directly instead of trusting inherited environment.
/// On Windows the profile comes from the current account's known-folder record.
pub(crate) fn system_home_directory() -> Option<PathBuf> {
    static HOME: OnceLock<PathBuf> = OnceLock::new();

    if let Some(home) = HOME.get() {
        return Some(home.clone());
    }
    let resolved = resolve_system_home_directory()?;
    // Another caller may have won the race with the same effective account.
    // Retain that answer when it did; otherwise retain this successful one.
    let _ = HOME.set(resolved.clone());
    HOME.get().cloned().or(Some(resolved))
}

#[cfg(unix)]
fn resolve_system_home_directory() -> Option<PathBuf> {
    // SAFETY: `sysconf` reads one process configuration value and has no
    // pointer preconditions.
    let configured = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let mut capacity = if configured > 0 {
        usize::try_from(configured).ok()?
    } else {
        16 * 1024
    }
    .clamp(1024, 1024 * 1024);
    loop {
        let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let mut storage = vec![0_u8; capacity];
        // SAFETY: `record`, `storage`, and `result` are live writable storage;
        // the buffer length matches the allocation and the UID is valid.
        let status = unsafe {
            libc::getpwuid_r(
                libc::geteuid(),
                record.as_mut_ptr(),
                storage.as_mut_ptr().cast::<libc::c_char>(),
                storage.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE && capacity < 1024 * 1024 {
            capacity = (capacity * 2).min(1024 * 1024);
            continue;
        }
        if status != 0 || result.is_null() {
            return None;
        }
        // SAFETY: a successful `getpwuid_r` initialized `record` and returned
        // its address through `result`.
        if unsafe { (*result).pw_dir.is_null() } {
            return None;
        }
        // SAFETY: the successful lookup placed a NUL-terminated directory
        // string inside `storage`, which remains alive for this copy.
        let directory = unsafe { CStr::from_ptr((*result).pw_dir) };
        let path = PathBuf::from(std::ffi::OsString::from_vec(directory.to_bytes().to_vec()));
        return (path.is_absolute() && !path.as_os_str().is_empty()).then_some(path);
    }
}

#[cfg(windows)]
fn resolve_system_home_directory() -> Option<PathBuf> {
    known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_Profile)
}

/// The current account's configured local application-data folder, including
/// account-level redirection. Inherited `LOCALAPPDATA` is not account evidence.
#[cfg(windows)]
pub(crate) fn system_local_app_data_directory() -> Option<PathBuf> {
    static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
    if let Some(directory) = DIRECTORY.get() {
        return Some(directory.clone());
    }
    let resolved = known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_LocalAppData)?;
    let _ = DIRECTORY.set(resolved.clone());
    DIRECTORY.get().cloned().or(Some(resolved))
}

#[cfg(windows)]
fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, ptr};
    use windows_sys::Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
        UI::Shell::SHGetKnownFolderPath,
    };

    struct Apartment(bool);
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: this guard remains on the calling thread and balances
                // exactly one successful CoInitializeEx, including S_FALSE.
                unsafe { CoUninitialize() };
            }
        }
    }
    // SAFETY: COM initialization is scoped to this thread, with no reserved data.
    let initialized = unsafe { CoInitializeEx(ptr::null(), COINIT_MULTITHREADED as u32) };
    let _apartment = match initialized {
        value if value >= 0 => Apartment(true),
        // An existing incompatible apartment still supplies initialized COM;
        // its caller owns the initialization count and apartment choice.
        RPC_E_CHANGED_MODE => Apartment(false),
        _ => return None,
    };

    struct Allocation(*mut u16);
    impl Drop for Allocation {
        fn drop(&mut self) {
            // SAFETY: SHGetKnownFolderPath owns this task-allocator pointer;
            // the API requires freeing it even when the lookup fails.
            unsafe { CoTaskMemFree(self.0.cast()) };
        }
    }

    let mut path = Allocation(ptr::null_mut());
    // SAFETY: id is live, the output pointer is writable, and a null token
    // selects the current user. Flags zero reads the configured location without
    // creating the folder or changing its redirection.
    let status = unsafe { SHGetKnownFolderPath(id, 0, ptr::null_mut(), &mut path.0) };
    if status < 0 || path.0.is_null() {
        return None;
    }
    // SAFETY: a successful lookup returns a NUL-terminated UTF-16 allocation.
    let units = unsafe {
        let mut length = 0;
        while *path.0.add(length) != 0 {
            length += 1;
        }
        std::slice::from_raw_parts(path.0, length)
    };
    let directory = PathBuf::from(OsString::from_wide(units));
    directory.is_absolute().then_some(directory)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{RPC_E_CHANGED_MODE, S_OK},
        System::Com::{
            COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
        },
        UI::Shell::{FOLDERID_LocalAppData, FOLDERID_Profile},
    };

    #[test]
    fn known_folder_lookup_preserves_the_callers_com_apartment_and_initialization_count() {
        for mode in [
            None,
            Some(COINIT_MULTITHREADED),
            Some(COINIT_APARTMENTTHREADED),
        ] {
            std::thread::spawn(move || {
                if let Some(mode) = mode {
                    // SAFETY: this fresh thread owns the initialization below.
                    assert_eq!(
                        unsafe { CoInitializeEx(std::ptr::null(), mode as u32) },
                        S_OK
                    );
                }
                assert!(known_folder(&FOLDERID_Profile).is_some());
                assert!(known_folder(&FOLDERID_LocalAppData).is_some());
                let next = if mode == Some(COINIT_APARTMENTTHREADED) {
                    COINIT_MULTITHREADED
                } else {
                    COINIT_APARTMENTTHREADED
                };
                if mode.is_some() {
                    // SAFETY: an incompatible probe must fail without changing
                    // the caller's apartment or its initialization count.
                    assert_eq!(
                        unsafe { CoInitializeEx(std::ptr::null(), next as u32) },
                        RPC_E_CHANGED_MODE
                    );
                    // SAFETY: balance this test's one successful initialization.
                    unsafe { CoUninitialize() };
                }
                // Successful initialization with the other model proves the
                // lookup left no extra COM initialization behind on this thread.
                assert_eq!(
                    unsafe { CoInitializeEx(std::ptr::null(), next as u32) },
                    S_OK
                );
                // SAFETY: balance the immediately preceding successful call.
                unsafe { CoUninitialize() };
            })
            .join()
            .unwrap();
        }
    }
}
