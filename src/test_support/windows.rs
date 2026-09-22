// SPDX-License-Identifier: MPL-2.0

//! Fixture-owned temporary storage; Windows tests do not need Unix socket limits.
use std::{
    ffi::OsStr,
    fs, io,
    ops::Deref,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug)]
pub struct TestRuntimeRoot {
    path: PathBuf,
    owner: String,
}
impl TestRuntimeRoot {
    pub fn new(label: &str) -> io::Result<Self> {
        Self::new_in(label, &std::env::temp_dir())
    }
    pub fn new_in(label: &str, base: &Path) -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = base.canonicalize()?;
        let label: String = label
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(16)
            .collect();
        for _ in 0..128 {
            let owner = format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let path = base.join(format!("ryt-{label}-{owner}"));
            match crate::windows_fs::create_private_directory(&path) {
                Ok(()) => {
                    fs::write(path.join(".runyte-test-owner"), &owner)?;
                    return Ok(Self { path, owner });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "test directory allocation exhausted",
        ))
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
                "expected one child directory",
            ));
        }
        let path = self.path.join(relative);
        crate::windows_fs::create_private_directory(&path)?;
        Ok(path)
    }
    /// Writes fixture metadata through the same protected directory owner as
    /// native endpoint records. The target must stay inside this test root.
    pub fn atomic_write_private(
        &self,
        directory: &Path,
        name: &OsStr,
        bytes: &[u8],
    ) -> io::Result<()> {
        if !directory.starts_with(&self.path) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "private fixture target is outside its test root",
            ));
        }
        crate::private_storage::Directory::open(directory, true)?.atomic_write(name, bytes)
    }
    pub fn cleanup_if_owned(&self) {
        if fs::read_to_string(self.path.join(".runyte-test-owner"))
            .is_ok_and(|owner| owner == self.owner)
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
impl Deref for TestRuntimeRoot {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}
impl Drop for TestRuntimeRoot {
    fn drop(&mut self) {
        self.cleanup_if_owned();
    }
}
