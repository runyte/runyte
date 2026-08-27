// SPDX-License-Identifier: MPL-2.0

//! Presentation-neutral health reports for optional editor services.
//!
//! A report is a point-in-time value. Rendering and picker code should not
//! probe processes or the filesystem; the application coordinator gathers
//! those facts before opening the surface and owns the resulting snapshot.

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use crate::command::{CommandCapability, CommandSpec};

/// Contextual command availability captured once for a palette projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandAvailability {
    Available,
    Unavailable(String),
}

/// Reason shared by palette projection and execution for Unix-only persistent
/// session commands.
pub const PERSISTENT_SESSION_UNSUPPORTED_REASON: &str =
    "persistent mode is not supported on this platform";

/// Reason a standalone workspace cannot answer a `session` command.
///
/// A standalone editor owns no durable host, so the whole namespace — the
/// manager included — has nothing to address. Naming the setting rather than
/// the mode says what to change.
pub const PERSISTENT_SESSION_STANDALONE_REASON: &str = "needs workspace.mode: persistent";

/// Projects the persistent-session boundary through injectable values so both
/// halves of the policy can be covered on a Unix development host.
///
/// The platform answer comes first: on a build without persistent sessions the
/// mode is not a thing the reader can change, so naming the mode would send
/// them after a setting that would not help.
pub fn persistent_session_availability(
    platform_supports_persistent_sessions: bool,
    workspace_is_persistent: bool,
) -> CommandAvailability {
    if !platform_supports_persistent_sessions {
        return CommandAvailability::Unavailable(PERSISTENT_SESSION_UNSUPPORTED_REASON.to_owned());
    }
    if !workspace_is_persistent {
        return CommandAvailability::Unavailable(PERSISTENT_SESSION_STANDALONE_REASON.to_owned());
    }
    CommandAvailability::Available
}

impl CommandAvailability {
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Available => None,
            Self::Unavailable(reason) => Some(reason),
        }
    }
}

/// The application facts needed to project contextual palette rows.
///
/// The value owns every reason so one call to `App::matching_commands` cannot
/// mix service states observed at different moments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppCapabilitySnapshot {
    pub syntax: CommandAvailability,
    pub lsp_manager: CommandAvailability,
    pub lsp_document: CommandAvailability,
    pub git_project: CommandAvailability,
    pub persistent_session: CommandAvailability,
}

impl AppCapabilitySnapshot {
    pub fn command_availability(&self, spec: &CommandSpec) -> CommandAvailability {
        match spec.capability() {
            Some(capability) => self.capability_availability(capability),
            None => CommandAvailability::Available,
        }
    }

    pub fn capability_availability(&self, capability: CommandCapability) -> CommandAvailability {
        match capability {
            CommandCapability::Syntax => self.syntax.clone(),
            CommandCapability::LspDocument => self.lsp_document.clone(),
            CommandCapability::LspManager => self.lsp_manager.clone(),
            CommandCapability::GitProject => self.git_project.clone(),
            CommandCapability::PersistentSession => self.persistent_session.clone(),
        }
    }
}

/// Coarse state shared by syntax and language-server rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceState {
    Ready,
    Idle,
    Degraded,
    Disabled,
    Unavailable,
}

impl ServiceState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Idle => "idle",
            Self::Degraded => "degraded",
            Self::Disabled => "disabled",
            Self::Unavailable => "unavailable",
        }
    }
}

/// One independently useful fact in a service-health report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceHealthEntry {
    pub service: &'static str,
    pub state: ServiceState,
    pub detail: String,
}

impl ServiceHealthEntry {
    pub fn new(service: &'static str, state: ServiceState, detail: impl Into<String>) -> Self {
        Self {
            service,
            state,
            detail: detail.into(),
        }
    }
}

/// Owned point-in-time report suitable for a TUI, headless host, or RPC.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ServiceHealthSnapshot {
    pub entries: Vec<ServiceHealthEntry>,
}

/// Resolves a configured executable without starting it.
///
/// Explicit paths are inspected exactly. Bare commands are searched through
/// the supplied PATH value so tests and headless callers need not mutate the
/// process environment.
pub fn resolve_configured_executable(
    command: &Path,
    search_path: Option<&OsStr>,
) -> Option<PathBuf> {
    if command.is_absolute() || command.components().count() > 1 {
        return executable(command).then(|| command.to_path_buf());
    }
    search_directories(search_path?)
        .map(|directory| directory.join(command))
        .find(|candidate| executable(candidate))
}

/// The directories a PATH value actually names.
///
/// Empty entries — from a leading, trailing, or doubled separator — are
/// dropped. A shell reads those as the working directory, but joining one
/// produces a bare relative path, and Runyte's working directory is the
/// project it was opened on. Honouring it would let a repository supply a
/// executable by name.
pub fn search_directories(search_path: &OsStr) -> impl Iterator<Item = PathBuf> + '_ {
    std::env::split_paths(search_path).filter(|directory| !directory.as_os_str().is_empty())
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "runyte-service-health-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn executable_lookup_is_injected_and_does_not_start_the_program() {
        let root = temporary("path");
        fs::create_dir_all(&root).unwrap();
        let tool = root.join("test-tool");
        fs::write(&tool, "#!/bin/sh\nexit 99\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        }

        assert_eq!(
            resolve_configured_executable(Path::new("test-tool"), Some(root.as_os_str())),
            Some(tool.clone())
        );
        assert_eq!(
            resolve_configured_executable(Path::new("missing-tool"), Some(root.as_os_str())),
            None
        );

        fs::remove_dir_all(root).unwrap();
    }

    /// A leading, trailing, or doubled separator leaves an empty PATH entry.
    /// Joining one yields a bare relative path, which would resolve against
    /// the project Runyte was opened on rather than a search directory.
    #[test]
    fn empty_path_entries_name_no_search_directory() {
        let joined = search_directories(OsStr::new(":/first::/second:"))
            .map(|directory| directory.join("test-tool"))
            .collect::<Vec<_>>();

        assert_eq!(
            joined,
            vec![
                PathBuf::from("/first/test-tool"),
                PathBuf::from("/second/test-tool"),
            ]
        );
        assert!(
            joined.iter().all(|candidate| candidate.is_absolute()),
            "a relative candidate would resolve against the project"
        );
        assert_eq!(search_directories(OsStr::new("")).count(), 0);
        assert_eq!(search_directories(OsStr::new("::")).count(), 0);
    }

    #[test]
    fn explicit_missing_paths_are_unavailable_without_consulting_path() {
        let root = temporary("explicit");
        let missing = root.join("missing");
        assert_eq!(
            resolve_configured_executable(&missing, Some(OsStr::new("/bin"))),
            None
        );
    }

    #[test]
    fn one_capability_snapshot_drives_syntax_and_lsp_commands() {
        let snapshot = AppCapabilitySnapshot {
            syntax: CommandAvailability::Unavailable("plain text buffer".to_owned()),
            lsp_manager: CommandAvailability::Available,
            lsp_document: CommandAvailability::Unavailable("no configured server".to_owned()),
            git_project: CommandAvailability::Unavailable("not a Git repository".to_owned()),
            persistent_session: CommandAvailability::Unavailable(
                PERSISTENT_SESSION_UNSUPPORTED_REASON.to_owned(),
            ),
        };
        let outline = crate::command::resolve_command("outline").unwrap();
        let status = crate::command::resolve_command("lsp-status").unwrap();
        let format = crate::command::resolve_command("format").unwrap();
        let git_status = crate::command::resolve_command("git-status").unwrap();
        let session_attach = crate::command::resolve_command("session-attach").unwrap();

        assert_eq!(
            snapshot.command_availability(outline).reason(),
            Some("plain text buffer")
        );
        assert!(snapshot.command_availability(status).is_available());
        assert_eq!(
            snapshot.command_availability(format).reason(),
            Some("no configured server")
        );
        assert_eq!(
            snapshot.command_availability(git_status).reason(),
            Some("not a Git repository")
        );
        assert_eq!(
            snapshot.command_availability(session_attach).reason(),
            Some(PERSISTENT_SESSION_UNSUPPORTED_REASON)
        );
    }

    #[test]
    fn session_capability_answers_platform_first_then_mode() {
        assert!(persistent_session_availability(true, true).is_available());
        assert_eq!(
            persistent_session_availability(true, false).reason(),
            Some(PERSISTENT_SESSION_STANDALONE_REASON)
        );
        // A build without persistent sessions has no mode to change, so the
        // platform answer wins whichever mode is configured.
        assert_eq!(
            persistent_session_availability(false, true).reason(),
            Some(PERSISTENT_SESSION_UNSUPPORTED_REASON)
        );
        assert_eq!(
            persistent_session_availability(false, false).reason(),
            Some(PERSISTENT_SESSION_UNSUPPORTED_REASON)
        );
    }
}
