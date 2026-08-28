// SPDX-License-Identifier: MPL-2.0

//! Owned command-line launch targets.
//!
//! This module deliberately knows nothing about Crossterm or editor panes. It
//! turns operating-system arguments into file identities and one-based text
//! positions before the terminal is entered.

use std::{ffi::OsString, num::NonZeroUsize, path::PathBuf};

use anyhow::{Context, Result, bail, ensure};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LaunchMode {
    #[default]
    Standalone,
    Serve,
    Persistent,
    Wait,
    ListSessions,
    StartSession,
    StopAllSessions,
    ClearAllSessions,
    RenameSession,
    RestartSession,
    StopSession,
}

/// A one-based source position requested on the command line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaunchPosition {
    pub line: NonZeroUsize,
    pub column: Option<NonZeroUsize>,
}

/// A file requested at process launch, optionally with an initial caret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchTarget {
    pub path: PathBuf,
    pub position: Option<LaunchPosition>,
}

impl LaunchTarget {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            position: None,
        }
    }

    pub fn at(path: impl Into<PathBuf>, position: LaunchPosition) -> Self {
        Self {
            path: path.into(),
            position: Some(position),
        }
    }
}

/// All process-launch arguments after syntactic validation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LaunchArguments {
    pub targets: Vec<LaunchTarget>,
    pub config: Option<PathBuf>,
    /// A directory the user explicitly wants to make and use as a workspace.
    pub init: Option<PathBuf>,
    pub cwd_file: Option<PathBuf>,
    pub workspace_selector: Option<PathBuf>,
    pub workspace_name: Option<String>,
    /// The workspace this process serves, when the caller has already resolved
    /// it. Discovery and its non-Git prompt are skipped in favour of this.
    pub project_root: Option<PathBuf>,
    pub help: bool,
    pub version: bool,
    pub mode: LaunchMode,
    /// Whether the command line, rather than configuration, selected a mode.
    pub mode_explicit: bool,
    /// Explicit permission for a lifecycle command to discard protected host
    /// state, including live terminal children.
    pub force: bool,
    /// How many times `-v` was given. Zero leaves the default warning level;
    /// each repetition raises it, and [`crate::log::Level::from_verbosity`]
    /// caps the result at trace.
    pub verbosity: u8,
    /// An explicit diagnostic log destination. Failing to honour it is a
    /// startup error: silently choosing another file would make the requested
    /// capture misleading.
    pub log: Option<PathBuf>,
}

impl LaunchArguments {
    /// Whether this invocation asked for logging different from the default.
    ///
    /// An attachment cannot reconfigure a running host's logger, so the
    /// attaching command reports the retention rather than appearing to have
    /// applied these.
    pub const fn requests_logging(&self) -> bool {
        self.verbosity > 0 || self.log.is_some()
    }

    pub fn parse() -> Result<Self> {
        Self::parse_from(std::env::args_os().skip(1))
    }

    /// Parses `runyte [OPTIONS] [+LINE[:COLUMN] FILE]... [-- FILE...]`.
    ///
    /// A target marker belongs only to the immediately following file. After
    /// `--`, every argument is a literal path, including names beginning with
    /// `-` or `+`.
    pub fn parse_from(arguments: impl IntoIterator<Item = OsString>) -> Result<Self> {
        let mut parsed = Self::default();
        let mut arguments = arguments.into_iter();
        let mut pending_position = None;
        let mut literal_paths = false;
        let mut mode_explicit = false;

        while let Some(argument) = arguments.next() {
            if literal_paths {
                parsed.targets.push(LaunchTarget {
                    path: PathBuf::from(argument),
                    position: None,
                });
                continue;
            }

            if argument == "--" {
                ensure!(
                    pending_position.is_none(),
                    "a +LINE[:COLUMN] target must be followed immediately by a file before --"
                );
                literal_paths = true;
                continue;
            }

            let display = argument.to_string_lossy();
            let is_option = display.starts_with('-');
            if is_option {
                ensure!(
                    pending_position.is_none(),
                    "a +LINE[:COLUMN] target must be followed immediately by a file, not an option"
                );
            }

            if let Some(path) = argument
                .to_str()
                .and_then(|value| value.strip_prefix("--cwd-file="))
            {
                ensure!(!path.is_empty(), "--cwd-file requires a path");
                parsed.cwd_file = Some(PathBuf::from(path));
                continue;
            }
            match display.as_ref() {
                "-h" | "--help" => parsed.help = true,
                "-V" | "--version" => parsed.version = true,
                "--standalone" => {
                    set_mode(&mut parsed.mode, &mut mode_explicit, LaunchMode::Standalone)?
                }
                "--serve" => set_mode(&mut parsed.mode, &mut mode_explicit, LaunchMode::Serve)?,
                // A bare `-a` attaches to the workspace found from the
                // current directory. A trailing selector names one outright,
                // in the same grammar `--session-start` and `:session-attach`
                // already accept, so attaching from anywhere is one launch
                // rather than a launch followed by an editor switch.
                "-a" | "--persistent" => {
                    set_mode(&mut parsed.mode, &mut mode_explicit, LaunchMode::Persistent)?
                }
                "--wait" => set_mode(&mut parsed.mode, &mut mode_explicit, LaunchMode::Wait)?,
                "-l" | "--session-list" => set_mode(
                    &mut parsed.mode,
                    &mut mode_explicit,
                    LaunchMode::ListSessions,
                )?,
                "--session-start" => set_mode(
                    &mut parsed.mode,
                    &mut mode_explicit,
                    LaunchMode::StartSession,
                )?,
                "--session-stop-all" => set_mode(
                    &mut parsed.mode,
                    &mut mode_explicit,
                    LaunchMode::StopAllSessions,
                )?,
                "--session-clear-all" => set_mode(
                    &mut parsed.mode,
                    &mut mode_explicit,
                    LaunchMode::ClearAllSessions,
                )?,
                "--session-rename" => {
                    set_mode(
                        &mut parsed.mode,
                        &mut mode_explicit,
                        LaunchMode::RenameSession,
                    )?;
                    parsed.workspace_selector = Some(PathBuf::from(arguments.next().context(
                        "--session-rename requires a workspace ID, name, or directory",
                    )?));
                    parsed.workspace_name = Some(utf8_option_value(
                        arguments.next(),
                        "--session-rename requires a UTF-8 new name",
                    )?);
                }
                "--session-restart" => set_mode(
                    &mut parsed.mode,
                    &mut mode_explicit,
                    LaunchMode::RestartSession,
                )?,
                "-s" | "--session-stop" => set_mode(
                    &mut parsed.mode,
                    &mut mode_explicit,
                    LaunchMode::StopSession,
                )?,
                "-f" | "--force" => parsed.force = true,
                // `-vv` and `-vvv` are how repetition is normally written, so
                // the one clustered short option Runyte accepts is this one.
                "--verbose" => parsed.verbosity = parsed.verbosity.saturating_add(1),
                value
                    if value.len() > 1
                        && value.starts_with('-')
                        && value[1..].bytes().all(|byte| byte == b'v') =>
                {
                    parsed.verbosity = parsed
                        .verbosity
                        .saturating_add(u8::try_from(value.len() - 1).unwrap_or(u8::MAX));
                }
                "--log" => {
                    let path = arguments
                        .next()
                        .map(PathBuf::from)
                        .context("--log requires a path")?;
                    ensure!(!path.as_os_str().is_empty(), "--log requires a path");
                    parsed.log = Some(path);
                }
                "-c" | "--config" => {
                    parsed.config = Some(
                        arguments
                            .next()
                            .map(PathBuf::from)
                            .context("--config requires a path")?,
                    );
                }
                "-i" | "--init" => {
                    let directory = arguments
                        .next()
                        .map(PathBuf::from)
                        .context("--init requires a directory")?;
                    ensure!(
                        !directory.as_os_str().is_empty(),
                        "--init requires a directory"
                    );
                    parsed.init = Some(directory);
                }
                "--cwd-file" => {
                    parsed.cwd_file = Some(
                        arguments
                            .next()
                            .map(PathBuf::from)
                            .context("--cwd-file requires a path")?,
                    );
                }
                "--project-root" => {
                    let root = arguments
                        .next()
                        .map(PathBuf::from)
                        .context("--project-root requires a path")?;
                    ensure!(
                        !root.as_os_str().is_empty(),
                        "--project-root requires a path"
                    );
                    parsed.project_root = Some(root);
                }
                value if value.starts_with("--cwd-file=") => {
                    bail!("--cwd-file=PATH requires a UTF-8 path; use --cwd-file PATH instead")
                }
                "-" => bail!(
                    "stdin input is not supported yet because Crossterm owns stdin; use a file path, or use -- - to open a file literally named '-'"
                ),
                value if value.starts_with('-') => bail!("unknown option: {value}"),
                value if value.starts_with('+') => {
                    ensure!(
                        pending_position.is_none(),
                        "consecutive +LINE[:COLUMN] targets are not allowed; each target must be followed immediately by a file"
                    );
                    pending_position = Some(parse_position(value)?);
                }
                _ => parsed.targets.push(LaunchTarget {
                    path: PathBuf::from(argument),
                    position: pending_position.take(),
                }),
            }
        }

        ensure!(
            pending_position.is_none(),
            "a trailing +LINE[:COLUMN] target requires a file"
        );
        ensure!(
            !parsed.force
                || matches!(
                    parsed.mode,
                    LaunchMode::RestartSession
                        | LaunchMode::StopSession
                        | LaunchMode::StopAllSessions
                ),
            "--force is available only with --session-stop, --session-stop-all, or --session-restart"
        );
        ensure!(
            parsed.init.is_none() || parsed.project_root.is_none(),
            "--init cannot be combined with --project-root"
        );
        ensure!(
            parsed.init.is_none() || parsed.targets.is_empty(),
            "--init does not accept file targets"
        );
        ensure!(
            parsed.init.is_none()
                || matches!(
                    parsed.mode,
                    LaunchMode::Standalone | LaunchMode::Serve | LaunchMode::Persistent
                ),
            "--init is not available in this session-management mode"
        );
        ensure!(
            parsed.mode != LaunchMode::Wait || !parsed.targets.is_empty(),
            "--wait requires at least one file"
        );
        ensure!(
            parsed.mode != LaunchMode::Wait
                || parsed
                    .targets
                    .iter()
                    .all(|target| target.position.is_none()),
            "--wait does not accept +LINE[:COLUMN] positions"
        );
        if matches!(
            parsed.mode,
            LaunchMode::Persistent
                | LaunchMode::StartSession
                | LaunchMode::RestartSession
                | LaunchMode::StopSession
        ) {
            ensure!(
                parsed.targets.len() <= 1,
                "this workspace mode accepts at most one workspace ID, name, or directory"
            );
            if let Some(target) = parsed.targets.pop() {
                ensure!(
                    target.position.is_none(),
                    "workspace selectors do not accept a +LINE[:COLUMN] position"
                );
                parsed.workspace_selector = Some(target.path);
            }
        }
        ensure!(
            !matches!(
                parsed.mode,
                LaunchMode::ListSessions
                    | LaunchMode::StopAllSessions
                    | LaunchMode::ClearAllSessions
                    | LaunchMode::RenameSession
            ) || parsed.targets.is_empty(),
            "this session-management mode does not accept file targets"
        );
        ensure!(
            parsed.workspace_selector.is_none() || parsed.init.is_none(),
            "a workspace selector cannot be combined with --init"
        );
        ensure!(
            parsed.workspace_selector.is_none() || parsed.project_root.is_none(),
            "a workspace selector cannot be combined with --project-root"
        );
        parsed.mode_explicit = mode_explicit;
        Ok(parsed)
    }
}

fn utf8_option_value(value: Option<OsString>, missing: &str) -> Result<String> {
    value
        .ok_or_else(|| anyhow::anyhow!(missing.to_owned()))?
        .into_string()
        .map_err(|_| anyhow::anyhow!(missing.to_owned()))
}

fn set_mode(current: &mut LaunchMode, explicit: &mut bool, requested: LaunchMode) -> Result<()> {
    ensure!(!*explicit, "workspace modes are mutually exclusive");
    *current = requested;
    *explicit = true;
    Ok(())
}

fn parse_position(value: &str) -> Result<LaunchPosition> {
    let target = value
        .strip_prefix('+')
        .expect("caller only parses plus-prefixed arguments");
    let mut parts = target.split(':');
    let line = parse_nonzero(parts.next().unwrap_or_default(), "line", value)?;
    let column = parts
        .next()
        .map(|column| parse_nonzero(column, "column", value))
        .transpose()?;
    ensure!(
        parts.next().is_none(),
        "malformed target {value:?}; expected +LINE or +LINE:COLUMN"
    );
    Ok(LaunchPosition { line, column })
}

fn parse_nonzero(value: &str, name: &str, target: &str) -> Result<NonZeroUsize> {
    let number = value.parse::<usize>().with_context(|| {
        format!("malformed target {target:?}; {name} must be a positive integer")
    })?;
    NonZeroUsize::new(number)
        .with_context(|| format!("malformed target {target:?}; {name} must be greater than zero"))
}

#[cfg(test)]
mod tests {
    use super::{LaunchArguments, LaunchMode, LaunchPosition, LaunchTarget};
    use std::{num::NonZeroUsize, path::PathBuf};

    fn position(line: usize, column: Option<usize>) -> LaunchPosition {
        LaunchPosition {
            line: NonZeroUsize::new(line).unwrap(),
            column: column.map(|column| NonZeroUsize::new(column).unwrap()),
        }
    }

    #[test]
    fn parses_multiple_files_positions_and_spaces() {
        let arguments = LaunchArguments::parse_from([
            "+2:3".into(),
            "a file.rs".into(),
            "plain.md".into(),
            "+8".into(),
            "last.py".into(),
        ])
        .unwrap();

        assert_eq!(
            arguments.targets,
            vec![
                LaunchTarget::at("a file.rs", position(2, Some(3))),
                LaunchTarget::new("plain.md"),
                LaunchTarget::at("last.py", position(8, None)),
            ]
        );
    }

    #[test]
    fn double_dash_makes_plus_dash_and_stdin_names_literal() {
        let arguments = LaunchArguments::parse_from([
            "ordinary".into(),
            "--".into(),
            "+12".into(),
            "-option".into(),
            "-".into(),
        ])
        .unwrap();

        assert_eq!(
            arguments
                .targets
                .into_iter()
                .map(|target| target.path)
                .collect::<Vec<_>>(),
            ["ordinary", "+12", "-option", "-"]
                .map(PathBuf::from)
                .to_vec()
        );
    }

    #[test]
    fn rejects_malformed_consecutive_and_trailing_targets() {
        for arguments in [
            vec!["+0"],
            vec!["+1:0"],
            vec!["+x"],
            vec!["+1:2:3"],
            vec!["+1", "+2", "file"],
            vec!["+1"],
            vec!["+1", "--", "file"],
        ] {
            assert!(
                LaunchArguments::parse_from(arguments.into_iter().map(OsString::from)).is_err()
            );
        }
    }

    #[test]
    fn lone_dash_explains_the_stdin_boundary() {
        let error = LaunchArguments::parse_from(["-".into()]).unwrap_err();
        assert!(error.to_string().contains("Crossterm owns stdin"));
        assert!(error.to_string().contains("-- -"));
    }

    #[test]
    fn workspace_modes_are_explicit_and_mutually_exclusive() {
        assert_eq!(
            LaunchArguments::parse_from(["--serve".into()])
                .unwrap()
                .mode,
            LaunchMode::Serve
        );
        for spelling in ["--persistent", "-a"] {
            assert_eq!(
                LaunchArguments::parse_from([spelling.into()]).unwrap().mode,
                LaunchMode::Persistent
            );
        }
        assert_eq!(
            LaunchArguments::parse_from(["--wait".into(), "note.txt".into()])
                .unwrap()
                .mode,
            LaunchMode::Wait
        );
        assert!(LaunchArguments::parse_from(["--wait".into()]).is_err());
        assert!(LaunchArguments::parse_from(["--serve".into(), "--persistent".into()]).is_err());
        // Attachment is spelled as the mode it selects. The colon command's
        // name is not a second spelling of it on the command line.
        assert!(LaunchArguments::parse_from(["--session-attach".into()]).is_err());
    }

    #[test]
    fn persistent_mode_takes_a_workspace_selector() {
        for spelling in ["--persistent", "-a"] {
            let selected = LaunchArguments::parse_from([spelling.into(), "a1b2c3".into()]).unwrap();
            assert_eq!(selected.mode, LaunchMode::Persistent);
            assert_eq!(selected.workspace_selector, Some(PathBuf::from("a1b2c3")));
            assert!(selected.targets.is_empty());

            let bare = LaunchArguments::parse_from([spelling.into()]).unwrap();
            assert_eq!(bare.mode, LaunchMode::Persistent);
            assert_eq!(bare.workspace_selector, None);
        }

        let by_directory = LaunchArguments::parse_from(["-a".into(), "/work/api".into()]).unwrap();
        assert_eq!(
            by_directory.workspace_selector,
            Some(PathBuf::from("/work/api"))
        );

        // A selector is a workspace, not a file, so it carries no caret and
        // never appears beside a second one.
        assert!(LaunchArguments::parse_from(["-a".into(), "one".into(), "two".into()]).is_err());
        assert!(LaunchArguments::parse_from(["-a".into(), "+3".into(), "api".into()]).is_err());
    }

    #[test]
    fn a_workspace_selector_rejects_the_options_that_resolve_a_project() {
        // Both options answer the question the selector already answered, and
        // --project-root additionally requires the launch directory to sit
        // inside the root it names, which an attachment from elsewhere never
        // does.
        assert!(
            LaunchArguments::parse_from([
                "-a".into(),
                "api".into(),
                "--project-root".into(),
                "/work/api".into(),
            ])
            .is_err()
        );
        assert!(
            LaunchArguments::parse_from([
                "-a".into(),
                "api".into(),
                "--init".into(),
                "/work/new".into(),
            ])
            .is_err()
        );
        assert!(
            LaunchArguments::parse_from([
                "--session-stop".into(),
                "api".into(),
                "--project-root".into(),
                "/work/api".into(),
            ])
            .is_err()
        );
    }

    #[test]
    fn init_accepts_its_public_spellings_and_rejects_ambiguous_launches() {
        for spelling in ["--init", "-i"] {
            let parsed =
                LaunchArguments::parse_from([spelling.into(), "/work/new".into()]).unwrap();
            assert_eq!(parsed.init, Some(PathBuf::from("/work/new")));
            assert_eq!(parsed.mode, LaunchMode::Standalone);
        }

        assert!(LaunchArguments::parse_from(["--init".into()]).is_err());
        assert!(LaunchArguments::parse_from(["--init".into(), "".into()]).is_err());
        assert!(
            LaunchArguments::parse_from(["--init".into(), "/work/new".into(), "note.txt".into(),])
                .is_err()
        );
        assert!(
            LaunchArguments::parse_from([
                "--session-list".into(),
                "--init".into(),
                "/work/new".into(),
            ])
            .is_err()
        );
        assert!(
            LaunchArguments::parse_from([
                "--init".into(),
                "/work/new".into(),
                "--project-root".into(),
                "/work/new".into(),
            ])
            .is_err()
        );
    }

    #[test]
    fn session_modes_accept_their_canonical_and_short_spellings() {
        for spelling in ["--session-list", "-l"] {
            assert_eq!(
                LaunchArguments::parse_from([spelling.into()]).unwrap().mode,
                LaunchMode::ListSessions
            );
        }

        assert_eq!(
            LaunchArguments::parse_from(["--session-start".into()])
                .unwrap()
                .mode,
            LaunchMode::StartSession
        );
        assert_eq!(
            LaunchArguments::parse_from(["--session-stop-all".into()])
                .unwrap()
                .mode,
            LaunchMode::StopAllSessions
        );
        assert_eq!(
            LaunchArguments::parse_from(["--session-clear-all".into()])
                .unwrap()
                .mode,
            LaunchMode::ClearAllSessions
        );

        for spelling in ["--session-stop", "-s"] {
            assert_eq!(
                LaunchArguments::parse_from([spelling.into()]).unwrap().mode,
                LaunchMode::StopSession
            );
        }

        assert_eq!(
            LaunchArguments::parse_from(["--session-restart".into()])
                .unwrap()
                .mode,
            LaunchMode::RestartSession
        );

        let renamed =
            LaunchArguments::parse_from(["--session-rename".into(), "a1b2c3".into(), "API".into()])
                .unwrap();
        assert_eq!(renamed.mode, LaunchMode::RenameSession);
        assert_eq!(renamed.workspace_selector, Some(PathBuf::from("a1b2c3")));
        assert_eq!(renamed.workspace_name.as_deref(), Some("API"));

        for spelling in ["--force", "-f"] {
            let forced =
                LaunchArguments::parse_from(["--session-stop".into(), spelling.into()]).unwrap();
            assert!(forced.force);
        }
    }

    #[test]
    fn superseded_workspace_and_host_spellings_are_unknown_options() {
        for spelling in [
            "--attach",
            "--workspace-list",
            "--wls",
            "--workspace-stop",
            "--wst",
            "--workspace-stop-all",
            "--workspace-clear-all",
            "--workspace-restart",
            "--workspace-name",
            "--workspace-rename",
            "--workspace-attach",
            "--list-workspaces",
            "--shutdown-workspace",
            "--restart-workspace",
            "--name-workspace",
            "--rename-workspace",
            "--list-hosts",
            "--shutdown-host",
            "--restart-host",
            "--name-host",
            "--rename-host",
        ] {
            let error = LaunchArguments::parse_from([spelling.into()]).unwrap_err();
            assert_eq!(error.to_string(), format!("unknown option: {spelling}"));
        }
    }

    #[test]
    fn a_trailing_workspace_selector_is_not_a_file_target() {
        assert!(
            !LaunchArguments::parse_from(Vec::<OsString>::new())
                .unwrap()
                .mode_explicit
        );
        assert!(
            LaunchArguments::parse_from(["--standalone".into()])
                .unwrap()
                .mode_explicit
        );

        for mode in [
            "--session-start",
            "--session-restart",
            "--session-stop",
            "-s",
            "--persistent",
            "-a",
        ] {
            let selected = LaunchArguments::parse_from([mode.into(), "/work/api".into()]).unwrap();
            assert_eq!(
                selected.workspace_selector,
                Some(PathBuf::from("/work/api"))
            );
            assert!(selected.targets.is_empty());
        }
    }

    #[test]
    fn rejects_malformed_session_management_arguments() {
        assert!(
            LaunchArguments::parse_from(["--session-rename".into(), "workspace".into()]).is_err()
        );
        assert!(
            LaunchArguments::parse_from(["--session-stop".into(), "one".into(), "two".into(),])
                .is_err()
        );
        assert!(
            LaunchArguments::parse_from(["--persistent".into(), "one".into(), "two".into()])
                .is_err()
        );
        assert!(LaunchArguments::parse_from(["--session-list".into(), "file".into()]).is_err());
        assert!(
            LaunchArguments::parse_from(["--session-stop-all".into(), "workspace".into()]).is_err()
        );
        assert!(
            LaunchArguments::parse_from(["--session-clear-all".into(), "workspace".into()])
                .is_err()
        );
        assert!(LaunchArguments::parse_from(["--force".into()]).is_err());
        assert!(LaunchArguments::parse_from(["--persistent".into(), "--force".into()]).is_err());
    }

    #[test]
    fn logging_options_are_repeatable_and_require_a_destination() {
        assert_eq!(
            LaunchArguments::parse_from(Vec::<OsString>::new())
                .unwrap()
                .verbosity,
            0
        );
        for spelling in ["-v", "--verbose"] {
            let once = LaunchArguments::parse_from([spelling.into()]).unwrap();
            assert_eq!(once.verbosity, 1);
            assert!(once.requests_logging());
        }
        assert_eq!(
            LaunchArguments::parse_from(["-v".into(), "-v".into(), "--verbose".into()])
                .unwrap()
                .verbosity,
            3
        );
        // The clustered spelling is how repetition is normally written.
        for (spelling, expected) in [("-vv", 2), ("-vvv", 3), ("-vvvvv", 5)] {
            assert_eq!(
                LaunchArguments::parse_from([spelling.into()])
                    .unwrap()
                    .verbosity,
                expected
            );
        }
        assert_eq!(
            LaunchArguments::parse_from(["-vv".into(), "-v".into()])
                .unwrap()
                .verbosity,
            3
        );
        // Clustering is limited to this one option; nothing else groups.
        assert!(LaunchArguments::parse_from(["-vf".into()]).is_err());

        let destination =
            LaunchArguments::parse_from(["--log".into(), "/tmp/runyte.log".into()]).unwrap();
        assert_eq!(destination.log, Some(PathBuf::from("/tmp/runyte.log")));
        assert!(destination.requests_logging());
        assert!(
            !LaunchArguments::parse_from(Vec::<OsString>::new())
                .unwrap()
                .requests_logging()
        );

        assert!(LaunchArguments::parse_from(["--log".into()]).is_err());
        assert!(LaunchArguments::parse_from(["--log".into(), "".into()]).is_err());
        // Verbosity is lower case; -V remains the version flag.
        assert!(LaunchArguments::parse_from(["-V".into()]).unwrap().version);
    }

    #[test]
    fn force_is_explicit_and_limited_to_stop_or_restart() {
        for mode in ["--session-stop", "--session-stop-all", "--session-restart"] {
            let parsed = LaunchArguments::parse_from([mode.into(), "--force".into()]).unwrap();
            assert!(parsed.force);
        }
    }

    use std::ffi::OsString;
}
