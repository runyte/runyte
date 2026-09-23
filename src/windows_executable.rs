// SPDX-License-Identifier: MPL-2.0

//! Metadata-only discovery of native executables. Callers separately establish
//! permission to launch them and own their argument vectors and lifetimes.

use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
};

pub(crate) fn resolve(
    command: &Path,
    search_path: Option<&OsStr>,
    pathext: Option<&OsStr>,
) -> Option<PathBuf> {
    let extensions: Vec<_> = pathext
        .unwrap_or(OsStr::new(".COM;.EXE"))
        .to_str()?
        .split(';')
        .filter(|extension| {
            extension.eq_ignore_ascii_case(".exe") || extension.eq_ignore_ascii_case(".com")
        })
        .collect();
    let inspect = |path: PathBuf| {
        crate::windows_fs::validate_relative(Path::new(path.file_name()?)).ok()?;
        if let Some(extension) = path.extension() {
            return ((extension.eq_ignore_ascii_case("exe")
                || extension.eq_ignore_ascii_case("com"))
                && path.is_file())
            .then_some(path);
        }
        extensions.iter().find_map(|extension| {
            let mut name = path.as_os_str().to_owned();
            name.push(extension);
            let candidate = PathBuf::from(name);
            candidate.is_file().then_some(candidate)
        })
    };
    if command.is_absolute() {
        return inspect(command.to_owned());
    }
    // Configuration documents an executable name or an absolute path. Never
    // search the workspace through relative paths, empty PATH entries or '.'.
    let mut components = command.components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return None;
    }
    crate::service_health::search_directories(search_path?)
        .filter(|directory| directory.is_absolute())
        .find_map(|directory| inspect(directory.join(command)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestRuntimeRoot;

    fn fixture(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        // Metadata-only fixture: never launch a test-written file.
        std::fs::write(&path, "not an executable").unwrap();
        path
    }

    #[test]
    fn native_discovery_uses_injected_search_order_and_explicit_paths() {
        let root = TestRuntimeRoot::new("lsp-discovery").unwrap();
        let first = root.join("first café 😀");
        let second = root.join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        fixture(&first, "server.exe");
        fixture(&second, "server.com");
        let path = std::env::join_paths([Path::new("."), &first, &second]).unwrap();
        assert_eq!(
            resolve(
                Path::new("server"),
                Some(&path),
                Some(OsStr::new(".CMD;.COM;.EXE"))
            ),
            Some(first.join("server.EXE"))
        );
        let native = fixture(&first, "server.com");
        assert_eq!(resolve(&native, None, None), Some(native));
        assert_eq!(
            resolve(&first.join("server"), None, None),
            Some(first.join("server.COM"))
        );
        assert!(resolve(Path::new("missing"), Some(&path), None).is_none());
    }

    #[test]
    fn wrappers_and_workspace_relative_search_are_not_native_commands() {
        let root = TestRuntimeRoot::new("lsp-discovery-refusal").unwrap();
        for name in ["server", "server.cmd", "server.bat", "server.ps1"] {
            let path = fixture(root.path(), name);
            assert!(resolve(&path, None, None).is_none());
        }
        fixture(root.path(), "server.exe");
        std::fs::create_dir(root.join("directory.exe")).unwrap();
        for command in [
            ".\\server.exe",
            "sub\\server.exe",
            "C:server.exe",
            "\\server.exe",
            "directory.exe",
        ] {
            assert!(resolve(Path::new(command), Some(root.as_os_str()), None).is_none());
        }
        assert!(
            resolve(
                Path::new("server.exe"),
                Some(OsStr::new(";.;relative;C:relative;")),
                None
            )
            .is_none()
        );
        assert!(
            resolve(
                Path::new("server"),
                Some(root.as_os_str()),
                Some(OsStr::new(".CMD;.exe/;.exe:;.BAT"))
            )
            .is_none()
        );
        assert!(resolve(Path::new("server.exe:stream"), Some(root.as_os_str()), None).is_none());
    }
}
