// SPDX-License-Identifier: MPL-2.0

//! Private Windows PowerShell directory handoff for physical frontends.
//!
//! The record is `RNYCWD\x01\0`, a little-endian u32 count, then that many
//! little-endian UTF-16 code units. There is no BOM, terminator, or newline.
//! Encoding preserves code units; it never substitutes replacement characters.
//! The shell accepts at most 32,767 units and rejects NUL and trailing bytes.

use crate::{private_storage::Directory, windows_fs};
use std::{
    ffi::{OsStr, OsString},
    io,
    os::windows::ffi::OsStrExt,
    path::{Component, Path, PathBuf, Prefix},
};

const MAGIC: &[u8; 8] = b"RNYCWD\x01\0";
const MAX_UNITS: usize = 32_767;

/// A private parent pinned before editing begins. Later path replacements
/// cannot redirect the handoff to a different directory.
pub struct Prepared {
    parent: Directory,
    name: OsString,
}

impl Prepared {
    /// Admit an existing owner-private local NTFS parent without changing its
    /// permissions. The PowerShell wrapper creates this directory privately.
    pub fn prepare(path: &Path) -> io::Result<Self> {
        let name = path
            .file_name()
            .ok_or_else(|| invalid("cwd file has no name"))?;
        if !matches!(
            Path::new(name).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        ) || name.encode_wide().count() > 255
        {
            return Err(invalid("cwd file requires one ordinary NTFS file name"));
        }
        windows_fs::validate_relative(Path::new(name))?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| invalid("cwd file must have an absolute private parent"))?;
        if !parent.is_absolute() {
            return Err(invalid("cwd file must have an absolute private parent"));
        }
        let parent = Directory::open_existing(parent, true)?;
        match parent.open_read(name) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(Self {
            parent,
            name: name.to_owned(),
        })
    }

    /// Revalidate the shell spelling and atomically publish only a complete
    /// record. This is called after a successful explicit `:quit-here`.
    pub fn write(&self, directory: &Path) -> io::Result<()> {
        let directory = ordinary_directory(directory)?;
        self.parent
            .atomic_write(&self.name, &encode(directory.as_os_str())?)
    }
}

/// Require a directory spelling supported by the Windows PowerShell 5.1
/// wrapper before committing to quit. The original path remains authoritative:
/// conversion from an extended prefix must preserve its native identity.
pub(crate) fn ordinary_directory(path: &Path) -> io::Result<PathBuf> {
    let canonical = path.canonicalize()?;
    if !canonical.is_dir() {
        return Err(invalid("shell handoff destination is not a directory"));
    }
    let mut parts = canonical.components();
    let mut ordinary = match parts.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                PathBuf::from(format!("{}:", drive as char))
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                let mut value = OsString::from(r"\\");
                value.push(server);
                value.push(r"\");
                value.push(share);
                PathBuf::from(value)
            }
            _ => {
                return Err(invalid(
                    "shell handoff requires an ordinary Windows directory",
                ));
            }
        },
        _ => {
            return Err(invalid(
                "shell handoff requires an absolute Windows directory",
            ));
        }
    };
    for part in parts {
        if let Component::Normal(name) = part {
            windows_fs::validate_relative(Path::new(name))?;
        }
        ordinary.push(part.as_os_str());
    }
    if ordinary.as_os_str().encode_wide().count() >= 260 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "PowerShell directory handoff requires a path shorter than 260 UTF-16 units",
        ));
    }
    if windows_fs::Identity::read(&ordinary)? != windows_fs::Identity::read(&canonical)? {
        return Err(io::Error::other(
            "shell handoff directory changed while resolving its Windows spelling",
        ));
    }
    Ok(ordinary)
}

fn encode(path: &OsStr) -> io::Result<Vec<u8>> {
    let units = path.encode_wide().take(MAX_UNITS + 1).collect::<Vec<_>>();
    if units.is_empty() || units.len() > MAX_UNITS || units.contains(&0) {
        return Err(invalid("invalid or oversized shell handoff path"));
    }
    let mut record = Vec::with_capacity(12 + units.len() * 2);
    record.extend_from_slice(MAGIC);
    record.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for unit in units {
        record.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(record)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestRuntimeRoot;
    use std::{fs, io::Read, os::windows::ffi::OsStringExt};

    #[test]
    fn encoding_preserves_exact_utf16_units_and_enforces_bounds() {
        let units = [b'C' as u16, 58, 92, 0xd800, 32, 0xdc00, 0xd83d, 0xde00];
        let record = encode(&OsString::from_wide(&units)).unwrap();
        assert_eq!(&record[..8], b"RNYCWD\x01\0");
        assert_eq!(&record[8..12], &(units.len() as u32).to_le_bytes());
        assert_eq!(
            record[12..]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
            units
        );
        assert!(encode(OsStr::new("")).is_err());
        assert!(encode(&OsString::from_wide(&[65, 0, 66])).is_err());
        assert!(encode(&OsString::from_wide(&vec![65; MAX_UNITS])).is_ok());
        assert!(encode(&OsString::from_wide(&vec![65; MAX_UNITS + 1])).is_err());
    }

    #[test]
    fn preparation_requires_existing_private_storage_without_creating_it() {
        let root = TestRuntimeRoot::new("cwd-preflight").unwrap();
        assert!(Prepared::prepare(&root.join("missing/cwd")).is_err());
        assert!(!root.join("missing").exists());
        assert!(Prepared::prepare(Path::new("relative/cwd")).is_err());
        let parent = Directory::open(&root.join("private"), true).unwrap();
        fs::create_dir(root.join("private/directory")).unwrap();
        assert!(Prepared::prepare(&root.join("private/directory")).is_err());
        parent.create_new(OsStr::new("issued")).unwrap();
        fs::hard_link(root.join("private/issued"), root.join("private/alias")).unwrap();
        assert!(Prepared::prepare(&root.join("private/alias")).is_err());
        assert!(Prepared::prepare(&root.join("private/cwd")).is_ok());
        assert!(!root.join("private/cwd").exists());
    }

    #[test]
    fn atomic_handoff_owns_its_parent_and_leaves_previous_readers_intact() {
        let root = TestRuntimeRoot::new("cwd-pinned").unwrap();
        let original = root.join("private");
        let moved = root.join("moved");
        let parent = Directory::open(&original, true).unwrap();
        let destination = root.join("literal [brackets] café $directory");
        fs::create_dir(&destination).unwrap();
        let prepared = Prepared::prepare(&original.join("cwd")).unwrap();
        assert!(!original.join("cwd").exists());
        // Windows can refuse renaming a directory with an open child. Move
        // the pinned parent first, then exercise replacement with a live reader.
        fs::rename(&original, &moved).unwrap();
        fs::create_dir(&original).unwrap();
        prepared.write(root.path()).unwrap();
        let mut previous = parent.open_read(OsStr::new("cwd")).unwrap();
        let first = fs::read(moved.join("cwd")).unwrap();
        prepared.write(&destination).unwrap();
        let mut held = Vec::new();
        previous.read_to_end(&mut held).unwrap();
        assert_eq!(held, first);
        assert_eq!(
            fs::read(moved.join("cwd")).unwrap(),
            encode(ordinary_directory(&destination).unwrap().as_os_str()).unwrap()
        );
        assert!(!original.join("cwd").exists());
        assert_eq!(fs::read_dir(&moved).unwrap().count(), 1);
    }

    #[test]
    fn shell_spelling_requires_a_real_identity_equivalent_ordinary_directory() {
        let root = TestRuntimeRoot::new("cwd-spelling").unwrap();
        let ordinary = ordinary_directory(root.path()).unwrap();
        assert!(!ordinary.as_os_str().to_string_lossy().starts_with(r"\\?\"));
        assert_eq!(
            windows_fs::Identity::read(&ordinary).unwrap(),
            windows_fs::Identity::read(root.path()).unwrap()
        );
        let file = root.join("file");
        fs::write(&file, b"fixture").unwrap();
        assert!(ordinary_directory(&file).is_err());
        assert!(ordinary_directory(&root.join("missing")).is_err());
        let mut long = root.path().to_path_buf();
        while long.as_os_str().encode_wide().count() < 270 {
            long.push("long-directory-segment");
        }
        fs::create_dir_all(&long).unwrap();
        assert_eq!(
            ordinary_directory(&long).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }
}
