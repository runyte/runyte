// SPDX-License-Identifier: MPL-2.0

//! Bounded opaque state; canonical content revisions and private atomic storage.
use super::{
    application::{Error, ErrorCode as Code},
    json,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

pub(crate) mod wire;

pub const MAX_DOCUMENT_BYTES: usize = super::MAX_BYTES - 4096;
// Tiny one-key BTreeMaps still allocate full leaves. Valid deeply nested
// 16K-node data can exceed 8 MiB before raw input and canonical output copies.
pub const PREPARE_CHARGE: usize = 16 * 1024 * 1024;
pub const LIMITS: json::Limits = json::Limits {
    max_bytes: MAX_DOCUMENT_BYTES,
    max_depth: 16,
    max_nodes: 16384,
    max_container: 1024,
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u32,
    pub data: Box<RawValue>,
}
impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StateDocument(<private>)")
    }
}
#[derive(Clone, Serialize)]
pub struct Info {
    pub revision: String,
    pub document: Option<Arc<RawValue>>,
}
impl std::fmt::Debug for Info {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateInfo")
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
impl Info {
    fn missing() -> Self {
        Self {
            revision: "s:missing".into(),
            document: None,
        }
    }
    fn canonical(text: String) -> Result<Self, Error> {
        let revision = format!("s:{}", crate::hash::sha256_hex(text.as_bytes()));
        let document = RawValue::from_string(text).map_err(|_| invalid())?;
        Ok(Self {
            revision,
            document: Some(document.into()),
        })
    }
}
pub(crate) enum Task {
    Get,
    Set {
        expected_revision: String,
        document: Document,
    },
    Delete {
        expected_revision: String,
    },
}
pub(crate) struct Control {
    phase: AtomicU8,
    #[cfg(test)]
    hook: std::sync::Mutex<Option<Hook>>,
}
#[cfg(test)]
pub(crate) type Hook = Arc<dyn Fn(Checkpoint) -> Result<(), Error> + Send + Sync>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Checkpoint {
    BeforeMutation,
    AfterMutation,
}
impl Control {
    pub(crate) fn new() -> Self {
        Self {
            phase: AtomicU8::new(0),
            #[cfg(test)]
            hook: Default::default(),
        }
    }
    pub(crate) fn cancel(&self) {
        let _ = self
            .phase
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
    }
    pub(crate) fn attempted(&self) -> bool {
        self.phase.load(Ordering::Acquire) == 2
    }
    fn check(&self) -> Result<(), Error> {
        if self.phase.load(Ordering::Acquire) == 1 {
            Err(Error::new(Code::Cancelled, "State operation cancelled"))
        } else {
            Ok(())
        }
    }
    fn begin_mutation(&self) -> Result<(), Error> {
        self.checkpoint(Checkpoint::BeforeMutation)?;
        self.phase
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| Error::new(Code::Cancelled, "State operation cancelled"))
    }
    fn checkpoint(&self, point: Checkpoint) -> Result<(), Error> {
        #[cfg(test)]
        if let Some(hook) = self.hook.lock().unwrap().clone() {
            return hook(point);
        }
        let _ = point;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn set_hook(&self, hook: Hook) {
        *self.hook.lock().unwrap() = Some(hook);
    }
}
fn invalid() -> Error {
    Error::new(Code::InvalidArgument, "Invalid state document")
}
fn unavailable() -> Error {
    Error::new(
        Code::Unavailable,
        "Private plugin state storage is unavailable",
    )
}
fn unknown() -> Error {
    Error::new(
        Code::OutcomeUnknown,
        "State mutation outcome is unknown; read state before retrying",
    )
}
fn stale() -> Error {
    Error::new(Code::Conflict, "Plugin state content changed")
}
fn canonical(document: Document) -> Result<Info, Error> {
    let Document { version, data } = document;
    json::validate(data.get(), LIMITS)?;
    let value: Value = serde_json::from_str(data.get()).map_err(|_| invalid())?;
    drop(data);
    let object = Value::Object(serde_json::Map::from_iter([
        ("version".into(), Value::from(version)),
        ("data".into(), value),
    ]));
    json::validate_value(&object, LIMITS)?;
    Info::canonical(serde_json::to_string(&object).map_err(|_| invalid())?)
}
fn decode(bytes: &[u8]) -> Result<Info, Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    json::validate(text, LIMITS)?;
    canonical(serde_json::from_str(text).map_err(|_| invalid())?)
}
fn expected(value: &str) -> Result<(), Error> {
    if value == "s:missing"
        || value.strip_prefix("s:").is_some_and(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
    {
        Ok(())
    } else {
        Err(Error::new(Code::InvalidArgument, "Invalid state revision"))
    }
}

pub(crate) fn run(
    root: &Path,
    identity: &str,
    task: Task,
    control: &Control,
) -> Result<Info, Error> {
    control.check()?;
    if !super::valid_name(identity) {
        return Err(invalid());
    }
    let (revision, next) = match task {
        Task::Get => (None, None),
        Task::Set {
            expected_revision,
            document,
        } => {
            expected(&expected_revision)?;
            (Some(expected_revision), Some(canonical(document)?))
        }
        Task::Delete { expected_revision } => {
            expected(&expected_revision)?;
            (Some(expected_revision), Some(Info::missing()))
        }
    };
    storage::run(root, identity, revision, next, control)
}

#[cfg(unix)]
mod storage {
    use super::*;
    use crate::private_storage::Directory;
    use std::{
        ffi::OsStr,
        io::{Read, Write},
        os::{fd::AsRawFd, unix::fs::MetadataExt},
    };
    const FILE: &str = "state.json";
    const PENDING: &str = "state.pending";
    fn read(directory: &Directory) -> Result<Info, Error> {
        let mut file = match directory.open_read(OsStr::new(FILE)) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Info::missing()),
            Err(_) => return Err(unavailable()),
        };
        let metadata = file.metadata().map_err(|_| unavailable())?;
        if metadata.len() > MAX_DOCUMENT_BYTES as u64 {
            return Err(Error::new(
                Code::LimitExceeded,
                "Stored state document exceeds its limit",
            ));
        }
        if metadata.mode() & 0o777 != 0o600 {
            return Err(unavailable());
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_DOCUMENT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| unavailable())?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(Error::new(
                Code::LimitExceeded,
                "Stored state document exceeds its limit",
            ));
        }
        decode(&bytes)
    }
    pub(super) fn run(
        root: &Path,
        identity: &str,
        revision: Option<String>,
        next: Option<Info>,
        control: &Control,
    ) -> Result<Info, Error> {
        let directory = Directory::open_durable(root, true)
            .and_then(|root| root.child(OsStr::new("plugins")))
            .and_then(|plugins| plugins.child(OsStr::new(identity)))
            .map_err(|_| unavailable())?;
        let lock = directory
            .append(OsStr::new(".lock"))
            .map_err(|_| unavailable())?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } < 0 {
            let error = std::io::Error::last_os_error();
            return Err(if error.kind() == std::io::ErrorKind::WouldBlock {
                Error::new(Code::Busy, "Plugin state storage is locked")
            } else {
                unavailable()
            });
        }
        control.check()?;
        // Only this fixed, validated, owned orphan can be reclaimed. No scan.
        match directory.open_read(OsStr::new(PENDING)) {
            Ok(file) => directory
                .remove_owned(OsStr::new(PENDING), &file)
                .map_err(|_| unavailable())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(unavailable()),
        }
        let current = read(&directory)?;
        let Some(revision) = revision else {
            return Ok(current);
        };
        if current.revision != revision {
            return Err(stale());
        }
        let next = next.unwrap();
        if current.revision == next.revision {
            control.check()?;
            return Ok(next);
        }
        drop(current);
        let pending = if let Some(document) = &next.document {
            let mut file = directory
                .create_new(OsStr::new(PENDING))
                .map_err(|_| unavailable())?;
            let result = file
                .write_all(document.get().as_bytes())
                .and_then(|()| file.sync_all());
            if result.is_err() {
                let _ = directory.remove_owned(OsStr::new(PENDING), &file);
                return Err(unavailable());
            }
            Some(file)
        } else {
            None
        };
        let result = (|| {
            // Recheck after staging IO, under the same cross-host lock.
            if read(&directory)?.revision != revision {
                return Err(stale());
            }
            control.begin_mutation()?;
            let mutation = if pending.is_some() {
                directory.rename(OsStr::new(PENDING), OsStr::new(FILE))
            } else {
                directory.remove(OsStr::new(FILE))
            };
            mutation.map_err(|_| unknown())?;
            control
                .checkpoint(Checkpoint::AfterMutation)
                .map_err(|_| unknown())?;
            directory.sync().map_err(|_| unknown())?;
            Ok(next)
        })();
        if let Some(pending) = pending {
            let _ = directory.remove_owned(OsStr::new(PENDING), &pending);
        }
        result
    }
}
#[cfg(not(unix))]
mod storage {
    use super::*;
    pub(super) fn run(
        _: &Path,
        _: &str,
        _: Option<String>,
        _: Option<Info>,
        _: &Control,
    ) -> Result<Info, Error> {
        Err(Error::new(
            Code::Unsupported,
            "Private plugin state is unavailable on this platform",
        ))
    }
}

#[derive(Debug)]
pub struct Event {
    pub(crate) identity: String,
    pub(crate) generation: String,
    pub(crate) request: String,
    pub(crate) result: Result<Info, Error>,
    pub(crate) _permit: Arc<tokio::sync::OwnedSemaphorePermit>,
}

#[cfg(test)]
mod tests;
