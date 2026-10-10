// SPDX-License-Identifier: MPL-2.0
//! Same-build helper launch and process-tree ownership for desktop services.
use anyhow::{Context, Result};
use std::{
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Pdf,
    Preview,
}
impl Role {
    pub fn parse(value: &OsStr) -> Result<Self> {
        match value.to_str() {
            Some("pdf") => Ok(Self::Pdf),
            Some("preview") => Ok(Self::Preview),
            _ => anyhow::bail!("unknown helper role: {}", value.to_string_lossy()),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Preview => "preview",
        }
    }
}

#[derive(Debug)]
pub(crate) struct Updated(Role);
impl std::fmt::Display for Updated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Runyte was updated on disk; restart it to use {}",
            match self.0 {
                Role::Pdf => "PDF viewing",
                Role::Preview => "document preview",
            }
        )
    }
}
impl std::error::Error for Updated {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    size: u64,
    modified: std::time::SystemTime,
}
impl Identity {
    fn read(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            size: metadata.len(),
            modified: metadata.modified()?,
        })
    }
}
static EXECUTABLE: OnceLock<Result<(PathBuf, Identity), String>> = OnceLock::new();

/// Capture before the editor starts; helper-role dispatch deliberately precedes this.
pub fn initialize() {
    EXECUTABLE.get_or_init(|| {
        let path = std::env::current_exe().map_err(|e| e.to_string())?;
        let identity = Identity::read(&path).map_err(|e| e.to_string())?;
        Ok((path, identity))
    });
}

fn checked_path(
    role: Role,
    path: &Path,
    saved: &Identity,
    read: impl FnOnce(&Path) -> std::io::Result<Identity>,
) -> Result<PathBuf> {
    if read(path).as_ref().ok() != Some(saved) {
        return Err(Updated(role).into());
    }
    Ok(path.to_owned())
}

pub(crate) fn command(role: Role) -> Result<Command> {
    #[cfg(target_os = "linux")]
    if std::fs::metadata("/proc/self/exe").is_ok() {
        return Ok(command_at(role, Path::new("/proc/self/exe")));
    }
    let (path, saved) = EXECUTABLE
        .get()
        .context("desktop executable identity was not captured")?
        .as_ref()
        .map_err(|error| anyhow::anyhow!("cannot identify desktop executable: {error}"))?;
    Ok(command_at(
        role,
        &checked_path(role, path, saved, Identity::read)?,
    ))
}
fn command_at(role: Role, path: &Path) -> Command {
    let mut command = Command::new(path);
    command.args(["--helper", role.name()]);
    command
}

pub(crate) struct Process {
    pub child: Child,
    pub errors: Arc<Mutex<Vec<u8>>>,
    reader: Option<std::thread::JoinHandle<()>>,
    #[cfg(unix)]
    group: runyte::process_group::OwnedGroup,
}
impl Process {
    pub fn spawn(command: &mut Command, role: Role) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
            #[cfg(target_os = "linux")]
            if role == Role::Preview {
                // SAFETY: only an async-signal-safe resource-limit syscall runs after fork.
                unsafe {
                    command.pre_exec(|| {
                        let memory = libc::rlimit {
                            rlim_cur: 2 * 1024 * 1024 * 1024,
                            rlim_max: 2 * 1024 * 1024 * 1024,
                        };
                        if libc::setrlimit(libc::RLIMIT_AS, &memory) != 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
            }
        }
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("start {} helper", role.name()))?;
        #[cfg(unix)]
        let group = {
            use runyte::process_group::{self, GroupAnchor, Site};
            process_group::record_spawn(role.name(), "desktop helper", child.id());
            process_group::claim_anchored_group(
                Site::new(role.name(), "helper cleanup"),
                child.id() as libc::pid_t,
                GroupAnchor::RunningLeader,
            )
        };
        // Own cleanup before taking pipes or starting fallible reader threads.
        let mut process = Self {
            child,
            errors: Arc::default(),
            reader: None,
            #[cfg(unix)]
            group,
        };
        let mut stderr = process.child.stderr.take().unwrap();
        let retained = process.errors.clone();
        process.reader = Some(
            std::thread::Builder::new()
                .name("runyte-helper-errors".into())
                .spawn(move || {
                    let mut block = [0; 4096];
                    while let Ok(count) = stderr.read(&mut block) {
                        if count == 0 {
                            break;
                        }
                        let mut bytes = retained.lock().unwrap();
                        let remaining = 4096usize.saturating_sub(bytes.len());
                        bytes.extend_from_slice(&block[..count.min(remaining)]);
                    }
                })?,
        );
        Ok(process)
    }
    pub fn status(&mut self) -> std::io::Result<Option<ExitStatus>> {
        #[cfg(unix)]
        {
            runyte::process_group::completed_without_reaping(&self.child)
        }
        #[cfg(not(unix))]
        {
            self.child.try_wait()
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        // Never reap before signalling: the unreaped leader anchors the group ID.
        #[cfg(unix)]
        self.group.signal(libc::SIGKILL);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roles_and_argument_vectors_are_private_and_exact() {
        for (name, role) in [("pdf", Role::Pdf), ("preview", Role::Preview)] {
            assert_eq!(Role::parse(OsStr::new(name)).unwrap(), role);
            let mut command = command_at(role, Path::new("/editor"));
            command.arg("a path with spaces");
            assert_eq!(command.get_program(), "/editor");
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                ["--helper", name, "a path with spaces"]
            );
        }
        assert!(Role::parse(OsStr::new("unknown")).is_err());
    }
    #[test]
    fn replacement_and_missing_identity_refuse_to_spawn() {
        let original = Identity {
            #[cfg(unix)]
            device: 1,
            #[cfg(unix)]
            inode: 2,
            size: 10,
            modified: std::time::SystemTime::UNIX_EPOCH,
        };
        for role in [Role::Pdf, Role::Preview] {
            let path = Path::new("/editor");
            assert_eq!(
                checked_path(role, path, &original, |_| Ok(original.clone())).unwrap(),
                path
            );
            let mut changed = original.clone();
            changed.size += 1;
            let error = checked_path(role, path, &original, |_| Ok(changed)).unwrap_err();
            assert!(error.is::<Updated>());
            assert_eq!(error.to_string(), Updated(role).to_string());
            assert!(
                checked_path(role, path, &original, |_| Err(
                    std::io::ErrorKind::NotFound.into()
                ))
                .unwrap_err()
                .is::<Updated>()
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn child_has_its_own_group_and_stderr_is_bounded_but_drained() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "head -c 16384 /dev/zero >&2; printf done"]);
        let config = tempfile::tempdir().unwrap();
        command.env("XDG_CONFIG_HOME", config.path());
        let mut process = Process::spawn(&mut command, Role::Pdf).unwrap();
        // SAFETY: only query the fixture's live process group.
        assert_eq!(
            unsafe { libc::getpgid(process.child.id() as libc::pid_t) },
            process.child.id() as libc::pid_t
        );
        process.child.stdin.take();
        let mut output = String::new();
        process
            .child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert_eq!(output, "done");
        let errors = process.errors.clone();
        drop(process);
        assert_eq!(errors.lock().unwrap().len(), 4096);
    }
}
