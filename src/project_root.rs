// SPDX-License-Identifier: MPL-2.0

//! Project-root discovery and the explicit non-Git fallback.
//!
//! Runtime state belongs to one workspace, not to whichever nested directory
//! happened to launch the editor. Discovery therefore prefers the nearest
//! version-control root: Git, Jujutsu or Subversion. Only when there is none
//! does an existing configured state directory (normally `.runyte`) identify
//! the root. An ide launch that finds neither makes the launch directory a
//! workspace, as Helix uses its working directory, unless that directory is
//! one no launch should adopt without being asked; see [`adopt`].

use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[cfg(windows)]
use std::path::Component;

use anyhow::{Context, Result, bail};

/// How startup selected a workspace. Discovery records the matching marker
/// during its existing scan, without looking at the filesystem again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceOrigin {
    VersionControl(VersionControl),
    StateDirectory,
    Created(CreatedBy),
}

pub(crate) const STARTUP_ORIGIN_ENV: &str = "RUNYTE_STARTUP_WORKSPACE_ORIGIN";

impl WorkspaceOrigin {
    pub(crate) fn startup_value(self) -> &'static str {
        match self {
            Self::VersionControl(VersionControl::Git) => "git",
            Self::VersionControl(VersionControl::Jujutsu) => "jj",
            Self::VersionControl(VersionControl::Subversion) => "svn",
            Self::StateDirectory => "state",
            Self::Created(CreatedBy::Adoption) => "adoption",
            Self::Created(CreatedBy::Init) => "init",
            Self::Created(CreatedBy::AttachDirectory) => "attach-directory",
        }
    }

    pub(crate) fn startup_handoff(self, root: &Path) -> String {
        format!(
            "{}:{}",
            crate::workspace::workspace_id(root),
            self.startup_value()
        )
    }

    pub(crate) fn from_startup_value(value: &str) -> Option<Self> {
        Some(match value {
            "git" => Self::VersionControl(VersionControl::Git),
            "jj" => Self::VersionControl(VersionControl::Jujutsu),
            "svn" => Self::VersionControl(VersionControl::Subversion),
            "state" => Self::StateDirectory,
            "adoption" => Self::Created(CreatedBy::Adoption),
            "init" => Self::Created(CreatedBy::Init),
            "attach-directory" => Self::Created(CreatedBy::AttachDirectory),
            _ => return None,
        })
    }
}

pub(crate) fn startup_origin(
    root: &Path,
    state: &Path,
    inherited: Option<&str>,
) -> io::Result<WorkspaceOrigin> {
    let captured = inherited
        .filter(|value| value.len() <= 64)
        .and_then(|value| {
            let (identity, origin) = value.split_once(':')?;
            (identity == crate::workspace::workspace_id(root))
                .then(|| WorkspaceOrigin::from_startup_value(origin))
                .flatten()
        });
    match captured {
        Some(origin) => Ok(origin),
        None => exact_origin(root, state),
    }
}

/// Exact explicitly selected root, as opposed to discovery from a launch
/// directory. Used before initialization to retain whether this launch made it.
pub(crate) fn exact_origin(root: &Path, state: &Path) -> io::Result<WorkspaceOrigin> {
    Ok(discover_candidates(&[root], state)?.map_or(
        WorkspaceOrigin::Created(CreatedBy::AttachDirectory),
        |workspace| workspace.origin,
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionControl {
    Git,
    Jujutsu,
    Subversion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreatedBy {
    Adoption,
    Init,
    AttachDirectory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredWorkspace {
    pub root: PathBuf,
    pub origin: WorkspaceOrigin,
}

/// The workspace mode and the source that selected it for this launch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceRequirement {
    pub mode: WorkspaceMode,
    pub source: ModeSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceMode {
    Ide,
    Mux,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModeSource {
    Flag(&'static str),
    Setting,
    PositionalTargets,
    WindowLaunch,
}

impl WorkspaceRequirement {
    pub const fn new(mode: WorkspaceMode, explicit: bool) -> Self {
        Self {
            mode,
            source: if explicit {
                ModeSource::Flag(match mode {
                    WorkspaceMode::Ide => "--ide",
                    WorkspaceMode::Mux => "--mux",
                })
            } else {
                ModeSource::Setting
            },
        }
    }

    /// One refusal structure for all launch and attachment paths. `home` is
    /// supplied by the caller, so formatting never resolves per-user storage.
    pub fn refusal(self, directory: &Path, reason: Option<&str>, home: Option<&Path>) -> String {
        let mode = match self.mode {
            WorkspaceMode::Ide => "ide",
            WorkspaceMode::Mux => "mux",
        };
        let source = match self.source {
            ModeSource::Flag(flag) => format!("the {flag} flag"),
            ModeSource::Setting => "the mode setting".to_owned(),
            ModeSource::PositionalTargets => {
                "file or directory targets override mode: mux".to_owned()
            }
            ModeSource::WindowLaunch => "window launch uses ide instead of mode: mux".to_owned(),
        };
        let mut message = format!(
            "runyte needs a workspace in {mode} mode ({source}), and none was found in {} or any directory above it.",
            display_workspace_path(directory, home)
        );
        if let Some(reason) = reason {
            message.push('\n');
            message.push_str(reason);
        }
        match self.mode {
            WorkspaceMode::Mux => message.push_str("\n  Open or create one:  runyte --mux DIRECTORY\n  Create one only:     runyte --init DIRECTORY"),
            WorkspaceMode::Ide => {
                message.push_str(&format!("\n  Open a project:      {}\n  Create one:          runyte --init DIRECTORY\n  Edit without one:    runyte --editor", ide_open_command("DIRECTORY")));
            },
        }
        message
    }
}

pub fn display_workspace_path(path: &Path, home: Option<&Path>) -> String {
    #[cfg(windows)]
    fn ordinary(path: &Path) -> PathBuf {
        let spelling = path.to_string_lossy();
        if let Some(unc) = spelling.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{unc}"))
        } else {
            PathBuf::from(spelling.strip_prefix(r"\\?\").unwrap_or(&spelling))
        }
    }
    #[cfg(windows)]
    let (path, home) = (ordinary(path), home.map(ordinary));
    #[cfg(windows)]
    let (path, home) = (path.as_path(), home.as_deref());
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(relative) if relative.as_os_str().is_empty() => "~".to_owned(),
        Some(relative) => format!("~/{}", relative.display()),
        None => path.display().to_string(),
    }
}

/// Opens the workspace after changing directory successfully. Windows uses
/// PowerShell 5.1 syntax and a literal path, including names containing `[]`.
pub fn ide_open_command(argument: &str) -> String {
    if cfg!(windows) {
        format!("Set-Location -LiteralPath {argument}; if ($?) {{ runyte }}")
    } else {
        format!("cd {argument} && runyte")
    }
}

/// A literal shell argument for the platform's documented shell (POSIX on
/// Unix, PowerShell on Windows). Human-readable path shortening is separate:
/// quoting a tilde would prevent shell expansion.
pub fn workspace_shell_argument(path: &Path) -> String {
    #[cfg(windows)]
    {
        // PowerShell 5.1's filesystem provider needs the ordinary spelling,
        // while workspace identities retain canonical extended paths.
        let ordinary = crate::windows_fs::ordinary_working_directory(path)
            .unwrap_or_else(|_| path.to_path_buf());
        format!("'{}'", ordinary.to_string_lossy().replace('\'', "''"))
    }
    #[cfg(not(windows))]
    {
        format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
    }
}

/// Finds the workspace directory that owns runtime state.
///
/// A version-control root has priority over an existing state directory even
/// when the latter is nearer to `start`. This makes an accidentally-created
/// nested state directory stop attracting later launches once it is inside a
/// checkout.
pub fn discover(
    start: &Path,
    configured_state_root: &Path,
) -> io::Result<Option<DiscoveredWorkspace>> {
    let start = start.canonicalize()?;
    let candidates = start.ancestors().collect::<Vec<_>>();
    discover_candidates(&candidates, configured_state_root)
}

/// Makes `requested` a durable workspace root and returns its canonical path.
///
/// Unlike discovery, initialization deliberately does not inspect ancestors:
/// the directory named by the user is the new workspace boundary. Creating an
/// existing state directory is harmless, so the same command also opens an
/// already-initialized workspace.
pub fn initialize(
    requested: &Path,
    configured_state_root: &Path,
    reserved_user_roots: &[PathBuf],
) -> Result<PathBuf> {
    let safety = crate::workspace_state::StartupSafety::capture(reserved_user_roots);
    initialize_with_safety(requested, configured_state_root, &safety)
}

pub fn initialize_with_safety(
    requested: &Path,
    configured_state_root: &Path,
    safety: &crate::workspace_state::StartupSafety,
) -> Result<PathBuf> {
    let project_root = requested
        .canonicalize()
        .with_context(|| format!("cannot resolve project directory {}", requested.display()))?;
    anyhow::ensure!(
        project_root.is_dir(),
        "project directory {} is not a directory",
        project_root.display()
    );
    let location = resolve_state_location(&project_root, configured_state_root);
    safety.validate(&project_root, &location)?;
    if matches!(location, StateLocation::OutOfTree(_)) {
        location.prepare_with_safety(&project_root, safety)?;
        return Ok(project_root);
    }
    let state_root = location.into_path_buf();
    fs::create_dir_all(&state_root).with_context(|| {
        format!(
            "cannot create the project workspace directory {}",
            state_root.display()
        )
    })?;
    Ok(project_root)
}

fn discover_candidates(
    candidates: &[&Path],
    configured_state_root: &Path,
) -> io::Result<Option<DiscoveredWorkspace>> {
    for candidate in candidates {
        if let Some(version_control) = version_control_root(candidate)? {
            return Ok(Some(DiscoveredWorkspace {
                root: (*candidate).to_path_buf(),
                origin: WorkspaceOrigin::VersionControl(version_control),
            }));
        }
    }

    if is_usable_relative_marker(configured_state_root) {
        for candidate in candidates {
            let runtime_root = candidate.join(configured_state_root);
            if runtime_root.is_dir() {
                return Ok(Some(DiscoveredWorkspace {
                    root: (*candidate).to_path_buf(),
                    origin: WorkspaceOrigin::StateDirectory,
                }));
            }
        }
    }

    Ok(None)
}

/// Makes `directory`, the launch directory of an ide launch that discovery
/// found no workspace for, a workspace of its own, and returns its canonical
/// path. `targets` are the launch's files and directories, joined to the
/// launch directory.
///
/// Some directories are refused rather than adopted, because a state
/// directory in them would be discovered by every later launch from a
/// directory below with no repository, quietly making all of it one
/// workspace: the home directory, as the environment and the account database
/// each name it in `homes`; the filesystem root; the temporary directory; and
/// on Unix a directory other accounts can write to or that another account
/// owns, where one person's state would capture another's launches. A launch
/// naming a target outside the directory is refused too: it is about that
/// file, not about the directory it happened to start in. `--init` still
/// makes any of these a workspace on request. A state directory configured as
/// an absolute path is refused as well: discovery cannot find it again, so
/// every directory adopted with it would share it.
pub fn adopt(
    directory: &Path,
    configured_state_root: &Path,
    reserved_user_roots: &[PathBuf],
    homes: &[PathBuf],
    targets: &[PathBuf],
    requirement: WorkspaceRequirement,
) -> Result<PathBuf> {
    let safety = crate::workspace_state::StartupSafety::capture(reserved_user_roots);
    adopt_with_safety(
        directory,
        configured_state_root,
        &safety,
        homes,
        targets,
        requirement,
    )
}

pub fn adopt_with_safety(
    directory: &Path,
    configured_state_root: &Path,
    safety: &crate::workspace_state::StartupSafety,
    homes: &[PathBuf],
    targets: &[PathBuf],
    requirement: WorkspaceRequirement,
) -> Result<PathBuf> {
    let typed = lexical(directory);
    let directory = directory
        .canonicalize()
        .with_context(|| format!("cannot resolve directory {}", directory.display()))?;
    let is = |candidate: &Path| {
        candidate
            .canonicalize()
            .is_ok_and(|candidate| candidate == directory)
    };
    let outside = targets
        .iter()
        .find(|target| !lexical(target).starts_with(&typed));
    let refusal = if directory.parent().is_none() {
        Some("the filesystem root is not made one automatically".to_owned())
    } else if homes.iter().any(|home| is(home)) {
        Some("the home directory is not made one automatically".to_owned())
    } else if is(&std::env::temp_dir()) {
        Some("the temporary directory is not made one automatically".to_owned())
    } else if let Some(sharing) = shared_directory(&directory)? {
        Some(format!("{sharing} is not made one automatically"))
    } else if let Some(target) = outside {
        Some(format!(
            "{} is outside {}, so that directory is not made one automatically",
            display_workspace_path(target, homes.first().map(PathBuf::as_path)),
            display_workspace_path(&directory, homes.first().map(PathBuf::as_path))
        ))
    } else if !is_usable_relative_marker(configured_state_root) {
        Some("none is made automatically while workspace.state is an absolute path".to_owned())
    } else {
        None
    };
    if let Some(refusal) = refusal {
        bail!(requirement.refusal(
            &directory,
            Some(&refusal),
            homes.first().map(PathBuf::as_path)
        ));
    }
    initialize_with_safety(&directory, configured_state_root, safety).map_err(|error| {
        let reason = format!("The launch directory could not be made a workspace: {error:#}.");
        error.context(requirement.refusal(
            &directory,
            Some(&reason),
            homes.first().map(PathBuf::as_path),
        ))
    })
}

/// `path` with `.` dropped and each `..` taking away the name before it, as
/// it was typed rather than where links lead: a target is inside the launch
/// directory when it was named there, even through a link or before its
/// directory exists.
fn lexical(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            component => resolved.push(component),
        }
    }
    resolved
}

/// Why other accounts share `directory`, when they do.
#[cfg(unix)]
fn shared_directory(directory: &Path) -> io::Result<Option<&'static str>> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::metadata(directory).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("cannot read {}: {error}", directory.display()),
        )
    })?;
    // SAFETY: `geteuid` reads the process's effective user and cannot fail.
    let owner = unsafe { libc::geteuid() };
    Ok(if metadata.mode() & 0o002 != 0 {
        Some("a directory other accounts can write to")
    } else if metadata.uid() != owner {
        Some("a directory another account owns")
    } else {
        None
    })
}

#[cfg(not(unix))]
fn shared_directory(_directory: &Path) -> io::Result<Option<&'static str>> {
    Ok(None)
}

/// Resolves the configured runtime path against its owning project directory.
pub fn resolve_state_root(project_root: &Path, configured_state_root: &Path) -> PathBuf {
    resolve_state_location(project_root, configured_state_root).into_path_buf()
}

/// Resolves placement once. Ordinary workspaces stay in-tree; #73 will supply
/// home-workspace selection here. Debug fixtures can select one exact project.
pub fn resolve_state_location(project_root: &Path, configured: &Path) -> StateLocation {
    #[cfg(debug_assertions)]
    if std::env::var_os("RUNYTE_TEST_OUT_OF_TREE_ROOT")
        .is_some_and(|root| Path::new(&root) == project_root)
        && let Some(root) = crate::workspace_state::fixture_state_root()
    {
        return StateLocation::out_of_tree(project_root, &root);
    }
    StateLocation::in_tree(project_root, configured)
}

pub use crate::workspace_state::StateLocation;

/// Keeps project journals and artifacts separate from per-user configuration
/// and caches. Both directions matter: neither side may contain the other.
pub fn validate_state_root(state_root: &Path, reserved_user_roots: &[PathBuf]) -> Result<()> {
    if let Some(reserved) = reserved_user_roots
        .iter()
        .find(|reserved| paths_overlap(state_root, reserved))
    {
        bail!(
            "{} overlaps per-user Runyte storage at {} and cannot store project data",
            state_root.display(),
            reserved.display()
        );
    }
    Ok(())
}

fn is_usable_relative_marker(path: &Path) -> bool {
    !path.is_absolute() && !path.as_os_str().is_empty() && path != Path::new(".")
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    comparable_paths_overlap(&comparable_path(left), &comparable_path(right))
}

#[cfg(windows)]
pub(crate) fn comparable_path(path: &Path) -> PathBuf {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                for component in suffix.iter().rev() {
                    resolved.push(component);
                }
                break resolved;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(name) = ancestor.file_name() else {
                    break path.to_path_buf();
                };
                suffix.push(name.to_owned());
                let Some(parent) = ancestor.parent() else {
                    break path.to_path_buf();
                };
                ancestor = parent;
            }
            Err(_) => break path.to_path_buf(),
        }
    }
}

pub(crate) fn comparable_paths_overlap(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::{os::windows::ffi::OsStrExt, path::Prefix};
        let left: Vec<_> = left.components().collect();
        let right: Vec<_> = right.components().collect();
        let component_eq = |left: &Component<'_>, right: &Component<'_>| match (left, right) {
            (Component::Prefix(left), Component::Prefix(right)) => {
                match (left.kind(), right.kind()) {
                    (
                        Prefix::Disk(left) | Prefix::VerbatimDisk(left),
                        Prefix::Disk(right) | Prefix::VerbatimDisk(right),
                    ) => left.eq_ignore_ascii_case(&right),
                    (
                        Prefix::UNC(left_server, left_share),
                        Prefix::UNC(right_server, right_share),
                    )
                    | (
                        Prefix::VerbatimUNC(left_server, left_share),
                        Prefix::VerbatimUNC(right_server, right_share),
                    )
                    | (
                        Prefix::UNC(left_server, left_share),
                        Prefix::VerbatimUNC(right_server, right_share),
                    )
                    | (
                        Prefix::VerbatimUNC(left_server, left_share),
                        Prefix::UNC(right_server, right_share),
                    ) => {
                        crate::windows_fs::compare_names(
                            &left_server.encode_wide().collect::<Vec<_>>(),
                            &right_server.encode_wide().collect::<Vec<_>>(),
                        )
                        .is_eq()
                            && crate::windows_fs::compare_names(
                                &left_share.encode_wide().collect::<Vec<_>>(),
                                &right_share.encode_wide().collect::<Vec<_>>(),
                            )
                            .is_eq()
                    }
                    _ => false,
                }
            }
            (Component::RootDir, Component::RootDir) => true,
            (Component::Normal(left), Component::Normal(right)) => {
                crate::windows_fs::compare_names(
                    &left.encode_wide().collect::<Vec<_>>(),
                    &right.encode_wide().collect::<Vec<_>>(),
                )
                .is_eq()
            }
            _ => false,
        };
        let starts_with = |path: &[Component<'_>], root: &[Component<'_>]| {
            path.len() >= root.len()
                && root
                    .iter()
                    .zip(path)
                    .all(|(root, path)| component_eq(root, path))
        };
        starts_with(&left, &right) || starts_with(&right, &left)
    }

    #[cfg(not(windows))]
    {
        left.starts_with(right) || right.starts_with(left)
    }
}

#[cfg(not(windows))]
pub(crate) fn comparable_path(path: &Path) -> PathBuf {
    // A future state directory may have several absent parents. Resolve the
    // existing ancestor before comparing it with per-user storage, otherwise
    // a symlink above those parents hides their eventual containment.
    for ancestor in path.ancestors() {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                // This is a comparison of prospective storage locations:
                // create_dir_all makes an absent component traversable before
                // a following `..`. Re-entering an existing directory can
                // expose another symlink later in the suffix, so resolve each
                // component instead of cancelling parents in the original path.
                for component in path.strip_prefix(ancestor).unwrap().components() {
                    match component {
                        std::path::Component::ParentDir => {
                            resolved.pop();
                        }
                        std::path::Component::CurDir => {}
                        _ => {
                            resolved.push(component);
                            match resolved.canonicalize() {
                                Ok(canonical) => resolved = canonical,
                                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                                Err(_) => return path.to_path_buf(),
                            }
                        }
                    }
                }
                return resolved;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => break,
        }
    }
    path.to_path_buf()
}

fn version_control_root(candidate: &Path) -> io::Result<Option<VersionControl>> {
    if is_git_root(candidate)? {
        Ok(Some(VersionControl::Git))
    } else if is_marked_directory(candidate, ".jj", "repo")? {
        Ok(Some(VersionControl::Jujutsu))
    } else if is_marked_directory(candidate, ".svn", "wc.db")? {
        Ok(Some(VersionControl::Subversion))
    } else {
        Ok(None)
    }
}

/// Whether `candidate` holds a `name` directory with an `entry` inside it.
/// An empty `.jj` or `.svn` left behind is not a working copy, just as an
/// empty `.git` is not a repository.
fn is_marked_directory(candidate: &Path, name: &str, entry: &str) -> io::Result<bool> {
    let marker = candidate.join(name);
    match fs::metadata(&marker) {
        Ok(metadata) if metadata.is_dir() => Ok(fs::symlink_metadata(marker.join(entry)).is_ok()),
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn is_git_root(candidate: &Path) -> io::Result<bool> {
    let marker = candidate.join(".git");
    let metadata = match fs::metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if metadata.is_dir() {
        return Ok(candidate.join(".git/HEAD").is_file());
    }
    if !metadata.is_file() {
        return Ok(false);
    }

    let mut prefix = [0; 7];
    let read = fs::File::open(marker)?.read(&mut prefix)?;
    Ok(read == prefix.len() && &prefix == b"gitdir:")
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};
    #[cfg(windows)]
    use std::{io, path::PathBuf};

    use super::{adopt, discover, discover_candidates, initialize, validate_state_root};

    #[test]
    fn refusals_have_one_complete_structure_for_every_mode_source_and_reason() {
        for mode in [super::WorkspaceMode::Ide, super::WorkspaceMode::Mux] {
            for explicit in [false, true] {
                for reason in [
                    None,
                    Some("the filesystem root is not made one automatically"),
                    Some("the temporary directory is not made one automatically"),
                    Some("a directory other accounts can write to is not made one automatically"),
                    Some("a directory another account owns is not made one automatically"),
                    Some(
                        "/elsewhere/file is outside ~/notes, so that directory is not made one automatically",
                    ),
                    Some("none is made automatically while workspace.state is an absolute path"),
                ] {
                    let (name, recovery) = match mode {
                        super::WorkspaceMode::Ide if cfg!(windows) => (
                            "ide",
                            "\n  Open a project:      Set-Location -LiteralPath DIRECTORY; if ($?) { runyte }\n  Create one:          runyte --init DIRECTORY\n  Edit without one:    runyte --editor",
                        ),
                        super::WorkspaceMode::Ide => (
                            "ide",
                            "\n  Open a project:      cd DIRECTORY && runyte\n  Create one:          runyte --init DIRECTORY\n  Edit without one:    runyte --editor",
                        ),
                        super::WorkspaceMode::Mux => (
                            "mux",
                            "\n  Open or create one:  runyte --mux DIRECTORY\n  Create one only:     runyte --init DIRECTORY",
                        ),
                    };
                    let source = if explicit {
                        format!("the --{name} flag")
                    } else {
                        "the mode setting".into()
                    };
                    let detail = reason
                        .map(|reason| format!("\n{reason}"))
                        .unwrap_or_default();
                    assert_eq!(
                        super::WorkspaceRequirement::new(mode, explicit).refusal(
                            Path::new("/fixture/home/notes"),
                            reason,
                            Some(Path::new("/fixture/home"))
                        ),
                        format!(
                            "runyte needs a workspace in {name} mode ({source}), and none was found in ~/notes or any directory above it.{detail}{recovery}"
                        )
                    );
                }
            }
        }
        assert_eq!(
            super::display_workspace_path(
                Path::new("/fixture/home"),
                Some(Path::new("/fixture/home"))
            ),
            "~"
        );
        assert_eq!(
            super::display_workspace_path(
                Path::new("/fixture/home-other"),
                Some(Path::new("/fixture/home"))
            ),
            "/fixture/home-other"
        );
    }

    #[cfg(windows)]
    #[test]
    fn canonical_windows_paths_abbreviate_the_same_home_as_ordinary_paths() {
        assert_eq!(
            super::display_workspace_path(
                Path::new(r"\\?\C:\Users\person\notes"),
                Some(Path::new(r"C:\Users\person"))
            ),
            "~/notes"
        );
        assert_eq!(
            super::display_workspace_path(
                Path::new(r"\\?\UNC\server\share\person\notes"),
                Some(Path::new(r"\\server\share\person"))
            ),
            "~/notes"
        );
    }

    #[test]
    fn discovery_retains_the_matching_marker_and_existing_precedence() {
        for (marker, entry, vcs) in [
            (".git", "HEAD", super::VersionControl::Git),
            (".jj", "repo", super::VersionControl::Jujutsu),
            (".svn", "wc.db", super::VersionControl::Subversion),
        ] {
            let root = tempfile();
            fs::create_dir_all(root.join("nested/.runyte")).unwrap();
            fs::create_dir(root.join(marker)).unwrap();
            fs::write(root.join(marker).join(entry), "").unwrap();
            let found = discover(&root.join("nested"), Path::new(".runyte"))
                .unwrap()
                .unwrap();
            assert_eq!(found.root, root);
            assert_eq!(found.origin, super::WorkspaceOrigin::VersionControl(vcs));
            fs::remove_dir_all(&root).unwrap();
        }
        let root = tempfile();
        fs::create_dir(root.join(".runyte")).unwrap();
        assert_eq!(
            discover(&root, Path::new(".runyte"))
                .unwrap()
                .unwrap()
                .origin,
            super::WorkspaceOrigin::StateDirectory
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_root_wins_over_a_nearer_runtime_directory() {
        let temporary = tempfile();
        fs::create_dir(temporary.join(".git")).unwrap();
        fs::write(temporary.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let nested = temporary.join("one/two");
        fs::create_dir_all(nested.join(".runyte")).unwrap();

        assert_eq!(
            discover(&nested, std::path::Path::new(".runyte"))
                .unwrap()
                .map(|workspace| workspace.root),
            Some(temporary.clone())
        );

        fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn existing_runtime_directory_is_the_non_git_fallback() {
        let temporary = tempfile();
        fs::create_dir(temporary.join(".runyte")).unwrap();
        let nested = temporary.join("one/two");
        fs::create_dir_all(&nested).unwrap();

        assert_eq!(
            discover(&nested, std::path::Path::new(".runyte"))
                .unwrap()
                .map(|workspace| workspace.root),
            Some(temporary.clone())
        );

        fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn jujutsu_and_subversion_working_copies_are_project_roots() {
        for (marker, entry) in [(".jj", "repo"), (".svn", "wc.db")] {
            let temporary = tempfile();
            fs::create_dir(temporary.join(marker)).unwrap();
            fs::write(temporary.join(marker).join(entry), "").unwrap();
            let nested = temporary.join("one/two");
            fs::create_dir_all(nested.join(".runyte")).unwrap();

            assert_eq!(
                discover(&nested, std::path::Path::new(".runyte"))
                    .unwrap()
                    .map(|workspace| workspace.root),
                Some(temporary.clone()),
                "{marker}"
            );

            fs::remove_dir_all(temporary).unwrap();
        }
    }

    #[test]
    fn an_empty_jujutsu_or_subversion_directory_is_not_a_project_root() {
        for marker in [".jj", ".svn"] {
            let temporary = tempfile();
            fs::create_dir(temporary.join(marker)).unwrap();

            assert_eq!(
                discover_candidates(&[temporary.as_path()], std::path::Path::new(".runyte"))
                    .unwrap(),
                None,
                "{marker}"
            );

            fs::remove_dir_all(temporary).unwrap();
        }
    }

    #[test]
    fn adoption_refusals_report_the_complete_cause_and_recovery() {
        let root = tempfile();
        let directory = root.join("plain");
        let elsewhere = root.join("elsewhere");
        fs::create_dir(&directory).unwrap();
        fs::create_dir(&elsewhere).unwrap();
        let requirement = super::WorkspaceRequirement::new(super::WorkspaceMode::Ide, true);
        let check = |launch: &Path, state: &Path, targets: &[std::path::PathBuf], reason: &str| {
            let error = adopt(launch, state, &[], &[], targets, requirement).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "runyte needs a workspace in ide mode (the --ide flag), and none was found in {} or any directory above it.\n{reason}\n  Open a project:      {}\n  Create one:          runyte --init DIRECTORY\n  Edit without one:    runyte --editor",
                    super::display_workspace_path(&launch.canonicalize().unwrap(), None),
                    super::ide_open_command("DIRECTORY")
                )
            );
        };
        check(
            &directory,
            &elsewhere.join("state"),
            &[],
            "none is made automatically while workspace.state is an absolute path",
        );
        check(
            &directory,
            Path::new(".runyte"),
            std::slice::from_ref(&elsewhere),
            &format!(
                "{} is outside {}, so that directory is not made one automatically",
                super::display_workspace_path(&elsewhere, None),
                super::display_workspace_path(&directory, None)
            ),
        );
        check(
            &std::env::temp_dir(),
            Path::new(".runyte"),
            &[],
            "the temporary directory is not made one automatically",
        );
        let filesystem_root = directory.ancestors().last().unwrap();
        check(
            filesystem_root,
            Path::new(".runyte"),
            &[],
            "the filesystem root is not made one automatically",
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o777)).unwrap();
            check(
                &directory,
                Path::new(".runyte"),
                &[],
                "a directory other accounts can write to is not made one automatically",
            );
        }
        assert!(!directory.join(".runyte").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn adoption_makes_the_directory_a_workspace_later_launches_find() {
        let directory = tempfile();
        let nested = directory.join("lib");
        fs::create_dir(&nested).unwrap();
        let home = tempfile();

        let adopted = adopt(
            &directory,
            std::path::Path::new(".runyte"),
            &[],
            std::slice::from_ref(&home),
            &[
                directory.join("lib/new.rs"),
                directory.join("lib"),
                directory.join("notes/not-yet/today.md"),
                directory.join("./lib/../lib/mod.rs"),
            ],
            super::WorkspaceRequirement::new(super::WorkspaceMode::Ide, false),
        )
        .unwrap();

        assert_eq!(adopted, directory.canonicalize().unwrap());
        assert!(directory.join(".runyte").is_dir());
        assert_eq!(
            discover(&nested, std::path::Path::new(".runyte"))
                .unwrap()
                .map(|workspace| workspace.root),
            Some(adopted)
        );

        fs::remove_dir_all(directory).unwrap();
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn adoption_refuses_the_home_directory_and_the_filesystem_root() {
        let home = tempfile();
        let other = tempfile();

        let refused = adopt(
            &home,
            std::path::Path::new(".runyte"),
            &[],
            &[other.clone(), home.clone()],
            &[],
            super::WorkspaceRequirement::new(super::WorkspaceMode::Ide, false),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("the home directory"), "{refused}");
        assert!(refused.contains("runyte --init DIRECTORY"), "{refused}");
        assert!(!home.join(".runyte").exists());

        let root = std::path::Path::new(if cfg!(windows) { "C:\\" } else { "/" });
        let refused = adopt(
            root,
            std::path::Path::new(".runyte"),
            &[],
            &[],
            &[],
            super::WorkspaceRequirement::new(super::WorkspaceMode::Ide, false),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("the filesystem root"), "{refused}");

        fs::remove_dir_all(home).unwrap();
        fs::remove_dir_all(other).unwrap();
    }

    #[test]
    fn adoption_refuses_the_temporary_directory() {
        let refused = adopt(
            &std::env::temp_dir(),
            std::path::Path::new(".runyte-never-created"),
            &[],
            &[],
            &[],
            super::WorkspaceRequirement::new(super::WorkspaceMode::Ide, false),
        )
        .unwrap_err()
        .to_string();

        assert!(refused.contains("the temporary directory"), "{refused}");
        assert!(!std::env::temp_dir().join(".runyte-never-created").exists());
    }

    #[cfg(unix)]
    #[test]
    fn adoption_refuses_a_directory_other_accounts_can_write_to() {
        use std::os::unix::fs::PermissionsExt;

        for mode in [0o1777, 0o777] {
            let directory = tempfile();
            fs::set_permissions(&directory, fs::Permissions::from_mode(mode)).unwrap();

            let refused = adopt(
                &directory,
                std::path::Path::new(".runyte"),
                &[],
                &[],
                &[],
                super::WorkspaceRequirement::new(super::WorkspaceMode::Ide, false),
            )
            .unwrap_err()
            .to_string();

            assert!(
                refused.contains("other accounts can write to"),
                "{mode:o}: {refused}"
            );
            assert!(!directory.join(".runyte").exists());
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn adoption_refuses_a_launch_naming_a_file_outside_the_directory() {
        let directory = tempfile();
        let elsewhere = tempfile();
        fs::write(elsewhere.join("hosts"), "").unwrap();

        for target in [elsewhere.join("hosts"), directory.join("../escape.txt")] {
            let refused = adopt(
                &directory,
                std::path::Path::new(".runyte"),
                &[],
                &[],
                std::slice::from_ref(&target),
                super::WorkspaceRequirement::new(super::WorkspaceMode::Ide, false),
            )
            .unwrap_err()
            .to_string();

            assert!(refused.contains("is outside"), "{refused}");
            assert!(!directory.join(".runyte").exists());
        }

        fs::remove_dir_all(directory).unwrap();
        fs::remove_dir_all(elsewhere).unwrap();
    }

    #[test]
    fn adoption_refuses_an_absolute_state_directory() {
        let directory = tempfile();
        let state = tempfile().join("shared-state");

        let refused = adopt(
            &directory,
            &state,
            &[],
            &[],
            &[],
            super::WorkspaceRequirement::new(super::WorkspaceMode::Ide, false),
        )
        .unwrap_err()
        .to_string();

        assert!(
            refused.contains("workspace.state is an absolute path"),
            "{refused}"
        );
        assert!(!state.exists());

        fs::remove_dir_all(directory).unwrap();
        fs::remove_dir_all(state.parent().unwrap()).unwrap();
    }

    #[test]
    fn adoption_refuses_a_directory_whose_state_would_enter_per_user_storage() {
        let reserved = tempfile();

        let refused = adopt(
            &reserved,
            std::path::Path::new(".runyte"),
            std::slice::from_ref(&reserved),
            &[],
            &[],
            super::WorkspaceRequirement::new(super::WorkspaceMode::Ide, false),
        )
        .unwrap_err();

        assert!(
            format!("{refused:#}").contains("overlaps per-user Runyte storage"),
            "{refused:#}"
        );
        assert!(refused.to_string().contains("runyte --editor"), "{refused}");
        assert!(!reserved.join(".runyte").exists());

        fs::remove_dir_all(reserved).unwrap();
    }

    #[test]
    fn initialization_creates_and_selects_the_exact_requested_workspace() {
        let outer = tempfile();
        fs::create_dir(outer.join(".runyte")).unwrap();
        let project = outer.join("project");
        fs::create_dir(&project).unwrap();

        let selected = initialize(&project, std::path::Path::new(".runyte"), &[]).unwrap();

        assert_eq!(selected, project);
        assert!(project.join(".runyte").is_dir());
        assert_eq!(
            discover(&project, std::path::Path::new(".runyte"))
                .unwrap()
                .map(|workspace| workspace.root),
            Some(project.clone())
        );

        // Repeating initialization is the supported way to use an existing
        // workspace, not an error or a destructive reset.
        fs::write(project.join(".runyte/kept"), "workspace state").unwrap();
        assert_eq!(
            initialize(&project, std::path::Path::new(".runyte"), &[]).unwrap(),
            project
        );
        assert_eq!(
            fs::read_to_string(project.join(".runyte/kept")).unwrap(),
            "workspace state"
        );

        fs::remove_dir_all(outer).unwrap();
    }

    #[test]
    fn initialization_honors_a_configured_relative_state_directory() {
        let project = tempfile();

        let selected = initialize(&project, std::path::Path::new(".local-workspace"), &[]).unwrap();

        assert_eq!(selected, project);
        assert!(project.join(".local-workspace").is_dir());

        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn a_home_directory_can_own_a_project_workspace() {
        let home = tempfile();
        fs::create_dir(home.join(".runyte")).unwrap();
        let nested = home.join("notes");
        fs::create_dir(&nested).unwrap();

        assert_eq!(
            discover_candidates(
                &[nested.as_path(), home.as_path()],
                std::path::Path::new(".runyte")
            )
            .unwrap()
            .map(|workspace| workspace.root),
            Some(home.clone())
        );

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn configured_relative_workspace_is_the_non_git_marker() {
        let outer = tempfile();
        fs::create_dir(outer.join(".runyte")).unwrap();
        let project = outer.join("project");
        fs::create_dir_all(project.join(".local-workspace")).unwrap();
        let nested = project.join("one/two");
        fs::create_dir_all(&nested).unwrap();

        assert_eq!(
            discover(&nested, std::path::Path::new(".local-workspace"))
                .unwrap()
                .map(|workspace| workspace.root),
            Some(project)
        );

        fs::remove_dir_all(outer).unwrap();
    }

    #[test]
    fn an_empty_dot_git_entry_is_not_a_repository_root() {
        let temporary = tempfile();
        fs::create_dir(temporary.join(".git")).unwrap();

        assert_eq!(
            discover_candidates(&[temporary.as_path()], std::path::Path::new(".runyte")).unwrap(),
            None
        );

        fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn state_root_cannot_contain_or_enter_per_user_storage() {
        let home = tempfile();
        let config = home.join(".config/runyte");
        let cache = home.join(".cache/runyte");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&cache).unwrap();

        assert!(validate_state_root(&config, std::slice::from_ref(&config)).is_err());
        assert!(
            validate_state_root(&config.join("workspace"), std::slice::from_ref(&config)).is_err()
        );
        assert!(validate_state_root(&home, &[cache]).is_err());
        assert!(validate_state_root(&home.join(".runyte"), &[config]).is_ok());

        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn state_root_overlap_resolves_aliases_above_missing_descendants() {
        let root = tempfile();
        let reserved = root.join("reserved");
        let alias = root.join("alias");
        fs::create_dir(&reserved).unwrap();
        std::os::unix::fs::symlink(&reserved, &alias).unwrap();
        let state = alias.join("not-created/yet");
        assert!(initialize(&root, &state, std::slice::from_ref(&reserved)).is_err());
        assert!(!reserved.join("not-created").exists());
        assert!(validate_state_root(&reserved, &[state]).is_err());

        let separate = root.join("separate/not-created/yet");
        initialize(&root, &separate, std::slice::from_ref(&reserved)).unwrap();
        assert!(separate.is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn state_root_overlap_predicts_parent_components_after_directory_creation() {
        let root = tempfile();
        let reserved = root.join("reserved");
        fs::create_dir_all(reserved.join("inside")).unwrap();
        std::os::unix::fs::symlink(&reserved, root.join("alias")).unwrap();
        std::os::unix::fs::symlink(reserved.join("inside"), root.join("nested-alias")).unwrap();
        for configured in [
            "missing/../reserved/runtime",
            "missing/../alias/runtime",
            "nested-alias/missing/../../runtime",
        ] {
            let state = root.join(configured);
            assert!(
                initialize(&root, &state, std::slice::from_ref(&reserved)).is_err(),
                "future state root enters reserved storage: {configured}"
            );
            assert!(!root.join("missing").exists());
            assert!(!reserved.join("inside/missing").exists());
            assert!(!reserved.join("runtime").exists());
            assert!(validate_state_root(&reserved, &[state]).is_err());
        }

        let separate = root.join("created/../separate/not-created/yet");
        initialize(&root, &separate, std::slice::from_ref(&reserved)).unwrap();
        assert!(root.join("created").is_dir());
        assert!(root.join("separate/not-created/yet").is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn native_state_root_overlap_uses_case_prefix_and_missing_descendant_rules() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let root = tempfile();
        let missing_state = root.join("Runyte");
        let missing_reserved = root.join("runyte/cache");
        assert!(validate_state_root(&missing_state, &[missing_reserved]).is_err());

        let state_case = root.join("PROJECT/STATE");
        let reserved_case = root.join("project/state/config");
        assert!(validate_state_root(&state_case, &[reserved_case]).is_err());

        let units: Vec<_> = root.as_os_str().encode_wide().collect();
        let ordinary =
            if units.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]) {
                PathBuf::from(std::ffi::OsString::from_wide(&units[4..]))
            } else {
                root.clone()
            };
        let ordinary_state = ordinary.join("verbatim-overlap");
        let verbatim_reserved = root.join("verbatim-overlap/cache");
        assert!(validate_state_root(&ordinary_state, &[verbatim_reserved]).is_err());

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn native_state_root_overlap_follows_a_reserved_junction_alias() {
        use std::{
            fs::OpenOptions,
            os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
            ptr,
        };
        use windows_sys::Win32::{
            Foundation::{GENERIC_READ, GENERIC_WRITE},
            Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
                FILE_SHARE_READ, FILE_SHARE_WRITE,
            },
        };

        let root = tempfile();
        let state = root.join("ordinary-state");
        let alias = root.join("reserved-alias");
        fs::create_dir(&state).unwrap();
        fs::create_dir(&alias).unwrap();
        let junction = OpenOptions::new()
            .access_mode(GENERIC_READ | GENERIC_WRITE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&alias)
            .unwrap();
        let target = crate::windows_fs::ordinary_working_directory(&state).unwrap();
        let substitute: Vec<_> = std::ffi::OsString::from(format!("\\??\\{}", target.display()))
            .encode_wide()
            .collect();
        let mut data = Vec::new();
        data.extend_from_slice(&0xA0000003u32.to_le_bytes());
        data.extend_from_slice(&((8 + (substitute.len() + 2) * 2) as u16).to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&((substitute.len() * 2) as u16).to_le_bytes());
        data.extend_from_slice(&(((substitute.len() + 1) * 2) as u16).to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        for unit in substitute {
            data.extend_from_slice(&unit.to_le_bytes());
        }
        data.extend_from_slice(&[0; 4]);
        let mut returned = 0;
        assert_ne!(
            unsafe {
                windows_sys::Win32::System::IO::DeviceIoControl(
                    junction.as_raw_handle(),
                    0x000900A4,
                    data.as_ptr().cast(),
                    data.len() as u32,
                    ptr::null_mut(),
                    0,
                    &mut returned,
                    ptr::null_mut(),
                )
            },
            0,
            "{}",
            io::Error::last_os_error()
        );
        drop(junction);

        assert!(validate_state_root(&state, std::slice::from_ref(&alias)).is_err());
        fs::remove_dir(&alias).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    fn tempfile() -> std::path::PathBuf {
        // The clock alone is not enough to separate these: tests run on
        // several threads and two of them can read the same nanosecond, which
        // makes the second one fail to create its directory.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "runyte-project-root-{}-{}-{sequence}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        // Explicit, so a permissive umask cannot make it look shared.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path.canonicalize().unwrap()
    }
}

#[cfg(test)]
#[path = "project_root/tests/startup.rs"]
mod startup_tests;
