// SPDX-License-Identifier: MPL-2.0

//! Project-root discovery and the explicit non-Git fallback.
//!
//! Runtime state belongs to one workspace, not to whichever nested directory
//! happened to launch the editor. Discovery therefore prefers the nearest Git
//! root. Only when there is no Git root does an existing configured state
//! directory (normally `.runyte`) identify the root.

use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[cfg(windows)]
use std::path::Component;

use anyhow::{Context, Result, bail};

/// Finds the workspace directory that owns runtime state.
///
/// Git has priority over an existing state directory even when the latter is
/// nearer to `start`. This makes an accidentally-created nested state
/// directory stop attracting later launches once it is inside a Git checkout.
pub fn discover(start: &Path, configured_state_root: &Path) -> io::Result<Option<PathBuf>> {
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
    let project_root = requested
        .canonicalize()
        .with_context(|| format!("cannot resolve project directory {}", requested.display()))?;
    anyhow::ensure!(
        project_root.is_dir(),
        "project directory {} is not a directory",
        project_root.display()
    );
    let state_root = resolve_state_root(&project_root, configured_state_root);
    validate_state_root(&state_root, reserved_user_roots)?;
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
) -> io::Result<Option<PathBuf>> {
    for candidate in candidates {
        if is_git_root(candidate)? {
            return Ok(Some((*candidate).to_path_buf()));
        }
    }

    if is_usable_relative_marker(configured_state_root) {
        for candidate in candidates {
            let runtime_root = candidate.join(configured_state_root);
            if runtime_root.is_dir() {
                return Ok(Some((*candidate).to_path_buf()));
            }
        }
    }

    Ok(None)
}

/// What a launch that needs a workspace says when discovery finds none.
/// Workspaces are created by `runyte --init`, never by answering a question
/// at startup, so the refusal names that command.
pub const NO_WORKSPACE_HERE: &str = "no workspace here; run runyte --init DIRECTORY to create one";

/// Resolves the configured runtime path against its owning project directory.
pub fn resolve_state_root(project_root: &Path, configured_state_root: &Path) -> PathBuf {
    if configured_state_root.is_absolute() {
        configured_state_root.to_path_buf()
    } else {
        project_root.join(configured_state_root)
    }
}

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
    #[cfg(windows)]
    {
        use std::{os::windows::ffi::OsStrExt, path::Prefix};

        let comparable = |path: &Path| {
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
        };
        let left = comparable(left);
        let right = comparable(right);
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
        let left = comparable_path(left);
        let right = comparable_path(right);
        left.starts_with(&right) || right.starts_with(&left)
    }
}

#[cfg(not(windows))]
fn comparable_path(path: &Path) -> PathBuf {
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
    use std::fs;
    #[cfg(windows)]
    use std::{io, path::PathBuf};

    use super::{discover, discover_candidates, initialize, validate_state_root};

    #[test]
    fn git_root_wins_over_a_nearer_runtime_directory() {
        let temporary = tempfile();
        fs::create_dir(temporary.join(".git")).unwrap();
        fs::write(temporary.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let nested = temporary.join("one/two");
        fs::create_dir_all(nested.join(".runyte")).unwrap();

        assert_eq!(
            discover(&nested, std::path::Path::new(".runyte")).unwrap(),
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
            discover(&nested, std::path::Path::new(".runyte")).unwrap(),
            Some(temporary.clone())
        );

        fs::remove_dir_all(temporary).unwrap();
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
            discover(&project, std::path::Path::new(".runyte")).unwrap(),
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
            .unwrap(),
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
            discover(&nested, std::path::Path::new(".local-workspace")).unwrap(),
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
        path.canonicalize().unwrap()
    }
}
