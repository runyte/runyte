// SPDX-License-Identifier: MPL-2.0

//! Optional Git discovery never launches a process or consults the workspace.
//! Git uses direct argument vectors, so PATHEXT shell wrappers are ineligible.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub(super) fn discover(search_path: Option<&OsStr>, pathext: Option<&OsStr>) -> Option<PathBuf> {
    let extensions: Vec<_> = pathext
        .unwrap_or(OsStr::new(".COM;.EXE"))
        .to_str()?
        .split(';')
        .filter(|extension| {
            extension.eq_ignore_ascii_case(".exe") || extension.eq_ignore_ascii_case(".com")
        })
        .collect();
    crate::service_health::search_directories(search_path?)
        .filter(|directory| directory.is_absolute())
        .find_map(|directory| {
            extensions
                .iter()
                .map(|extension| directory.join(format!("git{extension}")))
                .find(|candidate| Path::is_file(candidate))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestRuntimeRoot;

    fn fixture(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        // Discovery only inspects metadata. This file must never be executed.
        std::fs::write(&path, "discovery fixture, never executed").unwrap();
        path
    }

    #[test]
    fn missing_git_and_unusable_search_paths_disable_discovery() {
        let root = TestRuntimeRoot::new("git-absent").unwrap();
        std::fs::create_dir(root.join("git.exe")).unwrap();
        for search in [None, Some(OsStr::new("")), Some(root.as_os_str())] {
            assert_eq!(discover(search, None), None);
        }
        assert_eq!(
            discover(Some(OsStr::new(";.;relative;C:relative;")), None),
            None
        );
    }

    #[test]
    fn native_git_lookup_obeys_path_and_pathext_order_without_running_it() {
        let root = TestRuntimeRoot::new("git-lookup").unwrap();
        let first = root.join("first café 😀");
        let second = root.join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        fixture(&first, "git.EXE");
        fixture(&second, "git.COM");
        let search =
            std::env::join_paths([Path::new(""), Path::new("."), &first, &second]).unwrap();
        assert_eq!(
            discover(Some(&search), Some(OsStr::new(".COM;.EXE"))),
            Some(first.join("git.EXE"))
        );
        fixture(&first, "git.COM");
        assert_eq!(
            discover(Some(&search), Some(OsStr::new(".COM;.EXE"))),
            Some(first.join("git.COM"))
        );
        let found = discover(Some(&search), Some(OsStr::new(".eXe;.com"))).unwrap();
        assert!(found.is_absolute());
        assert_eq!(found, first.join("git.eXe"));
    }

    #[test]
    fn shell_wrappers_extensionless_files_and_malformed_extensions_are_skipped() {
        let root = TestRuntimeRoot::new("git-wrappers").unwrap();
        for name in ["git", "git.cmd", "git.bat", "git.ps1"] {
            fixture(root.path(), name);
        }
        let search = Some(root.as_os_str());
        assert_eq!(
            discover(search, Some(OsStr::new(".CMD;.BAT;.PS1;.EXE"))),
            None
        );
        fixture(root.path(), "git.exe");
        assert_eq!(discover(search, Some(OsStr::new(""))), None);
        assert_eq!(
            discover(
                search,
                Some(OsStr::new(".CMD;.BAT;.PS1;.exe/;.exe\\;.exe:"))
            ),
            None
        );
        assert_eq!(
            discover(search, Some(OsStr::new(".CMD;.BAT;.EXE"))),
            Some(root.join("git.EXE"))
        );
        assert_eq!(discover(search, None), Some(root.join("git.EXE")));
    }
}
