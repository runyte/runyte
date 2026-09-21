// SPDX-License-Identifier: MPL-2.0

//! Local drive documents only. A file URI cannot represent device namespaces
//! or names that depend on verbatim Windows pathname interpretation.
use super::Uri;
use std::{
    path::{Component, Path, PathBuf, Prefix},
    str::FromStr,
};

fn ordinary(path: &Path) -> Option<PathBuf> {
    path.to_str()?; // Never replace unpaired UTF-16 code units.
    let mut components = path.components();
    let drive = match components.next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) if drive.is_ascii_alphabetic() => {
                drive
            }
            _ => return None,
        },
        _ => return None,
    };
    if components.next()? != Component::RootDir {
        return None;
    }
    let mut result = PathBuf::from(format!("{}:\\", drive.to_ascii_uppercase() as char));
    for component in components {
        let Component::Normal(name) = component else {
            return None;
        };
        if name.to_str()?.contains(['/', '\\']) {
            return None;
        }
        crate::windows_fs::validate_relative(Path::new(name)).ok()?;
        result.push(name);
    }
    Some(result)
}

pub(super) fn path_to_uri(path: &Path) -> Option<Uri> {
    let path = ordinary(path)?;
    let url = url::Url::from_file_path(path).ok()?;
    Uri::from_str(url.as_str()).ok()
}

pub(super) fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let (scheme, rest) = uri.as_str().split_once("://")?;
    if !scheme.eq_ignore_ascii_case("file") {
        return None;
    }
    let slash = rest.find('/')?;
    let authority = &rest[..slash];
    if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
        return None;
    }
    let path = &rest[slash..];
    if path.contains(['?', '#', '\\']) || path.chars().any(char::is_control) {
        return None;
    }
    // Validate before URL parsing, which otherwise normalizes dot segments and
    // tolerates malformed escapes. Encoded separators must not introduce new
    // path components after this admission check.
    let mut decoded = Vec::with_capacity(path.len());
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        let byte = if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            let value = (high * 16 + low) as u8;
            if matches!(value, 0 | b'/' | b'\\') {
                return None;
            }
            value
        } else {
            byte
        };
        decoded.push(byte);
    }
    let decoded = String::from_utf8(decoded).ok()?;
    if decoded.split('/').any(|part| matches!(part, "." | "..")) {
        return None;
    }
    let url = url::Url::parse(uri.as_str()).ok()?;
    if url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    ordinary(&url.to_file_path().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};

    #[test]
    fn drive_paths_round_trip_without_losing_unicode_or_literal_escapes() {
        for path in [
            r"C:\plain.rs",
            r"C:\with space\café 😀.rs",
            r"C:\percent%20literal#name.rs",
            r"C:\",
            r"C:\a+b;=,@.rs",
        ] {
            let uri = path_to_uri(Path::new(path)).unwrap();
            assert!(uri.as_str().starts_with("file:///C:/"));
            assert_eq!(uri_to_path(&uri), Some(PathBuf::from(path)));
            assert_eq!(path_to_uri(Path::new(&format!(r"\\?\{path}"))), Some(uri));
        }
        assert_eq!(
            path_to_uri(Path::new(r"c:\a.rs")),
            path_to_uri(Path::new(r"C:\a.rs"))
        );
        let long = format!(r"C:\{}\a.rs", vec!["segment"; 80].join("\\"));
        assert_eq!(
            uri_to_path(&path_to_uri(Path::new(&long)).unwrap()),
            Some(PathBuf::from(long))
        );
    }

    #[test]
    fn namespaces_and_names_without_an_equivalent_local_uri_are_refused() {
        for path in [
            r"relative.rs",
            r"C:relative.rs",
            r"\rooted.rs",
            r"\\server\share\a.rs",
            r"\\?\UNC\server\share\a.rs",
            r"\\.\pipe\name",
            r"\\?\C:\name.",
            r"\\?\C:\name ",
            r"C:\a.rs:stream",
            r"C:\NUL",
            r"C:\a\..\b.rs",
            r"\\?\C:\a/b.rs",
        ] {
            assert!(path_to_uri(Path::new(path)).is_none(), "{path}");
        }
        let path = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd800, 46, 114, 115]));
        assert!(path_to_uri(&path).is_none());
    }

    #[test]
    fn incoming_uris_require_unambiguous_local_drive_paths() {
        for text in [
            "file:///c:/a.rs",
            "file://localhost/C:/a.rs",
            "file:///C%3A/a.rs",
        ] {
            assert_eq!(
                uri_to_path(&Uri::from_str(text).unwrap()),
                Some(PathBuf::from(r"C:\a.rs"))
            );
        }
        for text in [
            "file://server/C:/a.rs",
            "file://localhostevil/C:/a.rs",
            "file:////server/share/a.rs",
            "file:///C:/a.rs?query",
            "file:///C:/a.rs#fragment",
            "file:///C:/a%00.rs",
            "file:///C:/a%2Fb.rs",
            "file:///C:/a%5Cb.rs",
            "file:///C:/%FF.rs",
            "file:///C:/a.rs:stream",
            "file:///C:/a/%2e%2e/b.rs",
            "file:///C:/NUL",
            "file:///C:/name.",
            "file:///tmp/a.rs",
            "file://C:/a.rs",
            "file:///C:/a%2",
        ] {
            // Uri itself rejects some malformed spellings. The Windows
            // conversion must refuse everything that reaches it as well.
            if let Ok(uri) = Uri::from_str(text) {
                assert!(uri_to_path(&uri).is_none(), "{text}");
            }
        }
    }
}
