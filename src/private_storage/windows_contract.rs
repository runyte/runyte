// SPDX-License-Identifier: MPL-2.0

//! Native admission experiments for the Windows storage contract. These do not
//! enable the production storage interface or any deferred service.
use crate::{test_support::TestRuntimeRoot, windows_fs::Identity};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::Path,
    ptr,
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
            FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
        },
    },
    Win32::{
        Foundation::{
            GENERIC_READ, GENERIC_WRITE, OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE,
            RtlNtStatusToDosError, UNICODE_STRING,
        },
        Storage::FileSystem::*,
        System::IO::IO_STATUS_BLOCK,
    },
};

fn directory(path: &Path, write: bool) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(GENERIC_READ | if write { GENERIC_WRITE } else { 0 })
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn relative(parent: &File, name: &str, create: bool) -> io::Result<File> {
    relative_kind(parent, name, create, false)
}
fn relative_kind(parent: &File, name: &str, create: bool, is_directory: bool) -> io::Result<File> {
    if !matches!(
        Path::new(name).components().collect::<Vec<_>>().as_slice(),
        [std::path::Component::Normal(_)]
    ) {
        return Err(io::Error::other("one normal component required"));
    }
    crate::windows_fs::validate_relative(name.as_ref())?;
    let mut units: Vec<_> = std::ffi::OsStr::new(name).encode_wide().collect();
    let length = (units.len() * 2)
        .try_into()
        .map_err(|_| io::Error::other("name too long"))?;
    let mut name = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: units.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &mut name,
        Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        SecurityDescriptor: ptr::null_mut(),
        SecurityQualityOfService: ptr::null_mut(),
    };
    let mut handle = ptr::null_mut();
    let mut status: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_GENERIC_READ | FILE_GENERIC_WRITE,
            &attributes,
            &mut status,
            ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            if create { FILE_CREATE } else { FILE_OPEN },
            (if is_directory {
                FILE_DIRECTORY_FILE
            } else {
                FILE_NON_DIRECTORY_FILE
            }) | FILE_OPEN_REPARSE_POINT
                | FILE_SYNCHRONOUS_IO_NONALERT,
            ptr::null(),
            0,
        )
    };
    if result < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32,
        ));
    }
    let file = unsafe { File::from_raw_handle(handle) };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != is_directory
        || (!is_directory && info.nNumberOfLinks != 1)
    {
        return Err(io::Error::other("requires a regular single-link file"));
    }
    Ok(file)
}

#[test]
fn relative_file_admission_stays_in_the_pinned_directory_after_replacement() {
    let root = TestRuntimeRoot::new("storage-pinned-contract").unwrap();
    let original = root.join("original");
    let moved = root.join("moved");
    fs::create_dir(&original).unwrap();
    let pinned = directory(&original, false).unwrap();
    let identity = Identity::of(&pinned).unwrap();
    fs::rename(&original, &moved).unwrap();
    fs::create_dir(&original).unwrap();
    relative(&pinned, "private", true)
        .unwrap()
        .write_all(b"kept")
        .unwrap();
    assert_eq!(Identity::read(&moved).unwrap(), identity);
    assert_eq!(fs::read(moved.join("private")).unwrap(), b"kept");
    assert!(!original.join("private").exists());
    assert_eq!(
        relative(&pinned, "private", true).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    let mut text = String::new();
    relative(&pinned, "private", false)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(text, "kept");
    for name in [
        "../outside",
        "child\\outside",
        "file:stream",
        "NUL",
        "",
        ".",
        "..",
    ] {
        assert!(relative(&pinned, name, true).is_err(), "{name:?}");
    }
}

#[test]
fn native_admission_rejects_hardlinks_before_any_write() {
    let root = TestRuntimeRoot::new("storage-links-contract").unwrap();
    fs::write(root.join("original"), b"unchanged").unwrap();
    fs::hard_link(root.join("original"), root.join("alias")).unwrap();
    let pinned = directory(root.path(), false).unwrap();
    assert!(relative(&pinned, "alias", false).is_err());
    assert_eq!(fs::read(root.join("original")).unwrap(), b"unchanged");
}

#[test]
fn native_directory_handle_accepts_metadata_flush() {
    let root = TestRuntimeRoot::new("storage-flush-contract").unwrap();
    let pinned = directory(root.path(), true).unwrap();
    let file = relative(&pinned, "data", true).unwrap();
    file.sync_all().unwrap();
    // Exercise the documented flush primitive, not a power-cut simulation.
    pinned.sync_all().unwrap();
}

#[test]
fn native_directory_append_access_accepts_metadata_flush() {
    use windows_sys::Wdk::Storage::FileSystem::NtFlushBuffersFileEx;
    let root = TestRuntimeRoot::new("storage-ancestor-capabilities").unwrap();
    OpenOptions::new()
        .access_mode(FILE_APPEND_DATA | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(root.path())
        .and_then(|file| {
            let mut status: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
            let result = unsafe {
                NtFlushBuffersFileEx(file.as_raw_handle(), 0, ptr::null(), 0, &mut status)
            };
            if result < 0 {
                Err(io::Error::from_raw_os_error(
                    unsafe { RtlNtStatusToDosError(result) } as i32,
                ))
            } else {
                Ok(())
            }
        })
        .unwrap();
}

#[test]
fn native_relative_directory_admission_refuses_junctions() {
    let root = TestRuntimeRoot::new("storage-junction-contract").unwrap();
    let target = root.join("target");
    let link = root.join("redirect");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("kept"), b"unchanged").unwrap();
    // Build the native mount-point fixture directly; cmd's mklink cannot
    // consistently consume extended fixture paths. This requires no symlink
    // privilege or Developer Mode and never starts a test-written program.
    fs::create_dir(&link).unwrap();
    let junction = directory(&link, true).unwrap();
    let target = crate::windows_fs::ordinary_working_directory(&target).unwrap();
    let substitute: Vec<_> = std::ffi::OsString::from(format!("\\??\\{}", target.display()))
        .encode_wide()
        .collect();
    let mut data = Vec::new();
    data.extend_from_slice(&0xA0000003u32.to_le_bytes()); // IO_REPARSE_TAG_MOUNT_POINT
    data.extend_from_slice(&((8 + (substitute.len() + 2) * 2) as u16).to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes()); // substitute offset
    data.extend_from_slice(&((substitute.len() * 2) as u16).to_le_bytes());
    data.extend_from_slice(&(((substitute.len() + 1) * 2) as u16).to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes()); // empty print name
    for unit in substitute {
        data.extend_from_slice(&unit.to_le_bytes());
    }
    data.extend_from_slice(&[0; 4]);
    let mut returned = 0;
    assert_ne!(
        unsafe {
            windows_sys::Win32::System::IO::DeviceIoControl(
                junction.as_raw_handle(),
                0x000900A4,
                /* FSCTL_SET_REPARSE_POINT */ data.as_ptr().cast(),
                data.len() as u32,
                ptr::null_mut(),
                0,
                &mut returned,
                ptr::null_mut(),
            )
        },
        0,
        "{}",
        io::Error::last_os_error()
    );
    drop(junction);
    let pinned = directory(root.path(), false).unwrap();
    assert!(relative_kind(&pinned, "redirect", false, true).is_err());
    assert!(super::Directory::open(&link, true).is_err());
    assert!(super::Directory::open(&link.join("new"), true).is_err());
    assert!(!target.join("new").exists());
    let storage = super::Directory::open(root.path(), true).unwrap();
    assert!(storage.child(std::ffi::OsStr::new("redirect")).is_err());
    assert!(relative_kind(&pinned, "target", false, true).is_ok());
    assert_eq!(fs::read(target.join("kept")).unwrap(), b"unchanged");
    fs::remove_dir(&link).unwrap();
}

#[test]
fn private_creation_uses_an_owner_only_protected_acl() {
    use windows_sys::Win32::{
        Foundation::{GENERIC_ALL, LocalFree},
        Security::{Authorization::*, *},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    struct SecurityDescriptor(*mut std::ffi::c_void);
    impl Drop for SecurityDescriptor {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    let root = TestRuntimeRoot::new("storage-acl-contract").unwrap();
    let file = crate::windows_fs::create_private_file(&root.join("private")).unwrap();
    let mut owner = ptr::null_mut();
    let mut acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    assert_eq!(
        unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                &mut acl,
                ptr::null_mut(),
                &mut descriptor,
            )
        },
        0
    );
    let _descriptor = SecurityDescriptor(descriptor);
    let mut control = 0;
    let mut revision = 0;
    assert_ne!(
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) },
        0
    );
    assert_ne!(control & SE_DACL_PROTECTED, 0);
    assert!(!acl.is_null());
    assert_eq!(unsafe { (*acl).AceCount }, 1);
    let mut ace = ptr::null_mut();
    assert_ne!(unsafe { GetAce(acl, 0, &mut ace) }, 0);
    let ace = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
    assert_eq!(ace.Header.AceType, 0 /* ACCESS_ALLOWED_ACE_TYPE */);
    assert_eq!(ace.Header.AceFlags & INHERIT_ONLY_ACE as u8, 0);
    assert!(ace.Mask & GENERIC_ALL != 0 || ace.Mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS);
    let mut expected = [0usize; 9];
    let mut sid_size = size_of_val(&expected) as u32;
    assert_ne!(
        unsafe {
            CreateWellKnownSid(
                WinCreatorOwnerRightsSid,
                ptr::null_mut(),
                expected.as_mut_ptr().cast(),
                &mut sid_size,
            )
        },
        0
    );
    assert_ne!(
        unsafe {
            EqualSid(
                (&ace.SidStart as *const u32).cast_mut().cast(),
                expected.as_mut_ptr().cast(),
            )
        },
        0
    );
    let mut token = ptr::null_mut();
    assert_ne!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) },
        0
    );
    let token = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(token) };
    let mut length = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut length,
        );
    }
    assert!((1..=4096).contains(&length));
    let mut user = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    let obtained = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            user.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    };
    assert_ne!(obtained, 0);
    let user = unsafe { &*(user.as_ptr() as *const TOKEN_USER) };
    assert_ne!(
        unsafe { EqualSid(owner, user.User.Sid) },
        0,
        "private file owner must be the current user"
    );
}
