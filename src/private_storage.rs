// SPDX-License-Identifier: MPL-2.0

//! Descriptor-relative storage for private runtime files. Paths supplied by a
//! workspace must never turn a cache or log write into a write through a link.

use std::{fs::File, io, path::Path};

#[cfg(unix)]
mod platform {
    use super::*;
    use std::{
        ffi::{CString, OsStr},
        io::{Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{
                ffi::OsStrExt,
                fs::{MetadataExt, PermissionsExt},
            },
        },
        path::{Component, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    #[derive(Debug)]
    pub struct Directory(File);

    fn cstring(value: &OsStr) -> io::Result<CString> {
        CString::new(value.as_bytes()).map_err(|_| io::Error::other("storage path contains NUL"))
    }

    fn leaf(name: &OsStr) -> io::Result<CString> {
        if name.is_empty()
            || Path::new(name).components().count() != 1
            || !matches!(
                Path::new(name).components().next(),
                Some(Component::Normal(_))
            )
        {
            return Err(io::Error::other(
                "storage name must be one normal path component",
            ));
        }
        cstring(name)
    }

    fn owned_regular(file: &File) -> io::Result<()> {
        let metadata = file.metadata()?;
        // SAFETY: geteuid has no preconditions.
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
        {
            return Err(io::Error::other(
                "runtime storage requires an owned regular file with no hard links",
            ));
        }
        Ok(())
    }

    impl Directory {
        /// Opens every component without following links. New directories are
        /// private from creation. Only an explicitly private leaf is chmodded;
        /// an explicit log in /tmp must never change /tmp's permissions.
        pub fn open(path: &Path, private: bool) -> io::Result<Self> {
            Self::open_inner(path, private, false)
        }
        pub(crate) fn open_durable(path: &Path, private: bool) -> io::Result<Self> {
            Self::open_inner(path, private, true)
        }
        fn open_inner(path: &Path, private: bool, durable: bool) -> io::Result<Self> {
            let absolute = if path.is_absolute() {
                path.to_owned()
            } else {
                std::env::current_dir()?.join(path)
            };
            // macOS exposes these system-owned aliases in ordinary CLI paths.
            // Resolve only the known prefix, never a workspace component.
            #[cfg(target_os = "macos")]
            let absolute = ["/tmp", "/var", "/etc"]
                .into_iter()
                .find_map(|prefix| {
                    absolute
                        .strip_prefix(prefix)
                        .ok()
                        .map(|rest| Path::new("/private").join(&prefix[1..]).join(rest))
                })
                .unwrap_or(absolute);
            let mut directory = Self(File::open("/")?);
            for component in absolute.components() {
                let name = match component {
                    Component::RootDir | Component::CurDir => continue,
                    Component::Normal(name) => cstring(name)?,
                    Component::ParentDir => CString::new("..").unwrap(),
                    _ => {
                        return Err(io::Error::other(
                            "runtime storage path has an unsupported prefix",
                        ));
                    }
                };
                // SAFETY: directory owns its descriptor; name is NUL-terminated.
                let mut fd = unsafe {
                    libc::openat(
                        directory.0.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
                    let created =
                        unsafe { libc::mkdirat(directory.0.as_raw_fd(), name.as_ptr(), 0o700) };
                    if created < 0
                        && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
                    {
                        return Err(io::Error::last_os_error());
                    }
                    fd = unsafe {
                        libc::openat(
                            directory.0.as_raw_fd(),
                            name.as_ptr(),
                            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        )
                    };
                }
                if fd < 0 {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: openat returned a new owned descriptor.
                let next = Self(unsafe { File::from_raw_fd(fd) });
                // Retry a previous failed mkdir sync even when the entry exists.
                if durable {
                    directory.0.sync_all()?;
                }
                directory = next;
            }
            if private {
                let metadata = directory.0.metadata()?;
                if metadata.uid() != unsafe { libc::geteuid() } {
                    return Err(io::Error::other(
                        "runtime storage directory is owned by another user",
                    ));
                }
                directory
                    .0
                    .set_permissions(std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(directory)
        }

        /// Opens one owned private child relative to the pinned directory.
        pub(crate) fn child(&self, name: &OsStr) -> io::Result<Self> {
            let name = leaf(name)?;
            let created = unsafe { libc::mkdirat(self.0.as_raw_fd(), name.as_ptr(), 0o700) };
            if created < 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(io::Error::last_os_error());
            }
            // Existing entries may be remnants of an interrupted creation.
            self.0.sync_all()?;
            let fd = unsafe {
                libc::openat(
                    self.0.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            if file.metadata()?.uid() != unsafe { libc::geteuid() } {
                return Err(io::Error::other(
                    "Private directory is owned by another user",
                ));
            }
            file.set_permissions(std::fs::Permissions::from_mode(0o700))?;
            Ok(Self(file))
        }

        /// The caller owns both names under its stable advisory lock.
        pub(crate) fn rename(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
            let (from, to) = (leaf(from)?, leaf(to)?);
            if unsafe {
                libc::renameat(
                    self.0.as_raw_fd(),
                    from.as_ptr(),
                    self.0.as_raw_fd(),
                    to.as_ptr(),
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        fn open_file(&self, name: &OsStr, flags: i32, private: bool) -> io::Result<File> {
            let name_c = leaf(name)?;
            // Nonblocking open ensures a supplied FIFO cannot stall startup.
            let fd = unsafe {
                libc::openat(
                    self.0.as_raw_fd(),
                    name_c.as_ptr(),
                    flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let validated = owned_regular(&file).and_then(|()| {
                if private {
                    file.set_permissions(std::fs::Permissions::from_mode(0o600))
                } else {
                    Ok(())
                }
            });
            if let Err(error) = validated {
                // Exclusive creation owns the new directory entry even if
                // subsequent metadata validation fails. Existing files must
                // never be removed by a failed append/read admission.
                if flags & libc::O_CREAT != 0 && flags & libc::O_EXCL != 0 {
                    let _ = self.remove_owned(name, &file);
                }
                return Err(error);
            }
            Ok(file)
        }

        pub fn append(&self, name: &OsStr) -> io::Result<File> {
            self.open_file(name, libc::O_RDWR | libc::O_CREAT | libc::O_APPEND, true)
        }

        pub fn create_new(&self, name: &OsStr) -> io::Result<File> {
            self.open_file(name, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL, true)
        }

        pub fn open_read(&self, name: &OsStr) -> io::Result<File> {
            self.open_file(name, libc::O_RDONLY, false)
        }

        pub fn remove_owned(&self, name: &OsStr, file: &File) -> io::Result<()> {
            let name_c = leaf(name)?;
            let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: the descriptor, C string and output pointer are valid.
            if unsafe {
                libc::fstatat(
                    self.0.as_raw_fd(),
                    name_c.as_ptr(),
                    current.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: successful fstatat initialized the complete structure.
            let current = unsafe { current.assume_init() };
            let expected = file.metadata()?;
            if current.st_dev == expected.dev() && current.st_ino == expected.ino() {
                self.remove(name)?;
            }
            Ok(())
        }

        pub fn read(&self, name: &OsStr, limit: usize) -> io::Result<Vec<u8>> {
            let file = self.open_file(name, libc::O_RDONLY, true)?;
            let mut bytes = Vec::new();
            file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > limit {
                return Err(io::Error::other(
                    "runtime storage file exceeds its size limit",
                ));
            }
            Ok(bytes)
        }

        pub fn remove(&self, name: &OsStr) -> io::Result<()> {
            let name = leaf(name)?;
            if unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0) } < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        pub fn sync(&self) -> io::Result<()> {
            self.0.sync_all()
        }

        pub fn atomic_write(&self, name: &OsStr, bytes: &[u8]) -> io::Result<()> {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let destination = leaf(name)?;
            for _ in 0..64 {
                let pending = PathBuf::from(format!(
                    ".runyte-write-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                let mut file = match self.open_file(
                    pending.as_os_str(),
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                    true,
                ) {
                    Ok(file) => file,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error),
                };
                let result = (|| {
                    file.write_all(bytes)?;
                    file.sync_all()?;
                    let source = leaf(pending.as_os_str())?;
                    if unsafe {
                        libc::renameat(
                            self.0.as_raw_fd(),
                            source.as_ptr(),
                            self.0.as_raw_fd(),
                            destination.as_ptr(),
                        )
                    } < 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                    self.0.sync_all()
                })();
                if result.is_err() {
                    let _ = self.remove(pending.as_os_str());
                }
                return result;
            }
            Err(io::Error::other(
                "cannot create a unique runtime storage file",
            ))
        }
    }
}

#[cfg(not(unix))]
mod platform {
    use super::*;
    use std::ffi::OsStr;
    #[derive(Debug)]
    pub struct Directory;
    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private runtime storage is not supported on this platform",
        ))
    }
    impl Directory {
        pub fn open(_: &Path, _: bool) -> io::Result<Self> {
            unsupported()
        }
        pub(crate) fn open_durable(_: &Path, _: bool) -> io::Result<Self> {
            unsupported()
        }
        pub(crate) fn child(&self, _: &OsStr) -> io::Result<Self> {
            unsupported()
        }
        pub(crate) fn rename(&self, _: &OsStr, _: &OsStr) -> io::Result<()> {
            unsupported()
        }
        pub fn append(&self, _: &OsStr) -> io::Result<File> {
            unsupported()
        }
        pub fn create_new(&self, _: &OsStr) -> io::Result<File> {
            unsupported()
        }
        pub fn open_read(&self, _: &OsStr) -> io::Result<File> {
            unsupported()
        }
        pub fn remove_owned(&self, _: &OsStr, _: &File) -> io::Result<()> {
            unsupported()
        }
        pub fn read(&self, _: &OsStr, _: usize) -> io::Result<Vec<u8>> {
            unsupported()
        }
        pub fn remove(&self, _: &OsStr) -> io::Result<()> {
            unsupported()
        }
        pub fn sync(&self) -> io::Result<()> {
            unsupported()
        }
        pub fn atomic_write(&self, _: &OsStr, _: &[u8]) -> io::Result<()> {
            unsupported()
        }
    }
}

pub(crate) use platform::Directory;

mod owned_file;
pub(crate) use owned_file::OwnedFile;
