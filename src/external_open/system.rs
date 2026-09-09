// SPDX-License-Identifier: MPL-2.0

//! A fixed platform handler for explicitly authorized URLs and workspace files.
//! Prepare and launch run off the editor loop; neither consults the program cache.
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

pub const MAX_TARGET_BYTES: usize = 4096;
const MAX_OPENERS: usize = 16;

pub enum Target {
    Url(String),
    File(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidTarget,
    Unavailable,
    Busy,
    OutcomeUnknown,
}
impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidTarget => "External target is invalid or outside the workspace",
            Self::Unavailable => "System opener is unavailable",
            Self::Busy => "System opener limit reached",
            Self::OutcomeUnknown => {
                "System opener startup outcome is unknown; do not retry automatically"
            }
        })
    }
}
impl std::error::Error for Error {}

/// A validated single argument. The caller must recheck foreground authority
/// before consuming it; preparing a target does not authorize a launch.
pub struct Prepared {
    argument: OsString,
}
impl fmt::Debug for Prepared {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A browser URL can carry an authentication code in its query.
        formatter
            .debug_struct("PreparedSystemOpen")
            .finish_non_exhaustive()
    }
}

/// Validates a URL or resolves an existing regular file inside the workspace.
/// Filesystem calls belong on a worker, including canonicalizing the root.
pub fn prepare(root: &Path, target: Target) -> Result<Prepared, Error> {
    let argument = match target {
        Target::Url(text) => {
            validate_url(&text)?;
            OsString::from(text)
        }
        Target::File(text) => {
            if !safe_target(&text) {
                return Err(Error::InvalidTarget);
            }
            let root = root.canonicalize().map_err(|_| Error::InvalidTarget)?;
            let file = root
                .join(text)
                .canonicalize()
                .map_err(|_| Error::InvalidTarget)?;
            if !file.starts_with(&root) || !file.is_file() {
                return Err(Error::InvalidTarget);
            }
            file.into_os_string()
        }
    };
    Ok(Prepared { argument })
}
fn safe_target(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_TARGET_BYTES && !text.chars().any(char::is_control)
}

/// The URL parser checks host/port/IP syntax; raw checks also refuse ambiguous
/// whitespace, backslashes, and empty user-info that normalization could erase.
fn validate_url(text: &str) -> Result<(), Error> {
    if !safe_target(text) || text.chars().any(char::is_whitespace) || text.contains('\\') {
        return Err(Error::InvalidTarget);
    }
    let (scheme, remainder) = text.split_once("://").ok_or(Error::InvalidTarget)?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(Error::InvalidTarget);
    }
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(Error::InvalidTarget);
    }
    let url = url::Url::parse(text).map_err(|_| Error::InvalidTarget)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::InvalidTarget);
    }
    Ok(())
}

impl Prepared {
    /// Returns once the operating system accepts the handler spawn. It does
    /// not claim the handler opened a browser or that navigation succeeded.
    /// The child has independent stdio/process-group ownership and is reaped
    /// without being stopped when an application or editor handle disappears.
    pub fn launch(self) -> Result<(), Error> {
        let executable =
            platform_handler(super::OpenPlatform::CURRENT).ok_or(Error::Unavailable)?;
        launch_with(self, OsStr::new(executable), active_openers())
    }

    #[cfg(test)]
    pub(crate) fn launch_for_test(self, executable: &OsStr) -> Result<(), Error> {
        launch_with(self, executable, active_openers())
    }
}

fn active_openers() -> Arc<AtomicUsize> {
    static ACTIVE: OnceLock<Arc<AtomicUsize>> = OnceLock::new();
    ACTIVE.get_or_init(|| Arc::new(AtomicUsize::new(0))).clone()
}
fn platform_handler(platform: super::OpenPlatform) -> Option<&'static str> {
    match platform {
        super::OpenPlatform::Linux => Some("xdg-open"),
        super::OpenPlatform::MacOs => Some("/usr/bin/open"),
        super::OpenPlatform::Unsupported => None,
    }
}

struct Slot(Arc<AtomicUsize>);
impl Slot {
    fn reserve(active: Arc<AtomicUsize>) -> Result<Self, Error> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_OPENERS).then_some(count + 1)
            })
            .map_err(|_| Error::Busy)?;
        Ok(Self(active))
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn launch_with(
    prepared: Prepared,
    executable: &OsStr,
    active: Arc<AtomicUsize>,
) -> Result<(), Error> {
    launch_using(prepared, executable, active, |work| {
        std::thread::Builder::new()
            .name("system-opener".into())
            .spawn(work)
            .map(|_| ())
    })
}

type ReapWork = Box<dyn FnOnce() + Send>;
fn launch_using(
    prepared: Prepared,
    executable: &OsStr,
    active: Arc<AtomicUsize>,
    start_thread: impl FnOnce(ReapWork) -> std::io::Result<()>,
) -> Result<(), Error> {
    let slot = Slot::reserve(active)?;
    let executable = executable.to_owned();
    let (sender, receiver) = mpsc::sync_channel(1);
    // Create the reaper before the child: failure to create a thread cannot
    // leave a spawned child without a wait owner. There are at most 16 such
    // threads, including ones waiting on handlers that choose to stay alive.
    start_thread(Box::new(move || {
        let _slot = slot;
        let mut command = Command::new(executable);
        command
            .arg(prepared.argument)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        command.process_group(0);
        match command.spawn() {
            Ok(mut child) => {
                let _ = sender.send(Ok(()));
                let _ = child.wait();
            }
            Err(_) => {
                let _ = sender.send(Err(Error::Unavailable));
            }
        }
    }))
    .map_err(|_| Error::Unavailable)?;
    // The independent owner may still finish a slow spawn after this caller
    // stops waiting. Its slot remains held through actual wait/reap; timing
    // out does not cancel or retry an already authorized external handoff.
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap_or(Err(Error::OutcomeUnknown))
}

#[cfg(test)]
#[path = "tests/system.rs"]
mod tests;
