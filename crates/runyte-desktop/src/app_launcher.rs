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

#[cfg(any(target_os = "macos", test))]
fn search_path(path: Option<std::ffi::OsString>) -> std::ffi::OsString {
    let mut path = path
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "/usr/bin:/bin:/usr/sbin:/sbin".into());
    path.push(":/opt/homebrew/bin:/usr/local/bin");
    path
}

#[cfg(target_os = "macos")]
fn main() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let executable = std::env::current_exe()?;
    let editor = executable
        .parent()
        .ok_or_else(|| std::io::Error::other("launcher has no parent directory"))?
        .join("runyte");
    Err(std::process::Command::new(editor)
        .args(arguments(
            std::env::args_os().skip(1),
            std::env::current_dir()?.into_os_string(),
        ))
        .current_dir(runyte::launch::graphical_home_directory()?)
        .env("PATH", search_path(std::env::var_os("PATH")))
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
    fn path_preserves_user_priority_and_adds_package_managers() {
        assert_eq!(
            search_path(Some("/custom/bin:/usr/bin".into())),
            "/custom/bin:/usr/bin:/opt/homebrew/bin:/usr/local/bin"
        );
        for missing in [None, Some(OsString::new())] {
            assert_eq!(
                search_path(missing),
                "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin"
            );
        }
    }
}
