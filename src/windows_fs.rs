// SPDX-License-Identifier: MPL-2.0

//! Native file identity and private creation shared by saves and staging.
use std::{
    fs::{File, OpenOptions},
    io,
    mem::size_of,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::*,
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Identity {
    pub(crate) volume: u64,
    file: [u8; 16],
}

pub(crate) fn compare_names(left: &[u16], right: &[u16]) -> std::cmp::Ordering {
    use windows_sys::Win32::Globalization::CompareStringOrdinal;
    // Explorer input is bounded by the native path length before sorting.
    match unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
            1,
        )
    } {
        1 => std::cmp::Ordering::Less,
        3 => std::cmp::Ordering::Greater,
        2 => std::cmp::Ordering::Equal,
        _ => left.cmp(right),
    }
}

/// The opened directory entry, rather than the underlying file identity (which
/// hardlinks share). Opening reparse points themselves preserves link names.
pub(crate) fn entry_path(path: &Path) -> io::Result<PathBuf> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut units = vec![0; 32768];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            units.as_mut_ptr(),
            units.len() as u32,
            FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
        )
    } as usize;
    if length == 0 {
        return Err(io::Error::last_os_error());
    }
    if length >= units.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is too long",
        ));
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &units[..length],
    )))
}

/// Resolve existing ancestors before comparing proposed paths. This covers
/// native case and short-name aliases even when the destination is not present.
pub(crate) fn resolved_proposal(path: &Path) -> io::Result<PathBuf> {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    loop {
        match entry_path(ancestor) {
            Ok(mut resolved) => {
                for component in suffix.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(name) = ancestor.file_name() else {
                    return Err(error);
                };
                suffix.push(name);
                ancestor = ancestor.parent().ok_or(error)?;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn contains_proposal(
    source: &Path,
    target: &Path,
    allow_same: bool,
) -> io::Result<bool> {
    let source = resolved_proposal(source)?;
    let target = resolved_proposal(target)?;
    let source: Vec<_> = source.components().collect();
    let target: Vec<_> = target.components().collect();
    Ok(target.len() >= source.len()
        && (!allow_same || target.len() != source.len())
        && source.iter().zip(&target).all(|(a, b)| {
            compare_names(
                &a.as_os_str().encode_wide().collect::<Vec<_>>(),
                &b.as_os_str().encode_wide().collect::<Vec<_>>(),
            )
            .is_eq()
        }))
}

pub(crate) fn validate_relative(path: &Path) -> io::Result<()> {
    use std::path::Component;
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Windows directory entry name",
        )
    };
    if path.as_os_str().encode_wide().count() > 32_000 {
        return Err(invalid());
    }
    for component in path.components() {
        let name = match component {
            Component::Normal(name) => name.to_str().ok_or_else(invalid)?,
            Component::CurDir | Component::ParentDir => continue,
            Component::RootDir | Component::Prefix(_) => return Err(invalid()),
        };
        if name.ends_with([' ', '.'])
            || name
                .chars()
                .any(|ch| ch.is_control() || "<>:\"|?*".contains(ch))
        {
            return Err(invalid());
        }
        let stem = name
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end()
            .to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(
                &stem[3..],
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        {
            return Err(invalid());
        }
    }
    Ok(())
}
impl Identity {
    pub(crate) fn of(file: &File) -> io::Result<Self> {
        let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&mut info as *mut FILE_ID_INFO).cast(),
                size_of::<FILE_ID_INFO>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            volume: info.VolumeSerialNumber,
            file: info.FileId.Identifier,
        })
    }
    pub(crate) fn read(path: &Path) -> io::Result<Self> {
        // Observe the reparse entry itself, including directory junctions.
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        Self::of(&file)
    }
}

pub(crate) fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut units: Vec<_> = path.as_os_str().encode_wide().collect();
    if units.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    units.push(0);
    Ok(units)
}

struct Local(*mut std::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn with_private_security<T>(
    operation: impl FnOnce(&SECURITY_ATTRIBUTES) -> io::Result<T>,
) -> io::Result<T> {
    let text: Vec<_> = "D:P(A;;GA;;;OW)\0".encode_utf16().collect();
    let mut descriptor = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let _descriptor = Local(descriptor);
    operation(&SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    })
}

pub(crate) fn create_private_file(path: &Path) -> io::Result<File> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    let path = wide(path)?;
    with_private_security(|security| {
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                security,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_handle(handle) })
        }
    })
}

pub(crate) fn create_private_directory(path: &Path) -> io::Result<()> {
    let path = wide(path)?;
    with_private_security(|security| {
        if unsafe { CreateDirectoryW(path.as_ptr(), security) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    })
}

/// Verify an ordinary current-directory spelling before native process startup.
pub(crate) fn ordinary_working_directory(path: &Path) -> io::Result<PathBuf> {
    use std::{
        ffi::OsString,
        path::{Component, Prefix},
    };
    let canonical = path.canonicalize()?;
    let unsupported = || {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "process directory requires an extended Windows path; use a directory with an ordinary path shorter than 260 UTF-16 units",
        )
    };
    let mut components = canonical.components();
    let mut ordinary = match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:", drive as char)),
            Prefix::VerbatimUNC(server, share) => {
                let mut path = OsString::from(r"\\");
                path.push(server);
                path.push(r"\");
                path.push(share);
                PathBuf::from(path)
            }
            _ => return Err(unsupported()),
        },
        _ => return Err(unsupported()),
    };
    for component in components {
        if let Component::Normal(name) = component {
            crate::windows_fs::validate_relative(Path::new(name)).map_err(|_| unsupported())?;
        }
        ordinary.push(component.as_os_str());
    }
    if ordinary.as_os_str().encode_wide().count() >= 260
        || !ordinary.is_dir()
        || crate::windows_fs::Identity::read(&ordinary)?
            != crate::windows_fs::Identity::read(&canonical)?
    {
        return Err(unsupported());
    }
    Ok(ordinary)
}
