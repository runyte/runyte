// SPDX-License-Identifier: MPL-2.0

//! Private bounded downloads and immutable sources retained by confirmed plans.

use super::{
    application::{Error, ErrorCode},
    filesystem,
};
use crate::{
    fs_plan::{DesiredEntry, EntryKind, FsPlan, SourceFingerprint, TransferMode},
    private_storage::OwnedFile,
};
use std::{
    io::{Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_STAGING: usize = 2;
pub const STAGING_CHARGE: usize = 64 * 1024;
pub const PREPARE_CHARGE: usize = 24 * 1024 * 1024;

pub(crate) struct Issued {
    pub job: String,
    pub download: Download,
    pub busy: bool,
    pub cancelled: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
pub struct Download {
    source: Arc<OwnedFile>,
    bytes: usize,
}

fn check(cancelled: &AtomicBool) -> Result<(), Error> {
    if cancelled.load(Ordering::Acquire) {
        Err(Error::new(
            ErrorCode::Cancelled,
            "Download preparation cancelled",
        ))
    } else {
        Ok(())
    }
}

fn storage_error(error: std::io::Error) -> Error {
    Error::new(
        if error.kind() == std::io::ErrorKind::WouldBlock {
            ErrorCode::Busy
        } else {
            ErrorCode::Conflict
        },
        "Private download storage is unavailable or changed",
    )
}

impl Download {
    /// Called only on an IO worker. The plugin receives an empty existing file;
    /// the declared length is enforced when bytes are sealed, not preallocated.
    pub fn create(state_root: &Path, bytes: usize, cancelled: &AtomicBool) -> Result<Self, Error> {
        check(cancelled)?;
        if bytes > MAX_BYTES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "Download exceeds 8 MiB",
            ));
        }
        let directory = state_root.join("cache/plugin-downloads");
        let source = Arc::new(OwnedFile::create(&directory, "download").map_err(storage_error)?);
        check(cancelled)?;
        Ok(Self { source, bytes })
    }

    pub fn path(&self) -> &Path {
        self.source.path()
    }

    pub fn prepare(
        &self,
        root: &Path,
        directory: &filesystem::Directory,
        destination: &str,
        sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<FsPlan, Error> {
        check(cancelled)?;
        if sha256.len() != 64
            || !sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::new(
                ErrorCode::InvalidArgument,
                "Expected a lowercase SHA-256 digest",
            ));
        }
        crate::path_safety::ensure_within_root(root, &directory.path).map_err(|_| {
            Error::new(
                ErrorCode::InvalidArgument,
                "Destination directory is outside the workspace",
            )
        })?;
        let destination = filesystem::project_path(root, destination)?;
        let relative = crate::fs_plan::relative_from_root(&directory.path, &destination)
            .map_err(plan_error)?;
        if std::fs::symlink_metadata(&destination).is_ok() {
            return Err(Error::new(
                ErrorCode::Conflict,
                "Download destination already exists",
            ));
        }
        let mut input = self.source.open_read().map_err(storage_error)?;
        if input.metadata().map_err(storage_error)?.len() != self.bytes as u64 {
            return Err(Error::new(
                ErrorCode::Conflict,
                "Downloaded bytes differ from the declared length",
            ));
        }
        let mut content = Vec::with_capacity(self.bytes);
        let mut chunk = [0u8; 64 * 1024];
        loop {
            check(cancelled)?;
            let count = input.read(&mut chunk).map_err(storage_error)?;
            if count == 0 {
                break;
            }
            if content.len().saturating_add(count) > self.bytes {
                return Err(Error::new(
                    ErrorCode::Conflict,
                    "Downloaded bytes exceed the declared length",
                ));
            }
            content.extend_from_slice(&chunk[..count]);
        }
        check(cancelled)?;
        self.source.verify_path().map_err(storage_error)?;
        if content.len() != self.bytes || crate::hash::sha256_hex(&content) != sha256 {
            return Err(Error::new(
                ErrorCode::Conflict,
                "Downloaded bytes do not match the declared length and hash",
            ));
        }
        check(cancelled)?;
        let sealed = Arc::new(
            OwnedFile::create(self.path().parent().unwrap(), "sealed").map_err(storage_error)?,
        );
        let mut output = sealed.file();
        for chunk in content.chunks(64 * 1024) {
            check(cancelled)?;
            output.write_all(chunk).map_err(storage_error)?;
        }
        output.sync_all().map_err(storage_error)?;
        drop(content);
        check(cancelled)?;
        sealed.verify_path().map_err(storage_error)?;
        let fingerprint = SourceFingerprint::capture(sealed.path()).map_err(plan_error)?;
        let mut desired = directory
            .snapshot
            .entries()
            .iter()
            .map(|entry| DesiredEntry::existing(entry, entry.path.clone()))
            .collect::<Vec<_>>();
        desired.push(DesiredEntry::transfer(
            sealed.path(),
            relative,
            EntryKind::File,
            TransferMode::Copy,
            fingerprint,
        ));
        let mut limits = filesystem::OPERATION_LIMITS;
        limits.bytes = MAX_BYTES as u64;
        let mut plan = FsPlan::build_bounded(
            directory.path.clone(),
            directory.snapshot.clone(),
            desired,
            limits,
        )
        .map_err(plan_error)?;
        plan.retain_source(sealed, "downloaded file")
            .map_err(plan_error)?;
        check(cancelled)?;
        Ok(plan)
    }
}

fn plan_error(error: anyhow::Error) -> Error {
    let code = if error.is::<crate::fs_plan::OperationLimitExceeded>()
        || error.is::<crate::fs_plan::DirectoryLimitExceeded>()
    {
        ErrorCode::LimitExceeded
    } else {
        ErrorCode::Conflict
    };
    Error::new(
        code,
        "Download destination could not be prepared; refresh and retry",
    )
}

#[cfg(test)]
#[path = "tests/staging.rs"]
mod tests;
