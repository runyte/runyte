// SPDX-License-Identifier: MPL-2.0

//! Owned staging and conservative cleanup. No drop implementation deletes data.

use super::{
    ApplyIo, ApplyReport, EntryKind, FsOperation, IoStep, NEXT_TEMP, RecoveryEntry, RecoveryKind,
    entry_kind,
};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    kind: EntryKind,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl Identity {
    pub(super) fn read(path: &Path) -> io::Result<Self> {
        Ok(Self::from_metadata(&fs::symlink_metadata(path)?))
    }

    fn from_metadata(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            kind: entry_kind(metadata.file_type()),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
        }
    }

    pub(super) fn check(&self, path: &Path) -> io::Result<()> {
        if &Self::read(path)? != self {
            return Err(io::Error::other(format!(
                "staged entry changed identity: {}",
                path.display()
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(super) struct OwnedTree {
    pub(super) root: PathBuf,
    entries: Vec<(PathBuf, Identity)>,
}

impl OwnedTree {
    pub(super) fn allocate(parent: &Path, label: &str, io: &mut ApplyIo) -> io::Result<Self> {
        for _ in 0..64 {
            let value = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let root = parent.join(format!(".runyte-{label}-{}-{value}", std::process::id()));
            io.before(IoStep::Allocate, parent, &root)?;
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&root) {
                Ok(()) => {
                    let identity = Identity::read(&root)?;
                    return Ok(Self {
                        entries: vec![(root.clone(), identity)],
                        root,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate filesystem-plan staging after 64 collisions",
        ))
    }

    pub(super) fn payload(&self) -> PathBuf {
        self.root.join("entry")
    }

    pub(super) fn check_root(&self) -> io::Result<()> {
        self.entries[0].1.check(&self.root)
    }

    fn record(&mut self, path: &Path) -> io::Result<()> {
        self.entries
            .push((path.to_path_buf(), Identity::read(path)?));
        Ok(())
    }

    fn create_file(&mut self, path: &Path) -> io::Result<File> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        self.entries.push((
            path.to_path_buf(),
            Identity::from_metadata(&file.metadata()?),
        ));
        Ok(file)
    }

    /// Validate the complete owned tree before cleanup, then remove leaves
    /// before parents. Unknown children prevent any deletion. These checks do
    /// not authorize later pathname uses against a hostile concurrent process.
    pub(super) fn cleanup(&self, io: &mut ApplyIo) -> io::Result<()> {
        io.before(IoStep::Cleanup, &self.root, &self.root)?;
        let owned = self
            .entries
            .iter()
            .map(|(path, _)| path.as_path())
            .collect::<HashSet<_>>();
        for (path, identity) in &self.entries {
            identity.check(path)?;
            if identity.kind == EntryKind::Directory {
                for child in fs::read_dir(path)? {
                    let child = child?.path();
                    if !owned.contains(child.as_path()) {
                        return Err(io::Error::other(format!(
                            "unexpected entry in staging: {}",
                            child.display()
                        )));
                    }
                }
            }
        }
        for (path, identity) in self.entries.iter().rev() {
            identity.check(path)?;
            if identity.kind == EntryKind::Directory {
                fs::remove_dir(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    pub(super) fn cleanup_report(
        &self,
        original: &Path,
        kind: RecoveryKind,
        report: &mut ApplyReport,
        io: &mut ApplyIo,
    ) {
        if let Err(error) = self.cleanup(io) {
            report.recovery.push(RecoveryEntry {
                original: original.to_path_buf(),
                retained: self.root.clone(),
                kind,
                reason: error.to_string(),
            });
        }
    }

    pub(super) fn copy_entry(
        &mut self,
        source: &Path,
        target: &Path,
        io: &mut ApplyIo,
    ) -> io::Result<()> {
        io.before(IoStep::CopyEntry, source, target)?;
        self.check_root()?;
        let metadata = fs::symlink_metadata(source)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            let link = fs::read_link(source)?;
            super::create_symlink(source, &link, target).map_err(io::Error::other)?;
            self.record(target)?;
        } else if file_type.is_dir() {
            fs::create_dir(target)?;
            self.record(target)?;
            for entry in fs::read_dir(source)? {
                let entry = entry?;
                self.copy_entry(&entry.path(), &target.join(entry.file_name()), io)?;
            }
            fs::set_permissions(target, metadata.permissions())?;
        } else if file_type.is_file() {
            let mut source = File::open(source)?;
            let mut target = self.create_file(target)?;
            super::platform::copy_file(&mut source, &mut target)?;
        } else {
            return Err(io::Error::other("unsupported copy source"));
        }
        Ok(())
    }

    /// Once publication moved the payload away, only the container is ours.
    pub(super) fn published(&mut self) {
        self.entries.truncate(1);
    }
}

#[derive(Clone, Debug)]
pub(super) struct StagedMove {
    pub(super) operation: FsOperation,
    pub(super) original: PathBuf,
    pub(super) tree: OwnedTree,
    pub(super) identity: Identity,
}

impl StagedMove {
    pub(super) fn check(&self) -> io::Result<()> {
        self.tree.check_root()?;
        self.identity.check(&self.tree.payload())
    }

    pub(super) fn restore(&self, report: &mut ApplyReport, io: &mut ApplyIo) {
        let staged = self.tree.payload();
        let result = self
            .check()
            .and_then(|()| io.rename(IoStep::Restore, &staged, &self.original));
        match result {
            Ok(()) => self
                .tree
                .cleanup_report(&self.original, RecoveryKind::Staging, report, io),
            Err(error) => report.recovery.push(RecoveryEntry {
                original: self.original.clone(),
                retained: staged,
                kind: RecoveryKind::Original,
                reason: error.to_string(),
            }),
        }
    }
}

/// Exercise support in disposable owned entries before a mixed plan deletes
/// or stages user data. Runtime errors still never trigger an unsafe fallback.
pub(super) fn probe(parent: &Path, report: &mut ApplyReport, io: &mut ApplyIo) -> io::Result<()> {
    let mut tree = OwnedTree::allocate(parent, "probe", io)?;
    let result = (|| {
        let source = tree.root.join("source");
        let occupied = tree.root.join("occupied");
        let target = tree.root.join("target");
        drop(tree.create_file(&source)?);
        drop(tree.create_file(&occupied)?);
        match io.rename(IoStep::Probe, &source, &occupied) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error),
            Ok(()) => {
                return Err(io::Error::other(
                    "filesystem did not honor exclusive rename",
                ));
            }
        }
        for (path, identity) in &tree.entries {
            identity.check(path)?;
        }
        io.rename(IoStep::Probe, &source, &target)?;
        tree.entries[1].0 = target;
        Ok(())
    })();
    tree.cleanup_report(parent, RecoveryKind::Staging, report, io);
    result
}
