// SPDX-License-Identifier: MPL-2.0

//! Project-root discovery and the explicit non-Git fallback.
//!
//! Runtime state belongs to one workspace, not to whichever nested directory
//! happened to launch the editor. Discovery therefore prefers the nearest Git
//! root. Only when there is no Git root does an existing configured state
//! directory (normally `.runyte`) identify the root.

use std::{
    fs,
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
};

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

/// Asks which workspace directory should own runtime state, requires a
/// separate affirmative confirmation, and creates the state directory before
/// returning the workspace that owns it.
///
/// The creation is the point of the confirmation rather than a side effect of
/// it. [`discover`] recognizes a non-Git project by its state directory, so a
/// root that is only returned in memory is asked for again on every later
/// launch, and a process that cannot answer the question — a detached
/// `--serve` host has no stdin — cannot resolve the project at all.
///
/// `configured_state_root` is the configured state path. Relative paths are
/// shown beneath the proposed workspace directory; absolute paths remain
/// absolute. `home_directory` is only used to warn about the reach of that one
/// answer. Like `reserved_user_roots`, it is passed in rather than read from
/// the environment here, so tests never depend on the person's real home.
/// Keeping I/O injectable makes the confirmation contract testable without
/// touching a real terminal.
pub fn prompt<R: BufRead, W: Write>(
    launch_directory: &Path,
    configured_state_root: &Path,
    reserved_user_roots: &[PathBuf],
    home_directory: Option<&Path>,
    input: &mut R,
    output: &mut W,
) -> Result<PathBuf> {
    // Every path this compares against has been through `canonicalize`, and a
    // home directory reached by a symlink would otherwise never match.
    let home_directory = home_directory.and_then(|home| home.canonicalize().ok());
    writeln!(
        output,
        "No Git repository or existing project workspace directory was found."
    )?;

    loop {
        write!(
            output,
            "Project directory [{}]: ",
            launch_directory.display()
        )?;
        output.flush()?;

        let Some(answer) = read_answer(input)? else {
            bail!(
                "project directory was not confirmed; Runyte did not create a project workspace directory"
            );
        };
        let proposed = if answer.is_empty() {
            launch_directory.to_path_buf()
        } else {
            let path = PathBuf::from(answer);
            if path.is_absolute() {
                path
            } else {
                launch_directory.join(path)
            }
        };
        let proposed = match proposed.canonicalize() {
            Ok(path) if path.is_dir() => path,
            Ok(_) => {
                writeln!(output, "That path is not a directory.")?;
                continue;
            }
            Err(error) => {
                writeln!(output, "Cannot use {}: {error}", proposed.display())?;
                continue;
            }
        };
        let runtime_root = resolve_state_root(&proposed, configured_state_root);
        if let Err(error) = validate_state_root(&runtime_root, reserved_user_roots) {
            writeln!(output, "{error}")?;
            if configured_state_root.is_absolute() {
                bail!("configured state root overlaps per-user Runyte storage");
            }
            writeln!(output, "Choose a different project directory.")?;
            continue;
        }
        // Adopting the home directory is the one answer whose reach is larger
        // than the directory it names: `discover` walks upward, so every
        // directory below it without a marker of its own resolves here too.
        // That is a supported choice, but it should be a deliberate one rather
        // than what the suggested default happens to do.
        if home_directory.as_deref() == Some(proposed.as_path()) {
            writeln!(
                output,
                "{} is your home directory. Every directory below it with no Git repository\n\
                 and no project workspace directory of its own will belong to this one\n\
                 workspace.",
                proposed.display()
            )?;
        }
        if configured_state_root.is_absolute() {
            write!(
                output,
                "Use {} as the project directory and save Runyte project data in {}? [y/N]: ",
                proposed.display(),
                runtime_root.display()
            )?;
        } else {
            write!(
                output,
                "Save Runyte project data in {}? [y/N]: ",
                runtime_root.display()
            )?;
        }
        output.flush()?;

        let Some(confirmation) = read_answer(input)? else {
            bail!(
                "project directory was not confirmed; Runyte did not create a project workspace directory"
            );
        };
        if matches!(confirmation.to_ascii_lowercase().as_str(), "y" | "yes") {
            // Creating it here, rather than leaving it to whichever subsystem
            // first needs a file in it, is what makes the answer durable: an
            // editor session may never write runtime state at all, and the
            // person would then be asked the same question next time.
            fs::create_dir_all(&runtime_root).with_context(|| {
                format!(
                    "cannot create the project workspace directory {}",
                    runtime_root.display()
                )
            })?;
            return Ok(proposed);
        }
        writeln!(
            output,
            "Location not confirmed; choose a project directory."
        )?;
    }
}

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
    let left = comparable_path(left);
    let right = comparable_path(right);
    left.starts_with(&right) || right.starts_with(&left)
}

fn comparable_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        path.parent()
            .and_then(|parent| parent.canonicalize().ok())
            .and_then(|parent| path.file_name().map(|name| parent.join(name)))
            .unwrap_or_else(|| path.to_path_buf())
    })
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

fn read_answer(input: &mut impl BufRead) -> Result<Option<String>> {
    let mut answer = String::new();
    if input
        .read_line(&mut answer)
        .context("failed to read project directory confirmation")?
        == 0
    {
        return Ok(None);
    }
    while matches!(answer.as_bytes().last(), Some(b'\n' | b'\r')) {
        answer.pop();
    }
    Ok(Some(answer))
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Cursor};

    use super::{discover, discover_candidates, initialize, prompt, validate_state_root};

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
    fn prompt_suggests_cwd_but_requires_confirmation() {
        let temporary = tempfile();
        let mut input = Cursor::new(b"\ny\n");
        let mut output = Vec::new();

        let selected = prompt(
            &temporary,
            std::path::Path::new(".runyte"),
            &[],
            None,
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(selected, temporary);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("No Git repository"));
        assert!(output.contains("Project directory ["));
        assert!(output.contains("Save Runyte project data in"));
    }

    #[test]
    fn prompt_accepts_the_home_directory_but_says_what_it_takes_in() {
        let home = tempfile();
        let mut input = Cursor::new(b"\ny\n");
        let mut output = Vec::new();

        let selected = prompt(
            &home,
            std::path::Path::new(".runyte"),
            &[],
            Some(&home),
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(selected, home);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Save Runyte project data in"));
        // The warning states the consequence, not merely which directory this
        // is: what makes home worth a warning is everything below it.
        assert!(output.contains("is your home directory"));
        assert!(output.contains("Every directory below it"));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn an_ordinary_directory_is_confirmed_without_the_home_warning() {
        let home = tempfile();
        let project = home.join("project");
        fs::create_dir(&project).unwrap();
        let mut input = Cursor::new(b"\ny\n");
        let mut output = Vec::new();

        let selected = prompt(
            &project,
            std::path::Path::new(".runyte"),
            &[],
            Some(&home),
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(selected, project);
        assert!(
            !String::from_utf8(output)
                .unwrap()
                .contains("is your home directory")
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn the_home_warning_survives_a_symlinked_home() {
        let root = tempfile();
        let home = root.join("home");
        let linked = root.join("link");
        fs::create_dir(&home).unwrap();
        std::os::unix::fs::symlink(&home, &linked).unwrap();
        let mut input = Cursor::new(b"\ny\n");
        let mut output = Vec::new();

        // The launch directory is canonical while `$HOME` names the symlink,
        // which is the shape a login shell normally leaves behind.
        let selected = prompt(
            &home,
            std::path::Path::new(".runyte"),
            &[],
            Some(&linked),
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(selected, home);
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("is your home directory")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn absolute_workspace_confirmation_names_both_locations() {
        let project = tempfile();
        let storage = project.join("separate-runtime");
        let mut input = Cursor::new(b"\ny\n");
        let mut output = Vec::new();

        let selected = prompt(&project, &storage, &[], None, &mut input, &mut output).unwrap();

        assert_eq!(selected, project);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(&format!(
            "Use {} as the project directory",
            project.display()
        )));
        assert!(output.contains(&format!(
            "save Runyte project data in {}",
            storage.display()
        )));
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn rejected_location_is_not_accepted_implicitly() {
        let temporary = tempfile();
        let other = temporary.join("other");
        fs::create_dir(&other).unwrap();
        let mut input = Cursor::new(b"\nn\nother\nyes\n");
        let mut output = Vec::new();

        let selected = prompt(
            &temporary,
            std::path::Path::new(".runyte"),
            &[],
            None,
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(selected, other);
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("Location not confirmed")
        );
        fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn end_of_input_cannot_confirm_the_suggestion() {
        let temporary = tempfile();
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();

        let error = prompt(
            &temporary,
            std::path::Path::new(".runyte"),
            &[],
            None,
            &mut input,
            &mut output,
        )
        .unwrap_err();

        assert!(error.to_string().contains("was not confirmed"));
        assert!(!temporary.join(".runyte").exists());
        fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn confirming_the_prompt_creates_the_state_directory_it_named() {
        let temporary = tempfile();
        let mut input = Cursor::new(b"\ny\n");
        let mut output = Vec::new();

        let selected = prompt(
            &temporary,
            std::path::Path::new(".runyte"),
            &[],
            None,
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(selected, temporary);
        assert!(temporary.join(".runyte").is_dir());
        // The confirmation is durable only if the marker it leaves is the one
        // discovery reads, so a later launch resolves the same workspace
        // without asking again.
        assert_eq!(
            discover(&temporary, std::path::Path::new(".runyte")).unwrap(),
            Some(temporary.clone())
        );

        fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn confirming_an_absolute_state_root_creates_it_outside_the_project() {
        let temporary = tempfile();
        let project = temporary.join("project");
        let storage = temporary.join("runtime/state");
        fs::create_dir(&project).unwrap();
        let mut input = Cursor::new(b"\ny\n");
        let mut output = Vec::new();

        let selected = prompt(&project, &storage, &[], None, &mut input, &mut output).unwrap();

        assert_eq!(selected, project);
        assert!(storage.is_dir());

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
