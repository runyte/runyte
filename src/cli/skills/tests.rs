// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "runyte-skills-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
    fn skill(&self) -> PathBuf {
        self.0.join("project with spaces/.agents/skills/runyte-mcp")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn options(args: &[&str]) -> Result<Option<Options>> {
    parse(args.iter().map(OsString::from))
}
fn record(path: &Path, installed: Option<Release>, pending: Option<Release>) {
    fs::write(
        path.join(METADATA),
        serde_json::to_vec(&Metadata {
            schema: 1,
            installed,
            pending,
        })
        .unwrap(),
    )
    .unwrap();
}
fn old() -> Release {
    Release {
        sha256: sha256_hex(b"old skill"),
        ..Release::bundled()
    }
}

#[test]
fn explicit_scope_and_targets_are_required() {
    for args in [
        vec![],
        vec!["remove"],
        vec!["install", "--all"],
        vec!["install", "--user"],
        vec!["install", "--all", "--user", "--project"],
        vec!["install", "--all", "--user", "--force"],
    ] {
        assert!(options(&args).is_err(), "{args:?}");
    }
    assert!(options(&["--help"]).unwrap().is_none());
    assert!(options(&["update", "-h"]).unwrap().is_none());
    assert_eq!(
        options(&["status", "--claude", "--codex", "--project"]).unwrap(),
        Some(Options {
            action: Action::Status,
            claude: true,
            codex: true,
            user: false
        })
    );
}

#[test]
fn install_is_offline_idempotent_and_status_does_not_create_files() {
    let f = Fixture::new();
    let path = f.skill();
    assert_eq!(
        manage(&f.0, &path, Action::Status).unwrap(),
        "not installed"
    );
    assert!(!path.exists());
    assert!(manage(&f.0, &path, Action::Update).is_err());
    assert!(!path.exists());
    assert!(
        manage(&f.0, &path, Action::Install)
            .unwrap()
            .contains("installed")
    );
    assert_eq!(fs::read(path.join(PAYLOAD)).unwrap(), SKILL);
    let metadata = fs::read(path.join(METADATA)).unwrap();
    assert!(
        manage(&f.0, &path, Action::Install)
            .unwrap()
            .contains("current")
    );
    assert_eq!(fs::read(path.join(METADATA)).unwrap(), metadata);
    assert!(
        manage(&f.0, &path, Action::Status)
            .unwrap()
            .contains("current")
    );
    assert!(
        manage(&f.0, &path, Action::Update)
            .unwrap()
            .contains("current")
    );
}

#[test]
fn managed_updates_preserve_unrelated_files_and_reject_local_edits() {
    let f = Fixture::new();
    let path = f.skill();
    manage(&f.0, &path, Action::Install).unwrap();
    fs::write(path.join("notes.txt"), "user notes").unwrap();
    fs::write(path.join(PAYLOAD), b"old skill").unwrap();
    record(&path, Some(old()), None);
    assert!(
        manage(&f.0, &path, Action::Status)
            .unwrap()
            .contains("update available")
    );
    assert!(manage(&f.0, &path, Action::Install).is_err());
    manage(&f.0, &path, Action::Update).unwrap();
    assert_eq!(fs::read(path.join(PAYLOAD)).unwrap(), SKILL);
    assert_eq!(
        fs::read_to_string(path.join("notes.txt")).unwrap(),
        "user notes"
    );
    fs::write(path.join(PAYLOAD), b"my edits").unwrap();
    let metadata = fs::read(path.join(METADATA)).unwrap();
    for action in [Action::Install, Action::Update, Action::Status] {
        assert!(manage(&f.0, &path, action).is_err());
    }
    assert_eq!(fs::read(path.join(PAYLOAD)).unwrap(), b"my edits");
    assert_eq!(fs::read(path.join(METADATA)).unwrap(), metadata);
}

#[test]
fn unmanaged_files_are_only_adopted_when_identical() {
    let f = Fixture::new();
    let path = f.skill();
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join(PAYLOAD), b"other skill").unwrap();
    assert!(manage(&f.0, &path, Action::Install).is_err());
    assert!(!path.join(METADATA).exists());
    fs::write(path.join(PAYLOAD), SKILL).unwrap();
    assert!(
        manage(&f.0, &path, Action::Status)
            .unwrap()
            .contains("unmanaged")
    );
    assert!(!path.join(METADATA).exists());
    manage(&f.0, &path, Action::Install).unwrap();
    assert!(path.join(METADATA).exists());
}

#[test]
fn invalid_unknown_and_newer_metadata_are_not_overwritten() {
    let f = Fixture::new();
    let path = f.skill();
    manage(&f.0, &path, Action::Install).unwrap();
    for bytes in [
        b"broken".to_vec(),
        br#"{"schema":2,"installed":null,"pending":null}"#.to_vec(),
        br#"{"schema":1,"installed":null,"pending":null}"#.to_vec(),
    ] {
        fs::write(path.join(METADATA), &bytes).unwrap();
        assert!(manage(&f.0, &path, Action::Update).is_err());
        assert_eq!(fs::read(path.join(METADATA)).unwrap(), bytes);
    }
    record(
        &path,
        Some(Release {
            skill_version: VERSION + 1,
            ..Release::bundled()
        }),
        None,
    );
    assert!(
        manage(&f.0, &path, Action::Update)
            .unwrap_err()
            .to_string()
            .contains("downgrade")
    );
    record(
        &path,
        Some(Release {
            sha256: "invalid".into(),
            ..Release::bundled()
        }),
        None,
    );
    assert!(manage(&f.0, &path, Action::Update).is_err());
}

#[test]
fn interrupted_initial_install_and_both_update_boundaries_recover() {
    let f = Fixture::new();
    let path = f.skill();
    fs::create_dir_all(&path).unwrap();
    record(&path, None, Some(Release::bundled()));
    assert!(
        manage(&f.0, &path, Action::Status)
            .unwrap()
            .contains("interrupted")
    );
    manage(&f.0, &path, Action::Install).unwrap();
    fs::write(path.join(PAYLOAD), b"old skill").unwrap();
    record(&path, Some(old()), Some(Release::bundled()));
    manage(&f.0, &path, Action::Update).unwrap();
    record(&path, Some(old()), Some(Release::bundled()));
    manage(&f.0, &path, Action::Update).unwrap();
    let metadata = read_metadata(Some(&fs::read(path.join(METADATA)).unwrap()))
        .unwrap()
        .unwrap();
    assert!(metadata.pending.is_none());
    assert_eq!(metadata.installed, Some(Release::bundled()));
    record(&path, Some(old()), Some(Release::bundled()));
    fs::write(path.join(PAYLOAD), b"user edits after crash").unwrap();
    assert!(manage(&f.0, &path, Action::Update).is_err());
    assert_eq!(
        fs::read(path.join(PAYLOAD)).unwrap(),
        b"user edits after crash"
    );
}

#[test]
fn live_lock_refuses_another_installer_and_releases_on_close() {
    let f = Fixture::new();
    let path = f.skill();
    manage(&f.0, &path, Action::Install).unwrap();
    let directory = Directory::open_existing(&path, false).unwrap();
    let lock = locking::acquire(&directory).unwrap();
    assert!(manage(&f.0, &path, Action::Update).is_err());
    assert!(manage(&f.0, &path, Action::Status).is_ok());
    drop(lock);
    assert!(manage(&f.0, &path, Action::Update).is_ok());
}

#[test]
fn all_targets_report_partial_failure_and_continue() {
    let f = Fixture::new();
    let path = f.0.join(".claude/skills/runyte-mcp");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join(PAYLOAD), "custom").unwrap();
    let mut output = Vec::new();
    let options = options(&["install", "--all", "--user"]).unwrap().unwrap();
    assert!(execute(&options, &f.0, &mut output).is_err());
    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("Claude Code:")
            && output.contains("Codex:")
            && output.contains("installed")
    );
    assert_eq!(fs::read(path.join(PAYLOAD)).unwrap(), b"custom");
    assert_eq!(
        fs::read(f.0.join(".agents/skills/runyte-mcp/SKILL.md")).unwrap(),
        SKILL
    );
}

#[cfg(unix)]
#[test]
fn links_and_nonregular_files_never_replace_their_targets() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let path = f.skill();
    manage(&f.0, &path, Action::Install).unwrap();
    let target = f.0.join("important");
    fs::write(&target, "untouched").unwrap();
    fs::remove_file(path.join(PAYLOAD)).unwrap();
    symlink(&target, path.join(PAYLOAD)).unwrap();
    assert!(manage(&f.0, &path, Action::Update).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"untouched");
    fs::remove_file(path.join(PAYLOAD)).unwrap();
    fs::create_dir(path.join(PAYLOAD)).unwrap();
    assert!(manage(&f.0, &path, Action::Install).is_err());
    let linked = f.0.join("linked");
    symlink(&path, &linked).unwrap();
    assert!(manage(&f.0, &linked, Action::Install).is_err());
}

#[cfg(unix)]
#[test]
fn status_and_conflicts_preserve_file_and_directory_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let path = f.skill();
    fs::create_dir_all(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(path.join(PAYLOAD), SKILL).unwrap();
    fs::set_permissions(path.join(PAYLOAD), fs::Permissions::from_mode(0o644)).unwrap();
    manage(&f.0, &path, Action::Status).unwrap();
    assert_eq!(
        fs::metadata(path.join(PAYLOAD))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    manage(&f.0, &path, Action::Install).unwrap();
    assert_eq!(
        fs::metadata(path.join(PAYLOAD))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    fs::write(path.join(PAYLOAD), "user edits").unwrap();
    assert!(manage(&f.0, &path, Action::Update).is_err());
    assert_eq!(
        fs::metadata(path.join(PAYLOAD))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[test]
fn oversized_files_and_missing_managed_payload_fail_without_replacement() {
    let f = Fixture::new();
    let path = f.skill();
    manage(&f.0, &path, Action::Install).unwrap();
    fs::remove_file(path.join(PAYLOAD)).unwrap();
    assert!(manage(&f.0, &path, Action::Update).is_err());
    fs::write(path.join(PAYLOAD), vec![b'x'; 1024 * 1024 + 1]).unwrap();
    assert!(manage(&f.0, &path, Action::Update).is_err());
    assert_eq!(
        fs::metadata(path.join(PAYLOAD)).unwrap().len(),
        1024 * 1024 + 1
    );
}
