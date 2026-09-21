// SPDX-License-Identifier: MPL-2.0

use super::*;

fn resolve(values: &[(&str, &str)]) -> Option<PathBuf> {
    config_root_from(|name| {
        values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| OsString::from(value))
    })
}

#[test]
fn xdg_override_precedes_platform_default() {
    assert_eq!(
        resolve(&[
            ("XDG_CONFIG_HOME", "custom config"),
            ("APPDATA", "roaming"),
            ("HOME", "home"),
        ]),
        Some(PathBuf::from("custom config").join("runyte")),
    );
}

#[test]
fn missing_or_empty_environment_has_no_configuration_root() {
    assert_eq!(resolve(&[]), None);
    assert_eq!(
        resolve(&[("XDG_CONFIG_HOME", ""), ("APPDATA", ""), ("HOME", "")]),
        None,
    );
}

#[test]
fn empty_xdg_override_uses_platform_default() {
    let result = resolve(&[
        ("XDG_CONFIG_HOME", ""),
        ("APPDATA", "roaming"),
        ("HOME", "home"),
    ]);
    #[cfg(windows)]
    assert_eq!(result, Some(PathBuf::from("roaming").join("runyte")));
    #[cfg(not(windows))]
    assert_eq!(
        result,
        Some(PathBuf::from("home").join(".config").join("runyte")),
    );
}

#[cfg(windows)]
#[test]
fn windows_uses_roaming_appdata_without_home() {
    assert_eq!(
        resolve(&[("APPDATA", r"C:\Users\Example\AppData\Roaming")]),
        Some(PathBuf::from(r"C:\Users\Example\AppData\Roaming\runyte")),
    );
    assert_eq!(resolve(&[("HOME", r"C:\Users\Example")]), None);
}

#[cfg(windows)]
#[test]
fn windows_preserves_unicode_unc_and_verbatim_paths() {
    for root in [
        r"C:\Users\Zoë Example\AppData\Roaming",
        r"\\server\profiles\配置",
        r"\\?\C:\Users\Example\AppData\Roaming",
    ] {
        assert_eq!(
            resolve(&[("APPDATA", root)]),
            Some(PathBuf::from(root).join("runyte")),
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_preserves_non_unicode_path_units() {
    use std::os::windows::ffi::OsStringExt;
    let root = OsString::from_wide(&[b'C' as u16, b':' as u16, b'\\' as u16, 0xd800]);
    assert_eq!(
        config_root_from(|name| (name == "APPDATA").then(|| root.clone())),
        Some(PathBuf::from(root).join("runyte")),
    );
}

#[cfg(not(windows))]
#[test]
fn unix_uses_home_config_and_ignores_appdata() {
    assert_eq!(
        resolve(&[("HOME", "/home/example"), ("APPDATA", "/roaming")]),
        Some(PathBuf::from("/home/example/.config/runyte")),
    );
    assert_eq!(resolve(&[("APPDATA", "/roaming")]), None);
}
