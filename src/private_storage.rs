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
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
    }

    impl Directory {
        /// Opens every component without following links. New directories are
        /// private from creation. Only an explicitly private leaf is chmodded;
        /// an explicit log in /tmp must never change /tmp's permissions.
        pub fn open(path: &Path, private: bool) -> io::Result<Self> {
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
                directory = Self(unsafe { File::from_raw_fd(fd) });
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

        fn open_file(&self, name: &OsStr, flags: i32) -> io::Result<File> {
            let name = leaf(name)?;
            // Nonblocking open ensures a supplied FIFO cannot stall startup.
            let fd = unsafe {
                libc::openat(
                    self.0.as_raw_fd(),
                    name.as_ptr(),
                    flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            owned_regular(&file)?;
            Ok(file)
        }

        pub fn append(&self, name: &OsStr) -> io::Result<File> {
            self.open_file(name, libc::O_RDWR | libc::O_CREAT | libc::O_APPEND)
        }

        pub fn read(&self, name: &OsStr, limit: usize) -> io::Result<Vec<u8>> {
            let file = self.open_file(name, libc::O_RDONLY)?;
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
        pub fn append(&self, _: &OsStr) -> io::Result<File> {
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
