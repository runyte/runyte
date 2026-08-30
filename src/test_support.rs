// SPDX-License-Identifier: MPL-2.0

//! Narrow shared contracts used by process-level tests and their production
//! test hooks.

use std::{ffi::OsStr, path::PathBuf};

#[cfg(unix)]
use std::{
    fs, io,
    ops::Deref,
    os::unix::fs::DirBuilderExt,
    path::{Component, Path},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
const OWNER_MARKER: &str = ".runyte-test-owner";
#[cfg(unix)]
const WRAPPER_RESERVE: &str = "xxxxxxxxxxxxxxxx";

/// One private, short runtime root owned by the allocating test.
///
/// Unix-domain socket limits are much shorter than ordinary path limits. The
/// allocator measures the target platform's `sockaddr_un`, includes Runyte's
/// complete endpoint suffix in that budget, and falls back from a long
/// advertised temporary directory to canonical `/tmp`. No environment is
/// changed; callers pass this path explicitly to endpoints and child commands.
#[cfg(unix)]
#[derive(Debug)]
pub struct TestRuntimeRoot {
    path: PathBuf,
    owner: String,
}

#[cfg(unix)]
impl TestRuntimeRoot {
    pub fn new(label: &str) -> io::Result<Self> {
        Self::new_in(label, &std::env::temp_dir())
    }

    pub fn new_in(label: &str, advertised_base: &Path) -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);

        Self::new_in_with_candidates(label, advertised_base, || {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            let token =
                now ^ sequence.rotate_left(19) ^ u64::from(std::process::id()).rotate_left(41);
            let owner = format!("{token:016x}-{sequence:x}");
            (token, owner)
        })
    }

    fn new_in_with_candidates(
        label: &str,
        advertised_base: &Path,
        mut candidate: impl FnMut() -> (u64, String),
    ) -> io::Result<Self> {
        let label = short_label(label);
        let base = select_short_base(advertised_base, &label)?;
        loop {
            let (token, owner) = candidate();
            let path = base.join(format!("ryt-{label}-{:08x}", token as u32));
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => {
                    if let Err(error) = fs::write(path.join(OWNER_MARKER), owner.as_bytes()) {
                        let _ = fs::remove_dir_all(&path);
                        return Err(error);
                    }
                    debug_assert!(
                        test_socket_path(&path.join(WRAPPER_RESERVE))
                            .as_os_str()
                            .as_encoded_bytes()
                            .len()
                            <= unix_socket_path_capacity()
                    );
                    return Ok(Self { path, owner });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn create_private_dir(&self, relative: impl AsRef<Path>) -> io::Result<PathBuf> {
        let relative = relative.as_ref();
        if !matches!(
            relative.components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a private test directory must be one direct child of its runtime root",
            ));
        }
        let path = self.path.join(relative);
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(&path)?;
        Ok(path)
    }

    /// Removes this root only while its private owner marker still matches.
    ///
    /// Normal scoped fixtures rely on `Drop`; process-wide test fixtures call
    /// this from an exit hook because Rust does not drop static values.
    pub fn cleanup_if_owned(&self) {
        let marker = self.path.join(OWNER_MARKER);
        if fs::read_to_string(marker).is_ok_and(|owner| owner == self.owner) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(unix)]
impl Deref for TestRuntimeRoot {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.path()
    }
}

#[cfg(unix)]
impl AsRef<Path> for TestRuntimeRoot {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

#[cfg(unix)]
impl Drop for TestRuntimeRoot {
    fn drop(&mut self) {
        self.cleanup_if_owned();
    }
}

#[cfg(unix)]
pub fn unix_socket_path_capacity() -> usize {
    // `sockaddr_un` is a plain C address struct and all-zero is valid.
    let address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
    address.sun_path.len().saturating_sub(1)
}

#[cfg(unix)]
pub fn test_socket_path(runtime_root: &Path) -> PathBuf {
    runtime_root
        .join("runyte")
        .join("0".repeat(crate::workspace::WORKSPACE_ID_LENGTH))
        .join("workspace.sock")
}

#[cfg(unix)]
fn select_short_base(advertised: &Path, label: &str) -> io::Result<PathBuf> {
    let advertised = advertised.canonicalize()?;
    if base_fits(&advertised, label) {
        return Ok(advertised);
    }
    let short = Path::new("/tmp").canonicalize()?;
    if base_fits(&short, label) {
        return Ok(short);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "no temporary base leaves room for a Runyte test socket",
    ))
}

#[cfg(unix)]
fn base_fits(base: &Path, label: &str) -> bool {
    let longest_root = base.join(format!("ryt-{label}-00000000"));
    test_socket_path(&longest_root.join(WRAPPER_RESERVE))
        .as_os_str()
        .as_encoded_bytes()
        .len()
        <= unix_socket_path_capacity()
}

#[cfg(unix)]
fn short_label(label: &str) -> String {
    let label = label
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(5)
        .collect::<String>()
        .to_ascii_lowercase();
    if label.is_empty() {
        "test".to_owned()
    } else {
        label
    }
}

/// Appends a marker suffix without replacing any extension on the owning base
/// path. The suffix includes its separator, for example `.ready`.
pub fn marker_path(base: impl Into<PathBuf>, suffix: &OsStr) -> PathBuf {
    let base = base.into();
    let mut marker = base.into_os_string();
    marker.push(suffix);
    PathBuf::from(marker)
}

/// Marker pair for the one-shot wait-status barrier.
pub fn wait_status_barrier_paths(base: impl Into<PathBuf>) -> (PathBuf, PathBuf) {
    let base = base.into();
    (
        marker_path(&base, OsStr::new(".ready")),
        marker_path(base, OsStr::new(".release")),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn wait_barrier_markers_preserve_the_complete_base_name() {
        let (first_ready, first_release) = wait_status_barrier_paths("/tmp/wait.first");
        let (second_ready, second_release) = wait_status_barrier_paths("/tmp/wait.second");

        assert_eq!(first_ready, Path::new("/tmp/wait.first.ready"));
        assert_eq!(first_release, Path::new("/tmp/wait.first.release"));
        assert_eq!(second_ready, Path::new("/tmp/wait.second.ready"));
        assert_eq!(second_release, Path::new("/tmp/wait.second.release"));
        assert_ne!(first_ready, second_ready);
        assert_ne!(first_release, second_release);
    }

    #[cfg(unix)]
    #[test]
    fn long_advertised_temporary_base_falls_back_with_socket_budget_intact() {
        let outer = std::env::temp_dir().join(format!(
            "runyte-runtime-base-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let long = outer.join("a".repeat(80));
        fs::create_dir_all(&long).unwrap();
        let runtime = TestRuntimeRoot::new_in("long-base", &long).unwrap();
        let project = runtime.join("project");
        fs::create_dir(&project).unwrap();
        let endpoint = crate::workspace::transport::LocalEndpoint::discover_with_runtime(
            &project.join(".runyte"),
            &project,
            Some(runtime.path()),
        )
        .unwrap();

        assert!(!runtime.starts_with(long.canonicalize().unwrap()));
        assert!(
            endpoint.socket().as_os_str().as_encoded_bytes().len() <= unix_socket_path_capacity()
        );
        assert_eq!(
            endpoint.socket().as_os_str().as_encoded_bytes().len(),
            test_socket_path(&runtime)
                .as_os_str()
                .as_encoded_bytes()
                .len()
        );

        drop(runtime);
        fs::remove_dir_all(outer).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn advertised_temporary_alias_is_canonicalized() {
        let advertised = std::env::temp_dir();
        let canonical = advertised.canonicalize().unwrap();
        let alias = canonical
            .strip_prefix("/private")
            .ok()
            .map(Path::to_path_buf)
            .filter(|alias| alias.exists())
            .unwrap_or(advertised);
        let runtime = TestRuntimeRoot::new_in("alias", &alias).unwrap();
        let expected = if base_fits(&canonical, "alias") {
            canonical
        } else {
            Path::new("/tmp").canonicalize().unwrap()
        };

        assert_eq!(runtime.parent(), Some(expected.as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_allocations_and_a_stale_pid_name_never_collide() {
        let roots = (0..12)
            .map(|_| std::thread::spawn(|| TestRuntimeRoot::new("parallel").unwrap()))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        let paths = roots
            .iter()
            .map(|root| root.path().to_path_buf())
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(paths.len(), roots.len());
        for root in &roots {
            assert_eq!(
                fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_stale_candidate_name_forces_an_exclusive_allocation_retry() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
            ^ u64::from(std::process::id());
        let base = Path::new("/tmp").canonicalize().unwrap();
        let first_token = unique;
        let second_token = unique.wrapping_add(1);
        let stale = base.join(format!("ryt-stale-{:08x}", first_token as u32));
        fs::create_dir(&stale).unwrap();
        let mut candidates = [
            (first_token, "reused-pid-owner".to_owned()),
            (second_token, "current-owner".to_owned()),
        ]
        .into_iter();

        let runtime = TestRuntimeRoot::new_in_with_candidates("stale", &base, || {
            candidates.next().expect("allocator retried more than once")
        })
        .unwrap();

        assert_eq!(runtime.parent(), Some(base.as_path()));
        assert_eq!(
            runtime.file_name().unwrap(),
            format!("ryt-stale-{:08x}", second_token as u32).as_str()
        );
        assert!(stale.exists());
        drop(runtime);
        fs::remove_dir(stale).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_refuses_a_directory_whose_owner_marker_changed() {
        let runtime = TestRuntimeRoot::new("cleanup").unwrap();
        let path = runtime.path().to_path_buf();
        fs::write(path.join(OWNER_MARKER), "another allocation").unwrap();

        drop(runtime);

        assert!(path.exists());
        fs::remove_dir_all(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_children_cannot_escape_the_owned_runtime_root() {
        let runtime = TestRuntimeRoot::new("children").unwrap();
        let child = runtime.create_private_dir("cache").unwrap();

        assert_eq!(child.parent(), Some(runtime.path()));
        assert_eq!(
            fs::metadata(&child).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for rejected in ["../outside", "nested/child", ".", "/tmp/outside"] {
            assert_eq!(
                runtime.create_private_dir(rejected).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}
