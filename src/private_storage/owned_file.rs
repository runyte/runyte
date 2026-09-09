// SPDX-License-Identifier: MPL-2.0

//! A pinned runtime file whose final release never performs filesystem IO on
//! the editor thread. Every live file reserves room in the cleanup queue.

use super::Directory;
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
};

const MAX_OWNED_FILES: usize = 512;
static LIVE: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
struct CleanupFile {
    directory: Arc<Directory>,
    name: OsString,
    file: File,
}

#[derive(Debug)]
struct Reservation;
impl Reservation {
    fn acquire() -> io::Result<Self> {
        LIVE.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            (value < MAX_OWNED_FILES).then_some(value + 1)
        })
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "private file cleanup capacity reached",
            )
        })?;
        Ok(Self)
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

type Cleanup = (CleanupFile, Reservation);

fn cleanup_sender() -> io::Result<SyncSender<Cleanup>> {
    static SENDER: OnceLock<Mutex<Option<SyncSender<Cleanup>>>> = OnceLock::new();
    let mut sender = SENDER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| io::Error::other("private file cleanup state unavailable"))?;
    if let Some(sender) = &*sender {
        return Ok(sender.clone());
    }
    let (tx, rx) = mpsc::sync_channel::<Cleanup>(MAX_OWNED_FILES);
    std::thread::Builder::new()
        .name("runyte-private-cleanup".into())
        .spawn(move || {
            while let Ok((owned, reservation)) = rx.recv() {
                let _ = owned.directory.remove_owned(&owned.name, &owned.file);
                drop(owned);
                drop(reservation);
            }
        })?;
    *sender = Some(tx.clone());
    Ok(tx)
}

#[derive(Debug)]
pub(crate) struct OwnedFile {
    path: PathBuf,
    cleanup: Option<Cleanup>,
    sender: SyncSender<Cleanup>,
}

impl OwnedFile {
    /// Runs on an IO worker. The returned path is not permission isolation from
    /// another process of the same user; it identifies this exact issued inode.
    pub fn create(directory: &Path, prefix: &str) -> io::Result<Self> {
        let reservation = Reservation::acquire()?;
        let sender = cleanup_sender()?;
        let storage = Arc::new(Directory::open(directory, true)?);
        let mut nonce = [0u8; 16];
        File::open("/dev/urandom")?.read_exact(&mut nonce)?;
        let name = OsString::from(format!(
            "{prefix}-{}",
            nonce
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ));
        let file = storage.create_new(&name)?;
        let result = Self {
            path: directory.join(&name),
            cleanup: Some((
                CleanupFile {
                    directory: storage,
                    name,
                    file,
                },
                reservation,
            )),
            sender,
        };
        result.verify_path()?;
        Ok(result)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file(&self) -> &File {
        &self.cleanup.as_ref().unwrap().0.file
    }

    pub fn open_read(&self) -> io::Result<File> {
        let owned = &self.cleanup.as_ref().unwrap().0;
        let file = owned.directory.open_read(&owned.name)?;
        if !same_file(&file.metadata()?, &owned.file.metadata()?) {
            return Err(io::Error::other("private file was replaced"));
        }
        self.verify_path()?;
        Ok(file)
    }

    pub fn verify_path(&self) -> io::Result<()> {
        let current = std::fs::symlink_metadata(&self.path)?;
        if !same_file(&current, &self.file().metadata()?) {
            return Err(io::Error::other(
                "private file path no longer identifies its issued inode",
            ));
        }
        Ok(())
    }
}

#[cfg(unix)]
fn same_file(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.is_file()
        && left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.nlink() == 1
        && right.nlink() == 1
}
#[cfg(not(unix))]
fn same_file(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    false
}

impl Drop for OwnedFile {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            // Reservations cap queued + live files at the queue capacity. A
            // full queue is impossible while this file still owns a slot.
            let _ = self.sender.try_send(cleanup);
        }
    }
}
