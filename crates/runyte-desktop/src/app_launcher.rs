// SPDX-License-Identifier: MPL-2.0

#[cfg(any(target_os = "macos", test))]
fn arguments(
    args: impl IntoIterator<Item = std::ffi::OsString>,
    directory: std::ffi::OsString,
) -> Vec<std::ffi::OsString> {
    let mut args = args.into_iter().peekable();
    if args
        .peek()
        .is_some_and(|arg| arg.to_str().is_some_and(|arg| arg.starts_with("-psn_")))
    {
        args.next();
    }
    [
        "--window".into(),
        "--graphical-launch".into(),
        "--graphical-cwd".into(),
        directory,
    ]
    .into_iter()
    .chain(args)
    .collect()
}

/// `PATH` with the usual package-manager directories appended when absent,
/// so Poppler and other tools remain discoverable even when no login shell
/// environment could be read.
#[cfg(any(target_os = "macos", test))]
fn search_path(path: Option<&std::ffi::OsStr>) -> std::ffi::OsString {
    let mut path = path
        .filter(|path| !path.is_empty())
        .unwrap_or("/usr/bin:/bin:/usr/sbin:/sbin".as_ref())
        .to_owned();
    for directory in ["/opt/homebrew/bin", "/usr/local/bin"] {
        if !std::env::split_paths(&path).any(|entry| entry == std::path::Path::new(directory)) {
            path.push(":");
            path.push(directory);
        }
    }
    path
}

/// The environment the editor starts with: the account's login shell
/// environment, or the inherited one when it cannot be read, with `PATH`
/// extended as [`search_path`] describes.
#[cfg(any(target_os = "macos", test))]
fn editor_environment(
    mut environment: runyte::login_environment::Variables,
) -> runyte::login_environment::Variables {
    let path = environment
        .iter()
        .rev()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| search_path(Some(value)))
        .unwrap_or_else(|| search_path(None));
    environment.retain(|(name, _)| name != "PATH");
    environment.push(("PATH".into(), path));
    environment
}

#[cfg(target_os = "macos")]
fn main() -> std::io::Result<()> {
    use runyte::login_environment::{Outcome, graphical_environment};
    use std::os::unix::process::CommandExt;
    let executable = std::env::current_exe()?;
    let editor = executable
        .parent()
        .ok_or_else(|| std::io::Error::other("launcher has no parent directory"))?
        .join("runyte");
    let home = runyte::launch::graphical_home_directory()?;
    // LaunchServices starts this launcher with launchd's environment, which
    // no login startup file has touched. Everything the editor starts —
    // terminals, a persistent session host, language servers, Git — inherits
    // what is decided here, once, before any thread exists.
    let (environment, outcome) = graphical_environment(std::env::vars_os().collect(), &home);
    if let Outcome::Unavailable(error) = outcome {
        eprintln!("runyte: using the inherited environment: {error}");
    }
    Err(std::process::Command::new(editor)
        .args(arguments(
            std::env::args_os().skip(1),
            std::env::current_dir()?.into_os_string(),
        ))
        .current_dir(home)
        .env_clear()
        .envs(editor_environment(environment))
        .exec())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("runyte-app-launcher is only available on macOS");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn finder_argument_is_removed_only_at_the_start() {
        let input = ["-psn_0_123", "file with spaces", "-psn_keep"];
        assert_eq!(
            arguments(input.map(OsString::from), "/original cwd".into()),
            [
                "--window",
                "--graphical-launch",
                "--graphical-cwd",
                "/original cwd",
                "file with spaces",
                "-psn_keep"
            ]
            .map(OsString::from)
        );
        assert_eq!(
            arguments(["notes.txt".into()], "/original cwd".into()),
            [
                "--window",
                "--graphical-launch",
                "--graphical-cwd",
                "/original cwd",
                "notes.txt"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn original_directory_and_literal_arguments_keep_native_bytes() {
        let original = std::env::temp_dir().join("outside home");
        let parsed = runyte::launch::LaunchArguments::parse_from(arguments(
            ["-psn_0_1", "--", "relative.txt", "-literal", "+literal"].map(Into::into),
            original.clone().into_os_string(),
        ))
        .unwrap();
        assert_eq!(parsed.graphical_directory, Some(original));
        assert_eq!(
            parsed
                .targets
                .iter()
                .map(|target| target.path.as_os_str())
                .collect::<Vec<_>>(),
            ["relative.txt", "-literal", "+literal"].map(std::ffi::OsStr::new)
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let file = OsString::from_vec(b"raw-\xff.txt".to_vec());
            let args = arguments([file.clone()], "/original".into());
            assert_eq!(args.last(), Some(&file));
        }
    }

    #[test]
    fn path_preserves_user_priority_and_adds_missing_package_managers() {
        assert_eq!(
            search_path(Some("/custom/bin:/usr/bin".as_ref())),
            "/custom/bin:/usr/bin:/opt/homebrew/bin:/usr/local/bin"
        );
        assert_eq!(
            search_path(Some(
                "/opt/homebrew/bin:/usr/bin:/bin:/usr/local/bin".as_ref()
            )),
            "/opt/homebrew/bin:/usr/bin:/bin:/usr/local/bin"
        );
        for missing in [None, Some("".as_ref())] {
            assert_eq!(
                search_path(missing),
                "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin"
            );
        }
    }

    #[test]
    fn editor_environment_keeps_every_variable_and_one_path() {
        let environment = editor_environment(vec![
            ("HOME".into(), "/home".into()),
            ("PATH".into(), "/profile/bin:/usr/bin".into()),
            ("EMPTY".into(), OsString::new()),
        ]);
        assert_eq!(
            environment,
            [
                ("HOME", "/home"),
                ("EMPTY", ""),
                (
                    "PATH",
                    "/profile/bin:/usr/bin:/opt/homebrew/bin:/usr/local/bin"
                ),
            ]
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        );
        assert_eq!(
            editor_environment(Vec::new()),
            [(
                OsString::from("PATH"),
                OsString::from("/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin")
            )]
        );
    }
}
