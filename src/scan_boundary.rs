// SPDX-License-Identifier: MPL-2.0

//! Where a recursive scan begun from an arbitrary directory may go.
//!
//! A project scan starts at a project root someone chose. An editor-mode
//! search starts wherever the reader happens to be — `/etc`, a home
//! directory, `/` — so it is contained: it refuses to start from the root of
//! the filesystem tree or from a virtual filesystem, never crosses onto
//! another mounted filesystem, and stops after a fixed number of entries.
//! Knows nothing about pickers, ignore files, or what a scan collects.

use std::path::Path;

/// How many entries a contained scan visits before it stops and reports
/// itself limited. Project scans have no such cap; a directory chosen as a
/// project is expected to be walked in full.
pub const CONTAINED_SCAN_ENTRY_LIMIT: usize = 20_000;

/// Why a contained scan must not start at `root`, or `None` when it may.
///
/// `root` is expected to be canonical: the filesystem root has no parent,
/// and the virtual-filesystem checks compare the resolved location.
pub fn refusal(root: &Path) -> Option<String> {
    if root.parent().is_none() {
        return Some(format!(
            "searching everything below {} is refused without a workspace; open a narrower directory",
            root.display()
        ));
    }
    is_virtual(root).then(|| {
        format!(
            "{} is on a virtual filesystem; it is not searched without a workspace",
            root.display()
        )
    })
}

/// The filesystem a contained scan stays on.
#[derive(Clone, Copy, Debug)]
pub struct Boundary {
    #[cfg(unix)]
    device: u64,
}

impl Boundary {
    /// The boundary of the filesystem holding `root`.
    pub fn of(root: &Path) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                device: std::fs::metadata(root)?.dev(),
            })
        }
        #[cfg(not(unix))]
        {
            std::fs::metadata(root)?;
            Ok(Self {})
        }
    }

    /// Whether a directory with this metadata is on the same filesystem. A
    /// mount point below the root, such as `/proc` under `/`, reports another
    /// device and is not entered.
    pub fn admits(&self, metadata: &std::fs::Metadata) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.dev() == self.device
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            true
        }
    }
}

#[cfg(unix)]
fn is_virtual(root: &Path) -> bool {
    // `/dev` is often devtmpfs, which reports the same magic number as an
    // ordinary tmpfs, so the conventional mount points are named outright.
    ["/proc", "/sys", "/dev"]
        .iter()
        .any(|mount| root.starts_with(mount))
        || is_virtual_filesystem(root)
}

#[cfg(not(unix))]
fn is_virtual(_root: &Path) -> bool {
    false
}

#[cfg(target_os = "linux")]
fn is_virtual_filesystem(root: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    // Kernel pseudo-filesystems: their files describe the running system
    // rather than holding anything a person edits, and several of them are
    // effectively unbounded.
    const VIRTUAL: &[u32] = &[
        0x9fa0,      // proc
        0x6265_6572, // sysfs
        0x1cd1,      // devpts
        0x0027_e0eb, // cgroup
        0x6367_7270, // cgroup2
        0x6462_6720, // debugfs
        0x7472_6163, // tracefs
        0x7363_6673, // securityfs
        0x6165_676c, // pstore
        0xcafe_4a11, // bpf
        0x6265_6570, // configfs
        0x6573_5543, // fusectl
        0x1980_0202, // mqueue
        0xde5e_81e4, // efivarfs
        0x4249_4e4d, // binfmt_misc
        0xf97c_ff8c, // selinuxfs
    ];
    let Ok(path) = std::ffi::CString::new(root.as_os_str().as_bytes()) else {
        return false;
    };
    let mut status = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is a NUL-terminated string that outlives the call, and
    // `status` is writable storage of the type `statfs` fills.
    if unsafe { libc::statfs(path.as_ptr(), status.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: a successful `statfs` initialized the record.
    let kind = unsafe { status.assume_init() }.f_type;
    // `f_type` is signed and its width varies by target; the magic numbers
    // are 32-bit patterns, so compare those bits rather than sign-extending.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    VIRTUAL.contains(&(kind as u32))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn is_virtual_filesystem(_root: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("runyte-scan-boundary-{}-{id}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_filesystem_root_is_refused() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .ancestors()
            .last()
            .unwrap()
            .to_path_buf();
        let reason = refusal(&root).expect("the root is refused");
        assert!(reason.contains("open a narrower directory"), "{reason}");
    }

    #[test]
    fn an_ordinary_directory_is_admitted() {
        let directory = TempDir::new();
        assert_eq!(refusal(directory.path()), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn kernel_pseudo_filesystems_are_refused() {
        for path in ["/proc", "/proc/self", "/sys", "/dev"] {
            let path = Path::new(path);
            if path.exists() {
                let reason = refusal(path).expect("pseudo-filesystem is refused");
                assert!(reason.contains("virtual filesystem"), "{reason}");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_boundary_admits_its_own_filesystem() {
        let directory = TempDir::new();
        std::fs::create_dir(directory.path().join("nested")).unwrap();
        let boundary = Boundary::of(directory.path()).unwrap();
        let nested = std::fs::metadata(directory.path().join("nested")).unwrap();
        assert!(boundary.admits(&nested));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_boundary_rejects_another_mounted_filesystem() {
        let root = Boundary::of(Path::new("/")).unwrap();
        if let Ok(proc) = std::fs::metadata("/proc") {
            assert!(!root.admits(&proc));
        }
    }
}
