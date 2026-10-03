// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::App,
    command::parse_named_command,
    indentation::Scope,
    settings::{SettingId, SettingValue, persist_override, persist_setting},
};
use std::{
    ffi::CString,
    os::unix::{ffi::OsStrExt, fs::FileTypeExt},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn special_file_config_reads_refuse_without_blocking() {
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "runyte-config-special-file-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    )));
    fs::create_dir_all(&fixture.0).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "config::special_file_tests::special_file_config_fixture",
            "--nocapture",
        ])
        .env("RUNYTE_CONFIG_SPECIAL_FILE_TEST_ROOT", &fixture.0)
        .env("XDG_CONFIG_HOME", fixture.0.join("xdg"))
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "config special-file fixture failed: {status}"
            );
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("configuration I/O blocked on a special file");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "owned by the bounded config special-file parent fixture"]
fn special_file_config_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_CONFIG_SPECIAL_FILE_TEST_ROOT").unwrap());
    let path = root.join("config.yaml");
    fs::write(&path, "editor: {tab_width: 7}\n").unwrap();
    let (config, _) = Config::load(Some(&path)).unwrap();
    let mut app = App::new(config, None).unwrap();
    app.note_loaded_config(&path);
    fs::remove_file(&path).unwrap();
    let native_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: native_path is NUL-terminated and the FIFO is fixture-owned.
    assert_eq!(unsafe { libc::mkfifo(native_path.as_ptr(), 0o600) }, 0);
    let alias = root.join("alias.yaml");
    std::os::unix::fs::symlink(&path, &alias).unwrap();

    let outcome = app
        .execute(parse_named_command("config-reload", None).unwrap())
        .unwrap();
    assert!(matches!(outcome, crate::app::CommandOutcome::UserError(_)));
    assert_eq!(app.config.editor.tab_width, 7);

    for candidate in [&path, &alias, Path::new("/dev/null")] {
        assert!(Config::load(Some(candidate)).is_err());
        assert!(Config::reload(candidate).is_err());
        assert!(
            persist_setting(
                candidate,
                SettingId::EditorTabWidth,
                &SettingValue::Integer(8)
            )
            .is_err()
        );
        assert!(
            persist_override(
                candidate,
                SettingId::EditorTabWidth,
                &Scope::Language("rust".into()),
                Some(&SettingValue::Integer(8))
            )
            .is_err()
        );
    }
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_fifo());
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    fs::remove_file(&path).unwrap();
    assert!(Config::load(Some(&path)).is_ok());
    assert!(Config::reload(&path).unwrap().is_none());
    persist_setting(&path, SettingId::EditorTabWidth, &SettingValue::Integer(8)).unwrap();
    assert_eq!(Config::load(Some(&alias)).unwrap().0.editor.tab_width, 8);
    persist_setting(&alias, SettingId::EditorTabWidth, &SettingValue::Integer(6)).unwrap();
    assert_eq!(Config::reload(&alias).unwrap().unwrap().editor.tab_width, 6);
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
