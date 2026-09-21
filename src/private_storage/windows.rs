// SPDX-License-Identifier: MPL-2.0

//! Pinned, component-relative local NTFS runtime storage. No operation below
//! resolves a leaf through the original (possibly replaced) directory path.
use super::windows_security;
use crate::windows_fs::{self, Identity};
use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    mem::{offset_of, size_of},
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Component, Path, Prefix},
    ptr,
    sync::atomic::{AtomicU64, Ordering},
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
            FILE_OPEN_REPARSE_POINT, FILE_RENAME_INFORMATION, FILE_RENAME_POSIX_SEMANTICS,
            FILE_RENAME_REPLACE_IF_EXISTS, FILE_SYNCHRONOUS_IO_NONALERT, FILE_WRITE_THROUGH,
            FileFsDeviceInformation, FileRenameInformationEx, NtCreateFile,
            NtQueryVolumeInformationFile, NtSetInformationFile,
        },
    },
    Win32::{
        Foundation::{
            OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
        },
        Storage::FileSystem::*,
        System::IO::IO_STATUS_BLOCK,
    },
};

#[derive(Debug)]
pub struct Directory(File);

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn leaf(name: &OsStr) -> io::Result<Vec<u16>> {
    if !matches!(
        Path::new(name).components().collect::<Vec<_>>().as_slice(),
        [Component::Normal(_)]
    ) {
        return Err(invalid("storage name must be one normal path component"));
    }
    windows_fs::validate_relative(Path::new(name))?;
    let units: Vec<_> = name.encode_wide().collect();
    if units.len() > 255 {
        return Err(invalid("storage name exceeds the NTFS component limit"));
    }
    Ok(units)
}

pub(super) fn regular(file: &File, directory: bool) -> io::Result<()> {
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
        || (!directory && info.nNumberOfLinks != 1)
    {
        return Err(io::Error::other(
            "runtime storage requires a non-reparse directory or single-link regular file",
        ));
    }
    Ok(())
}

fn relative(
    parent: &File,
    name: &OsStr,
    access: u32,
    create: bool,
    directory: bool,
) -> io::Result<File> {
    let mut units = leaf(name)?;
    let mut name = UNICODE_STRING {
        Length: (units.len() * 2) as u16,
        MaximumLength: (units.len() * 2) as u16,
        Buffer: units.as_mut_ptr(),
    };
    windows_fs::with_private_security(|security| {
        let attributes = OBJECT_ATTRIBUTES {
            Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: parent.as_raw_handle(),
            ObjectName: &mut name,
            Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
            SecurityDescriptor: if create {
                security.lpSecurityDescriptor.cast()
            } else {
                ptr::null_mut()
            },
            SecurityQualityOfService: ptr::null_mut(),
        };
        let mut handle = ptr::null_mut();
        let mut status: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
        let result = unsafe {
            NtCreateFile(
                &mut handle,
                access | SYNCHRONIZE | if create { DELETE } else { 0 },
                &attributes,
                &mut status,
                ptr::null(),
                FILE_ATTRIBUTE_NORMAL,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                if create { FILE_CREATE } else { FILE_OPEN },
                (if directory {
                    FILE_DIRECTORY_FILE
                } else {
                    FILE_NON_DIRECTORY_FILE
                }) | FILE_OPEN_REPARSE_POINT
                    | FILE_SYNCHRONOUS_IO_NONALERT
                    | FILE_WRITE_THROUGH,
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
        if let Err(error) = regular(&file, directory) {
            if create {
                let _ = delete(&file);
            }
            return Err(error);
        }
        Ok(file)
    })
}

fn ntfs(file: &File) -> io::Result<()> {
    // FILE_FS_DEVICE_INFORMATION contains two ULONGs. Query the pinned handle,
    // so a mapped drive cannot bypass locality by reporting the server's NTFS.
    let mut device = [0u32; 2];
    let mut status: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
    let result = unsafe {
        NtQueryVolumeInformationFile(
            file.as_raw_handle(),
            &mut status,
            device.as_mut_ptr().cast(),
            size_of_val(&device) as u32,
            FileFsDeviceInformation,
        )
    };
    nt_result(result)?;
    local_device(device[1])?;
    let mut flags = 0;
    let mut name = [0u16; 32];
    if unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut flags,
            name.as_mut_ptr(),
            name.len() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if name[..5] != [b'N' as u16, b'T' as u16, b'F' as u16, b'S' as u16, 0]
        || flags & 0x00000008 /* FILE_PERSISTENT_ACLS */ == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private runtime storage requires local NTFS with persistent ACLs",
        ));
    }
    Ok(())
}

fn nt_result(status: i32) -> io::Result<()> {
    if status < 0 {
        Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ))
    } else {
        Ok(())
    }
}
fn local_device(characteristics: u32) -> io::Result<()> {
    if characteristics & 0x10 /* FILE_REMOTE_DEVICE */ != 0 {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private runtime storage does not support remote volumes",
        ))
    } else {
        Ok(())
    }
}

fn reopen(file: &File, access: u32) -> io::Result<File> {
    reopen_kind(file, access, true)
}

fn reopen_kind(file: &File, access: u32, directory: bool) -> io::Result<File> {
    let mut name = UNICODE_STRING {
        Length: 0,
        MaximumLength: 0,
        Buffer: ptr::null_mut(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: file.as_raw_handle(),
        ObjectName: &mut name,
        Attributes: OBJ_DONT_REPARSE,
        SecurityDescriptor: ptr::null_mut(),
        SecurityQualityOfService: ptr::null_mut(),
    };
    let mut handle = ptr::null_mut();
    let mut status: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
    nt_result(unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &attributes,
            &mut status,
            ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            (if directory {
                FILE_DIRECTORY_FILE
            } else {
                FILE_NON_DIRECTORY_FILE
            }) | FILE_OPEN_REPARSE_POINT
                | FILE_SYNCHRONOUS_IO_NONALERT
                | FILE_WRITE_THROUGH,
            ptr::null(),
            0,
        )
    })?;
    Ok(unsafe { File::from_raw_handle(handle) })
}

/// Truncate the admitted file without replacing the append handle that owns
/// its logging lock. Native reopening uses the object itself, never its name.
pub(crate) fn truncate(file: &File) -> io::Result<()> {
    regular(file, false)?;
    windows_security::admit(file, false)?;
    let writable = reopen_kind(
        file,
        FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | READ_CONTROL,
        false,
    )?;
    regular(&writable, false)?;
    writable.set_len(0)
}

#[allow(dead_code)] // Some callers remain disabled until their native integration is ready.
impl Directory {
    pub fn open(path: &Path, private: bool) -> io::Result<Self> {
        Self::open_inner(path, private, false, true)
    }
    pub(crate) fn open_durable(path: &Path, private: bool) -> io::Result<Self> {
        Self::open_inner(path, private, true, true)
    }
    pub(crate) fn open_existing(path: &Path, private: bool) -> io::Result<Self> {
        Self::open_inner(path, private, false, false)
    }

    /// The caller supplies an independently provisioned, already durable
    /// anchor (for example an OS-created user directory). This commits only
    /// the relative subtree, never the anchor's ancestry. Do not infer this
    /// precondition from the nearest existing directory after a failed write.
    pub(crate) fn open_durable_beneath(anchor: &Path, relative: &Path) -> io::Result<Self> {
        let names: Vec<_> = relative
            .components()
            .map(|component| match component {
                Component::Normal(name) => {
                    leaf(name)?;
                    Ok(name)
                }
                _ => Err(invalid(
                    "durable subtree requires normal relative components",
                )),
            })
            .collect::<io::Result<_>>()?;
        if names.is_empty() {
            return Err(invalid("durable subtree cannot replace its anchor"));
        }
        let mut directory = Self::open_existing(anchor, false)?;
        directory.sync()?;
        for name in names {
            directory = directory.child(name)?;
        }
        Ok(directory)
    }

    fn open_inner(path: &Path, private: bool, durable: bool, create: bool) -> io::Result<Self> {
        Self::open_with_flush(path, private, durable, create, flush_directory)
    }

    fn open_with_flush(
        path: &Path,
        private: bool,
        durable: bool,
        create: bool,
        mut flush: impl FnMut(&File) -> io::Result<()>,
    ) -> io::Result<Self> {
        let absolute = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut components = absolute.components();
        let drive = match components.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
                _ => return Err(invalid("private storage requires a local drive path")),
            },
            _ => return Err(invalid("private storage requires an absolute path")),
        };
        if components.next() != Some(Component::RootDir) {
            return Err(invalid("drive-relative storage path"));
        }
        let root = format!("\\\\?\\{}:\\", drive as char);
        let mut directory = Self(
            OpenOptions::new()
                .access_mode(FILE_GENERIC_READ)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(root)?,
        );
        regular(&directory.0, true)?;
        ntfs(&directory.0)?;
        let names: Vec<_> = components.filter(|c| *c != Component::CurDir).collect();
        for (index, component) in names.iter().enumerate() {
            let Component::Normal(name) = component else {
                return Err(invalid("storage paths cannot traverse parent components"));
            };
            let last = index + 1 == names.len();
            // Retry every ancestor, including entries left by an interrupted
            // creation. Refuse missing flush rights before creating anything.
            if durable {
                flush(&directory.0)?;
            }
            let access = FILE_GENERIC_READ
                | if last && create {
                    FILE_GENERIC_WRITE | if private { WRITE_DAC } else { 0 }
                } else {
                    0
                };
            let next = match relative(&directory.0, name, access, false, true) {
                Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                    match relative(
                        &directory.0,
                        name,
                        FILE_GENERIC_READ | FILE_GENERIC_WRITE | WRITE_DAC,
                        true,
                        true,
                    ) {
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            relative(&directory.0, name, access, false, true)?
                        }
                        result => result?,
                    }
                }
                result => result?,
            };
            if durable {
                flush(&next)?;
                flush(&directory.0)?;
            }
            directory = Self(next);
        }
        if private {
            windows_security::admit(&directory.0, create)?;
        }
        if durable {
            flush(&directory.0)?;
        }
        // Retain only read access between operations; acquire the flush
        // capability temporarily without resolving the directory path again.
        Ok(Self(reopen(&directory.0, FILE_GENERIC_READ)?))
    }

    pub(crate) fn child(&self, name: &OsStr) -> io::Result<Self> {
        leaf(name)?;
        self.sync()?;
        let access = FILE_GENERIC_READ | FILE_GENERIC_WRITE | WRITE_DAC;
        let file = match relative(&self.0, name, access, true, true) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                relative(&self.0, name, access, false, true)?
            }
            result => result?,
        };
        windows_security::admit(&file, true)?;
        file.sync_all()?;
        self.sync()?;
        Ok(Self(reopen(&file, FILE_GENERIC_READ)?))
    }

    fn file(&self, name: &OsStr, access: u32, create: bool, harden: bool) -> io::Result<File> {
        self.file_with_admission(name, access, create, harden, windows_security::admit)
    }
    fn file_with_admission(
        &self,
        name: &OsStr,
        access: u32,
        create: bool,
        harden: bool,
        admit: impl FnOnce(&File, bool) -> io::Result<()>,
    ) -> io::Result<File> {
        let file = relative(
            &self.0,
            name,
            access | READ_CONTROL | if harden { WRITE_DAC } else { 0 },
            create,
            false,
        )?;
        if let Err(error) = admit(&file, harden) {
            if create {
                let _ = delete(&file);
            }
            return Err(error);
        }
        Ok(file)
    }
    pub fn append(&self, name: &OsStr) -> io::Result<File> {
        // FILE_APPEND_DATA without FILE_WRITE_DATA gives each write an atomic
        // EOF position across independently opened handles, including std::File.
        let access = FILE_GENERIC_READ | (FILE_GENERIC_WRITE & !FILE_WRITE_DATA);
        match self.file(name, access, true, true) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                self.file(name, access, false, true)
            }
            result => result,
        }
    }
    pub fn create_new(&self, name: &OsStr) -> io::Result<File> {
        self.file(
            name,
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE,
            true,
            true,
        )
    }
    pub fn open_read(&self, name: &OsStr) -> io::Result<File> {
        self.file(name, FILE_GENERIC_READ, false, false)
    }
    pub fn read(&self, name: &OsStr, limit: usize) -> io::Result<Vec<u8>> {
        let file = self.file(name, FILE_GENERIC_READ, false, true)?;
        let mut bytes = Vec::new();
        file.take((limit as u64).saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(io::Error::other(
                "runtime storage file exceeds its size limit",
            ));
        }
        Ok(bytes)
    }
    pub fn remove_owned(&self, name: &OsStr, expected: &File) -> io::Result<()> {
        let current = self.file(name, FILE_GENERIC_READ | DELETE, false, false)?;
        if Identity::of(&current)? == Identity::of(expected)? {
            delete(&current)?;
        }
        Ok(())
    }
    pub fn remove(&self, name: &OsStr) -> io::Result<()> {
        delete(&self.file(name, FILE_GENERIC_READ | DELETE, false, false)?)
    }
    pub(crate) fn rename(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
        let file = self.file(
            from,
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE,
            false,
            false,
        )?;
        self.rename_file(&file, to)
    }
    fn rename_file(&self, file: &File, to: &OsStr) -> io::Result<()> {
        let name = leaf(to)?;
        let size = (offset_of!(FILE_RENAME_INFORMATION, FileName) + name.len() * 2)
            .max(size_of::<FILE_RENAME_INFORMATION>());
        let mut buffer = vec![0usize; size.div_ceil(size_of::<usize>())];
        let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
        unsafe {
            (*info).Anonymous.Flags = FILE_RENAME_REPLACE_IF_EXISTS | FILE_RENAME_POSIX_SEMANTICS;
            (*info).RootDirectory = self.0.as_raw_handle();
            (*info).FileNameLength = (name.len() * 2) as u32;
            ptr::copy_nonoverlapping(
                name.as_ptr(),
                ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
                name.len(),
            );
            let mut status: IO_STATUS_BLOCK = std::mem::zeroed();
            nt_result(NtSetInformationFile(
                file.as_raw_handle(),
                &mut status,
                info.cast(),
                size as u32,
                FileRenameInformationEx,
            ))?;
        }
        file.sync_all()
    }
    pub fn sync(&self) -> io::Result<()> {
        flush_directory(&self.0)
    }
    pub fn atomic_write(&self, name: &OsStr, bytes: &[u8]) -> io::Result<()> {
        leaf(name)?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..64 {
            let pending = format!(
                ".runyte-write-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let mut file = match self.create_new(OsStr::new(&pending)) {
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                result => result?,
            };
            let result = (|| {
                file.write_all(bytes)?;
                file.sync_all()?;
                self.rename_file(&file, name)?;
                self.sync()
            })();
            if result.is_err() {
                let _ = self.remove_owned(OsStr::new(&pending), &file);
            }
            return result;
        }
        Err(io::Error::other(
            "cannot create a unique runtime storage file",
        ))
    }
}

fn flush_directory(file: &File) -> io::Result<()> {
    // Native flush accepts write OR append access. The latter is enough for
    // directory metadata and does not request arbitrary child-file creation.
    let file = reopen(file, FILE_APPEND_DATA)?;
    let mut status: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
    nt_result(unsafe {
        windows_sys::Wdk::Storage::FileSystem::NtFlushBuffersFileEx(
            file.as_raw_handle(),
            0,
            ptr::null(),
            0,
            &mut status,
        )
    })
}

fn delete(file: &File) -> io::Result<()> {
    let info = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    };
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfoEx,
            (&info as *const FILE_DISPOSITION_INFO_EX).cast(),
            size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestRuntimeRoot;
    use std::{fs, sync::Barrier};

    #[test]
    fn exclusive_append_and_bounded_read_keep_every_writer() {
        let root = TestRuntimeRoot::new("native-storage-writers").unwrap();
        let directory = Directory::open(root.path(), true).unwrap();
        let name = OsStr::new("log");
        for _ in 0..16 {
            let barrier = Barrier::new(8);
            std::thread::scope(|scope| {
                for index in 0..8 {
                    let (directory, barrier) = (&directory, &barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        directory
                            .append(name)
                            .unwrap()
                            .write_all(&[b'0' + index; 32])
                            .unwrap();
                    });
                }
            });
        }
        let bytes = directory.read(name, 4096).unwrap();
        assert_eq!(bytes.len(), 4096);
        assert!(
            bytes
                .chunks_exact(32)
                .all(|chunk| chunk.iter().all(|b| *b == chunk[0]))
        );
        for value in b'0'..=b'7' {
            assert_eq!(bytes.iter().filter(|b| **b == value).count(), 512);
        }
        assert!(directory.read(name, 4095).is_err());
        assert_eq!(
            directory.create_new(name).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
    }

    #[test]
    fn replacement_and_cleanup_stay_with_the_issued_identity() {
        let root = TestRuntimeRoot::new("native-storage-identity").unwrap();
        let original = root.join("original");
        let moved = root.join("moved");
        let directory = Directory::open(&original, true).unwrap();
        fs::rename(&original, &moved).unwrap();
        fs::create_dir(&original).unwrap();
        let issued = directory.create_new(OsStr::new("issued")).unwrap();
        directory.atomic_write(OsStr::new("data"), b"one").unwrap();
        let mut reader = directory.open_read(OsStr::new("data")).unwrap();
        directory.atomic_write(OsStr::new("data"), b"two").unwrap();
        let mut old = Vec::new();
        reader.read_to_end(&mut old).unwrap();
        assert_eq!(old, b"one");
        assert_eq!(fs::read(moved.join("data")).unwrap(), b"two");
        assert!(!original.join("data").exists());
        directory
            .rename(OsStr::new("issued"), OsStr::new("old"))
            .unwrap();
        directory
            .create_new(OsStr::new("issued"))
            .unwrap()
            .write_all(b"replacement")
            .unwrap();
        directory
            .remove_owned(OsStr::new("issued"), &issued)
            .unwrap();
        assert_eq!(
            directory.read(OsStr::new("issued"), 64).unwrap(),
            b"replacement"
        );
        directory.remove_owned(OsStr::new("old"), &issued).unwrap();
        assert!(!moved.join("old").exists());
        directory.remove(OsStr::new("data")).unwrap();
        directory.sync().unwrap();
        assert!(!moved.join("data").exists());
    }

    #[test]
    fn readonly_admission_never_creates_or_hardens_and_links_are_refused() {
        let root = TestRuntimeRoot::new("native-storage-admission").unwrap();
        let missing = root.join("missing");
        assert_eq!(
            Directory::open_existing(&missing, true).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(!missing.exists());
        let directory = Directory::open(&missing, true).unwrap();
        Directory::open_existing(&missing, true).unwrap();
        directory
            .create_new(OsStr::new("file"))
            .unwrap()
            .write_all(b"unchanged")
            .unwrap();
        fs::hard_link(missing.join("file"), missing.join("alias")).unwrap();
        assert!(directory.append(OsStr::new("alias")).is_err());
        assert!(directory.open_read(OsStr::new("alias")).is_err());
        assert_eq!(fs::read(missing.join("file")).unwrap(), b"unchanged");
        for name in ["../escape", "NUL", "file:stream", "", ".", ".."] {
            assert!(directory.create_new(OsStr::new(name)).is_err());
        }
        let child = directory.child(OsStr::new("child")).unwrap();
        child.atomic_write(OsStr::new("record"), b"data").unwrap();
    }

    #[test]
    fn owned_file_verifies_public_path_and_cleans_only_original() {
        let root = TestRuntimeRoot::new("native-owned-file").unwrap();
        let storage = root.join("storage");
        let owned = crate::private_storage::OwnedFile::create(&storage, "issued").unwrap();
        owned.file().write_all(b"private").unwrap();
        owned.open_read().unwrap();
        let name = owned.path().file_name().unwrap().to_owned();
        let moved = root.join("moved");
        // Native directory rename can be refused while a child is open. Use a
        // file-name replacement here; the directory test covers pinned parents.
        fs::create_dir(&moved).unwrap();
        fs::rename(owned.path(), moved.join(&name)).unwrap();
        let replacement = Directory::open(&storage, true).unwrap();
        replacement
            .create_new(&name)
            .unwrap()
            .write_all(b"keep")
            .unwrap();
        assert!(owned.verify_path().is_err());
        assert!(owned.open_read().is_err());
        drop(owned);
        let sentinel = crate::private_storage::OwnedFile::create(&storage, "sentinel").unwrap();
        let sentinel_path = sentinel.path().to_owned();
        drop(sentinel);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        // Cleanup must not remove either a replacement or the issued file once
        // another operation has moved it outside its original directory/name.
        while sentinel_path.exists() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(moved.join(&name).exists());
        assert!(!sentinel_path.exists());
        assert_eq!(fs::read(storage.join(&name)).unwrap(), b"keep");
    }

    #[test]
    fn durable_retry_flushes_the_parent_of_an_existing_entry() {
        let root = TestRuntimeRoot::new("native-storage-durable-retry").unwrap();
        let root_id = Identity::read(root.path()).unwrap();
        let leaf = root.join("created");
        let result = Directory::open_with_flush(&leaf, true, true, true, |file| {
            if Identity::of(file)? == root_id && leaf.is_dir() {
                return Err(io::Error::other("injected post-create flush failure"));
            }
            Ok(())
        });
        assert!(result.is_err());
        assert!(leaf.is_dir());
        let mut retried = false;
        Directory::open_with_flush(&leaf, true, true, true, |file| {
            if Identity::of(file)? == root_id {
                retried = true;
            }
            Ok(())
        })
        .unwrap();
        assert!(retried);
        assert_eq!(
            local_device(0x10).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        local_device(0).unwrap();
    }

    #[test]
    fn failed_admission_removes_only_exclusive_creations() {
        let root = TestRuntimeRoot::new("native-storage-admission-cleanup").unwrap();
        let directory = Directory::open(root.path(), true).unwrap();
        let name = OsStr::new("file");
        let failure = |_: &File, _: bool| Err(io::Error::other("injected admission failure"));
        assert!(
            directory
                .file_with_admission(
                    name,
                    FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                    true,
                    true,
                    failure
                )
                .is_err()
        );
        assert!(!root.join(name).exists());
        directory
            .create_new(name)
            .unwrap()
            .write_all(b"keep")
            .unwrap();
        assert!(
            directory
                .file_with_admission(
                    name,
                    FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                    false,
                    true,
                    failure
                )
                .is_err()
        );
        assert_eq!(fs::read(root.join(name)).unwrap(), b"keep");
    }

    #[test]
    fn durable_subtree_preserves_the_provisioned_anchor() {
        let root = TestRuntimeRoot::new("native-storage-durable-subtree").unwrap();
        let before = Identity::read(root.path()).unwrap();
        let leaf = Directory::open_durable_beneath(root.path(), Path::new("private/leaf")).unwrap();
        leaf.atomic_write(OsStr::new("record"), b"kept").unwrap();
        let reopened =
            Directory::open_durable_beneath(root.path(), Path::new("private/leaf")).unwrap();
        assert_eq!(reopened.read(OsStr::new("record"), 4).unwrap(), b"kept");
        assert_eq!(Identity::read(root.path()).unwrap(), before);
        assert!(
            Directory::open_durable_beneath(&root.join("missing"), Path::new("child")).is_err()
        );
        assert!(!root.join("missing").exists());
        assert!(
            Directory::open_durable_beneath(root.path(), Path::new("uncreated/../escape")).is_err()
        );
        assert!(!root.join("uncreated").exists());
    }
}
