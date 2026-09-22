// SPDX-License-Identifier: MPL-2.0

//! Ownership and private ACL admission on an already pinned native file handle.
use std::{
    fs::File,
    io,
    mem::{offset_of, size_of},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{GENERIC_ALL, LocalFree},
    Security::{Authorization::*, *},
    Storage::FileSystem::FILE_ALL_ACCESS,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0) };
    }
}

fn denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn win32(result: u32) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result as i32))
    }
}

/// TOKEN_USER contains a native pointer, so its backing buffer must be aligned
/// and must remain alive whenever the SID is inspected.
struct User(Vec<usize>);
impl User {
    fn current() -> io::Result<Self> {
        let mut token = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
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
        if !(size_of::<TOKEN_USER>()..=4096).contains(&(length as usize)) {
            return Err(io::Error::other("invalid native token user length"));
        }
        let mut storage = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                storage.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(storage))
    }

    fn sid(&self) -> PSID {
        // GetTokenInformation initialized this aligned buffer and the SID it
        // points to. Moving the Vec leaves that allocation in place.
        unsafe { (*(self.0.as_ptr().cast::<TOKEN_USER>())).User.Sid }
    }
}

fn inspect(file: &File) -> io::Result<(Descriptor, PSID, *mut ACL)> {
    let mut descriptor = ptr::null_mut();
    let mut owner = ptr::null_mut();
    let mut acl = ptr::null_mut();
    win32(unsafe {
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
    })?;
    Ok((Descriptor(descriptor), owner, acl))
}

fn owned_by(owner: PSID, user: &User) -> io::Result<()> {
    if owner.is_null()
        || unsafe { IsValidSid(owner) } == 0
        || unsafe { EqualSid(owner, user.sid()) } == 0
    {
        return Err(denied("runtime storage is not owned by the current user"));
    }
    Ok(())
}

fn private_acl(descriptor: &Descriptor, acl: *mut ACL, user: &User) -> io::Result<()> {
    let invalid = || denied("runtime storage requires a protected owner-only full-access ACL");
    let mut control = 0;
    let mut revision = 0;
    if unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if control & SE_DACL_PROTECTED == 0
        || control & SE_DACL_PRESENT == 0
        || acl.is_null()
        || unsafe { IsValidAcl(acl) } == 0
        || unsafe { (*acl).AceCount } != 1
    {
        return Err(invalid());
    }
    let mut entry = ptr::null_mut();
    if unsafe { GetAce(acl, 0, &mut entry) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // The native ACL is valid, but inspect the ACE header before treating the
    // variable-sized entry as an ACCESS_ALLOWED_ACE.
    let header = unsafe { &*entry.cast::<ACE_HEADER>() };
    if header.AceType != 0 /* ACCESS_ALLOWED_ACE_TYPE */
        || header.AceFlags != 0
        || (header.AceSize as usize) < size_of::<ACCESS_ALLOWED_ACE>()
    {
        return Err(invalid());
    }
    let allowed = unsafe { &*entry.cast::<ACCESS_ALLOWED_ACE>() };
    let sid_offset = offset_of!(ACCESS_ALLOWED_ACE, SidStart);
    let sid_bytes = header.AceSize as usize - sid_offset;
    // A SID has an eight-byte header followed by SubAuthorityCount u32s.
    if sid_bytes < 8 {
        return Err(invalid());
    }
    let sid = unsafe { entry.cast::<u8>().add(sid_offset) };
    let sub_authorities = unsafe { *sid.add(1) };
    if sid_bytes != 8 + usize::from(sub_authorities) * size_of::<u32>()
        || unsafe { IsValidSid(sid.cast()) } == 0
        || (unsafe { EqualSid(sid.cast(), user.sid()) } == 0
            && unsafe { IsWellKnownSid(sid.cast(), WinCreatorOwnerRightsSid) } == 0)
        || !(allowed.Mask & GENERIC_ALL != 0 || allowed.Mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS)
    {
        return Err(invalid());
    }
    Ok(())
}

fn owner_rights_descriptor() -> io::Result<Descriptor> {
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
    Ok(Descriptor(descriptor))
}

/// Requires READ_CONTROL on the handle, and WRITE_DAC when hardening. Never
/// takes ownership. Read-only admission does not modify the security descriptor.
/// The caller separately validates entry kind, reparse points, and link count.
pub(super) fn admit(file: &File, harden: bool) -> io::Result<()> {
    let user = User::current()?;
    let (descriptor, owner, acl) = inspect(file)?;
    owned_by(owner, &user)?;
    if !harden {
        return private_acl(&descriptor, acl, &user);
    }
    let replacement = owner_rights_descriptor()?;
    // SetSecurityInfo propagates removal of inheritable ACEs into existing
    // descendants. Admission must only harden this pinned entry, like chmod
    // on Unix. The native per-object setter avoids that recursive operation.
    // Microsoft documents NtSetSecurityObject as the user-mode entry point:
    // https://learn.microsoft.com/windows-hardware/drivers/ddi/ntifs/nf-ntifs-zwsetsecurityobject
    let status = unsafe {
        windows_sys::Wdk::Storage::FileSystem::NtSetSecurityObject(
            file.as_raw_handle(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            replacement.0,
        )
    };
    if status < 0 {
        return Err(io::Error::from_raw_os_error(unsafe {
            windows_sys::Win32::Foundation::RtlNtStatusToDosError(status)
        } as i32));
    }
    let (descriptor, owner, acl) = inspect(file)?;
    owned_by(owner, &user)?;
    private_acl(&descriptor, acl, &user)
}

#[cfg(test)]
pub(crate) fn test_set_acl(file: &File, text: &str) -> io::Result<()> {
    let text: Vec<_> = text.encode_utf16().chain(Some(0)).collect();
    let mut raw = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            SDDL_REVISION_1,
            &mut raw,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let descriptor = Descriptor(raw);
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = ptr::null_mut();
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    win32(unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl,
            ptr::null_mut(),
        )
    })
}

#[cfg(test)]
pub(super) fn test_security_text(file: &File) -> io::Result<String> {
    let (descriptor, _, _) = inspect(file)?;
    let mut text = ptr::null_mut();
    let mut length = 0;
    if unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor.0,
            SDDL_REVISION_1,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut text,
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let _text = Descriptor(text.cast());
    String::from_utf16(unsafe { std::slice::from_raw_parts(text, length as usize) })
        .map_err(|_| io::Error::other("native security descriptor is not UTF-16"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestRuntimeRoot;
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::READ_CONTROL;

    fn owned_fixture(path: &std::path::Path) -> File {
        // The default creation owner can be a group in an elevated token.
        // Provision the same explicit user ownership as production admission.
        drop(crate::windows_fs::create_private_file(path).unwrap());
        OpenOptions::new()
            .access_mode(FILE_ALL_ACCESS)
            .open(path)
            .unwrap()
    }

    fn set_acl(file: &File, text: &str) {
        let text: Vec<_> = text.encode_utf16().chain(Some(0)).collect();
        let mut raw = ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    text.as_ptr(),
                    SDDL_REVISION_1,
                    &mut raw,
                    ptr::null_mut(),
                )
            },
            0
        );
        let descriptor = Descriptor(raw);
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = ptr::null_mut();
        assert_ne!(
            unsafe {
                GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted)
            },
            0
        );
        win32(unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                acl,
                ptr::null_mut(),
            )
        })
        .unwrap();
    }

    fn security_text(file: &File) -> String {
        let (descriptor, _, _) = inspect(file).unwrap();
        let mut text = ptr::null_mut();
        let mut length = 0;
        assert_ne!(
            unsafe {
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    descriptor.0,
                    SDDL_REVISION_1,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    &mut text,
                    &mut length,
                )
            },
            0
        );
        let _text = Descriptor(text.cast());
        String::from_utf16(unsafe { std::slice::from_raw_parts(text, length as usize) }).unwrap()
    }

    #[test]
    fn hardening_a_directory_preserves_existing_descendant_acls() {
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;

        let root = TestRuntimeRoot::new("private-acl-no-propagation").unwrap();
        let parent = OpenOptions::new()
            .access_mode(FILE_ALL_ACCESS)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(root.path())
            .unwrap();
        // Deliberately establish inherited permissions before admission. The
        // children must retain them when only their parent is made private.
        set_acl(&parent, "D:P(A;OICI;GA;;;OW)");
        std::fs::write(root.join("file"), b"direct").unwrap();
        std::fs::create_dir(root.join("child")).unwrap();
        std::fs::write(root.join("child/nested"), b"nested").unwrap();
        let children: Vec<_> = ["file", "child", "child/nested"]
            .into_iter()
            .map(|name| {
                let file = OpenOptions::new()
                    .access_mode(READ_CONTROL)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                    .open(root.join(name))
                    .unwrap();
                let (descriptor, _, acl) = inspect(&file).unwrap();
                let mut control = 0;
                let mut revision = 0;
                assert_ne!(
                    unsafe {
                        GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision)
                    },
                    0
                );
                assert_eq!(control & SE_DACL_PROTECTED, 0);
                let mut ace = ptr::null_mut();
                assert_ne!(unsafe { GetAce(acl, 0, &mut ace) }, 0);
                assert_ne!(
                    unsafe { (*ace.cast::<ACE_HEADER>()).AceFlags } & INHERITED_ACE as u8,
                    0
                );
                let before = security_text(&file);
                (file, before)
            })
            .collect();
        admit(&parent, true).unwrap();
        admit(&parent, false).unwrap();
        for (file, before) in children {
            assert_eq!(
                security_text(&file),
                before,
                "child ACL was changed by parent admission"
            );
        }
        // Reopen by path to prove permissions still permit access; retained
        // handles alone could conceal a removed inherited grant.
        assert_eq!(std::fs::read(root.join("file")).unwrap(), b"direct");
        assert_eq!(std::fs::read(root.join("child/nested")).unwrap(), b"nested");
    }

    #[test]
    fn readonly_rejects_broad_acl_without_changing_it_and_hardening_repairs_it() {
        let root = TestRuntimeRoot::new("private-acl-hardening").unwrap();
        let path = root.join("file");
        let file = owned_fixture(&path);
        set_acl(&file, "D:P(A;;GA;;;OW)(A;;GR;;;WD)");
        assert_eq!(
            admit(&file, false).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        let (_descriptor, _, acl) = inspect(&file).unwrap();
        assert_eq!(
            unsafe { (*acl).AceCount },
            2,
            "read-only admission changed the ACL"
        );
        admit(&file, true).unwrap();
        let readonly = OpenOptions::new()
            .access_mode(READ_CONTROL)
            .open(&path)
            .unwrap();
        admit(&readonly, false).unwrap();
        assert!(admit(&readonly, true).is_err(), "hardening needs WRITE_DAC");
    }

    #[test]
    fn private_acl_rejects_empty_null_partial_and_inherit_only_permissions() {
        let root = TestRuntimeRoot::new("private-acl-shapes").unwrap();
        let file = owned_fixture(&root.join("file"));
        for text in [
            "D:P",
            "D:NO_ACCESS_CONTROL",
            "D:P(A;;GR;;;OW)",
            "D:P(A;OIIO;GA;;;OW)",
        ] {
            set_acl(&file, text);
            assert!(admit(&file, false).is_err(), "accepted {text}");
        }
        admit(&file, true).unwrap();
        admit(&file, false).unwrap();
    }

    #[test]
    fn readonly_accepts_an_explicit_current_user_full_access_acl() {
        let root = TestRuntimeRoot::new("private-acl-current-user").unwrap();
        let file = owned_fixture(&root.join("file"));
        let user = User::current().unwrap();
        let bytes = size_of::<ACL>()
            + offset_of!(ACCESS_ALLOWED_ACE, SidStart)
            + unsafe { GetLengthSid(user.sid()) } as usize;
        let mut storage = vec![0u32; bytes.div_ceil(size_of::<u32>())];
        let acl = storage.as_mut_ptr().cast();
        assert_ne!(unsafe { InitializeAcl(acl, bytes as u32, ACL_REVISION) }, 0);
        assert_ne!(
            unsafe { AddAccessAllowedAce(acl, ACL_REVISION, FILE_ALL_ACCESS, user.sid()) },
            0
        );
        win32(unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                acl,
                ptr::null_mut(),
            )
        })
        .unwrap();
        admit(&file, false).unwrap();
    }

    #[test]
    fn ownership_check_refuses_a_foreign_sid() {
        let user = User::current().unwrap();
        let mut foreign = [0usize; 9];
        let mut size = size_of_val(&foreign) as u32;
        assert_ne!(
            unsafe {
                CreateWellKnownSid(
                    WinWorldSid,
                    ptr::null_mut(),
                    foreign.as_mut_ptr().cast(),
                    &mut size,
                )
            },
            0
        );
        assert_eq!(
            owned_by(foreign.as_mut_ptr().cast(), &user)
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        owned_by(user.sid(), &user).unwrap();
    }
}
