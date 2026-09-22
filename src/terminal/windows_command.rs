// SPDX-License-Identifier: MPL-2.0

//! Native command-line framing. Shell syntax is interpreted only by an
//! explicitly requested shell; PATH lookup never searches the workspace.
use std::{
    ffi::{OsStr, OsString},
    io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};

pub(super) fn split(value: &str) -> Option<Vec<String>> {
    use windows_sys::Win32::{Foundation::LocalFree, UI::Shell::CommandLineToArgvW};
    if value.contains('\0') || value.len() > 32767 {
        return None;
    }
    let wide: Vec<_> = value.encode_utf16().chain(Some(0)).collect();
    let mut count = 0;
    let argv = unsafe { CommandLineToArgvW(wide.as_ptr(), &mut count) };
    if argv.is_null() {
        return None;
    }
    let result = (0..count as usize)
        .map(|index| {
            let value = unsafe { *argv.add(index) };
            let mut len = 0;
            while unsafe { *value.add(len) } != 0 {
                len += 1;
            }
            String::from_utf16(unsafe { std::slice::from_raw_parts(value, len) }).ok()
        })
        .collect();
    unsafe {
        LocalFree(argv.cast());
    }
    result
}

pub(super) fn line(program: &OsStr, arguments: &[String]) -> io::Result<Vec<u16>> {
    if Path::new(program)
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("cmd.exe"))
        && let Some(index) = arguments
            .iter()
            .position(|arg| arg.eq_ignore_ascii_case("/c") || arg.eq_ignore_ascii_case("/k"))
    {
        let [payload] = &arguments[index + 1..] else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cmd.exe /c and /k require one quoted command string",
            ));
        };
        if payload.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "command contains NUL",
            ));
        }
        let mut prefix = arguments[..index].to_vec();
        if !prefix.iter().any(|arg| arg.eq_ignore_ascii_case("/s")) {
            prefix.push("/s".into());
        }
        prefix.push(arguments[index].clone());
        let mut line = crt_line(program, &prefix)?;
        line.pop();
        // /s strips exactly these outer quotes. Inside is explicitly authored
        // shell text; CRT backslash escaping would alter quotes and commands.
        line.extend(" \"".encode_utf16());
        line.extend(payload.encode_utf16());
        line.extend([b'"' as u16, 0]);
        if line.len() > 32767 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "command line is too long",
            ));
        }
        return Ok(line);
    }
    crt_line(program, arguments)
}

fn crt_line(program: &OsStr, arguments: &[String]) -> io::Result<Vec<u16>> {
    let mut line = Vec::new();
    for (index, value) in std::iter::once(program)
        .chain(arguments.iter().map(OsStr::new))
        .enumerate()
    {
        if index != 0 {
            line.push(b' ' as u16);
        }
        let units: Vec<_> = value.encode_wide().collect();
        if units.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "command contains NUL",
            ));
        }
        if index != 0 && !units.is_empty() && !units.iter().any(|unit| matches!(*unit, 9 | 32 | 34))
        {
            line.extend(units);
            continue;
        }
        // Double backslashes before quotes and the closing quote. Leave bare
        // switches bare: cmd.exe does not use the CRT parser for its switches.
        line.push(b'"' as u16);
        let mut slashes = 0;
        for unit in units {
            if unit == b'\\' as u16 {
                slashes += 1;
                continue;
            }
            line.extend(std::iter::repeat_n(
                b'\\' as u16,
                if unit == b'"' as u16 {
                    slashes * 2 + 1
                } else {
                    slashes
                },
            ));
            slashes = 0;
            line.push(unit);
        }
        line.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        line.push(b'"' as u16);
    }
    if line.len() >= 32767 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command line is too long",
        ));
    }
    line.push(0);
    Ok(line)
}

pub(super) fn resolve_for_terminal(
    command: &Path,
    directory: &Path,
    search_path: Option<&OsStr>,
    pathext: Option<&OsStr>,
) -> Option<PathBuf> {
    if !command.is_absolute() && command.components().count() > 1 {
        if matches!(
            command.components().next(),
            Some(std::path::Component::Prefix(_))
        ) {
            return None;
        }
        return resolve(&directory.join(command), search_path, pathext);
    }
    resolve(command, search_path, pathext)
}

/// ConPTY startup can stall with an extended-prefix current directory even
/// when ordinary CreateProcess accepts it. Use the equivalent DOS/UNC spelling
/// only after proving it names the same directory; never reinterpret an entry
/// that depends on verbatim name semantics.
pub(super) fn working_directory(path: &Path) -> io::Result<PathBuf> {
    crate::windows_fs::ordinary_working_directory(path)
}

pub(crate) fn resolve(
    command: &Path,
    search_path: Option<&OsStr>,
    pathext: Option<&OsStr>,
) -> Option<PathBuf> {
    let extensions: Vec<_> = pathext
        .unwrap_or(OsStr::new(".COM;.EXE;.BAT;.CMD"))
        .to_string_lossy()
        .split(';')
        .filter(|s| s.starts_with('.') && !s.contains(['/', '\\', ':']))
        .map(str::to_owned)
        .collect();
    let inspect = |path: PathBuf| {
        if path.is_file() {
            return Some(path);
        }
        if path.extension().is_some() {
            return None;
        }
        extensions
            .iter()
            .map(|extension| {
                let mut name = path.as_os_str().to_os_string();
                name.push(extension);
                PathBuf::from(name)
            })
            .find(|path| path.is_file())
    };
    if command.is_absolute() || command.components().count() > 1 {
        return inspect(command.to_path_buf());
    }
    crate::service_health::search_directories(search_path?)
        .filter(|directory| directory.is_absolute())
        .find_map(|directory| inspect(directory.join(command)))
}

pub(super) fn environment(parent_context: Option<&str>) -> Vec<u16> {
    let mut values: Vec<_> = std::env::vars_os()
        .filter(|(name, _)| {
            ![
                "TERM",
                "COLORTERM",
                "TERM_PROGRAM",
                "TERM_PROGRAM_VERSION",
                "RUNYTE_PARENT_CONTEXT",
            ]
            .iter()
            .any(|blocked| name.to_string_lossy().eq_ignore_ascii_case(blocked))
        })
        .collect();
    values.extend([
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("COLORTERM"), OsString::from("truecolor")),
        (
            OsString::from(crate::workspace::parent::ENVIRONMENT),
            OsString::from(parent_context.unwrap_or("standalone")),
        ),
    ]);
    values.sort_by(|a, b| {
        crate::windows_fs::compare_names(
            &a.0.encode_wide().collect::<Vec<_>>(),
            &b.0.encode_wide().collect::<Vec<_>>(),
        )
    });
    let mut block = Vec::new();
    for (name, value) in values {
        block.extend(name.encode_wide());
        block.push(b'=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStringExt;

    fn environment_value(block: &[u16], expected: &str) -> Vec<String> {
        block
            .split(|unit| *unit == 0)
            .filter(|value| !value.is_empty())
            .map(OsString::from_wide)
            .filter_map(|entry| {
                let entry = entry.to_string_lossy();
                let (name, value) = entry.split_once('=')?;
                name.eq_ignore_ascii_case(expected)
                    .then(|| value.to_owned())
            })
            .collect()
    }

    #[test]
    fn native_environment_installs_only_the_explicit_parent_marker() {
        assert_eq!(
            environment_value(&environment(None), crate::workspace::parent::ENVIRONMENT),
            ["standalone"]
        );
        assert_eq!(
            environment_value(
                &environment(Some("exact-native-parent")),
                crate::workspace::parent::ENVIRONMENT
            ),
            ["exact-native-parent"]
        );
    }

    #[test]
    fn native_quoted_arguments_round_trip() {
        let args: Vec<_> = [
            "",
            r"C:\folder with spaces\",
            "Bob's notes",
            "embedded \"quote\"",
            "café 😀",
            "a&b",
        ]
        .map(str::to_owned)
        .into();
        let encoded = line(OsStr::new(r"C:\Program Files\app.exe"), &args).unwrap();
        let decoded =
            split(&OsString::from_wide(&encoded[..encoded.len() - 1]).to_string_lossy()).unwrap();
        assert_eq!(decoded[0], r"C:\Program Files\app.exe");
        assert_eq!(&decoded[1..], args);
    }

    #[test]
    fn discovery_uses_injected_pathext_and_ignores_relative_path_entries() {
        let root = crate::test_support::TestRuntimeRoot::new("command-lookup").unwrap();
        std::fs::write(root.join("native.EXE"), "lookup fixture, never executed").unwrap();
        let search =
            std::env::join_paths([Path::new(""), Path::new("relative"), root.path()]).unwrap();
        assert_eq!(
            resolve(
                Path::new("native"),
                Some(&search),
                Some(OsStr::new(".EXE;.CMD"))
            ),
            Some(root.join("native.EXE"))
        );
        assert_eq!(
            resolve(Path::new("native"), Some(&search), Some(OsStr::new(".CMD"))),
            None
        );
        assert_eq!(
            resolve(Path::new("native"), Some(OsStr::new(";relative;")), None),
            None
        );
    }

    #[test]
    fn explicit_relative_program_uses_captured_terminal_directory() {
        let root = crate::test_support::TestRuntimeRoot::new("relative-program").unwrap();
        let requested = root.join("requested");
        std::fs::create_dir(&requested).unwrap();
        std::fs::write(root.join("tool.exe"), "not executed").unwrap();
        std::fs::write(requested.join("tool.exe"), "not executed").unwrap();
        assert_eq!(
            resolve_for_terminal(
                Path::new(r".\tool.exe"),
                &requested,
                Some(root.as_os_str()),
                None
            ),
            Some(requested.join(r".\tool.exe"))
        );
        assert_eq!(
            resolve_for_terminal(
                Path::new("tool.exe"),
                &requested,
                Some(root.as_os_str()),
                None
            ),
            Some(root.join("tool.exe"))
        );
        assert!(resolve_for_terminal(Path::new(r"C:tool.exe"), &requested, None, None).is_none());
    }

    #[test]
    fn terminal_directory_conversion_refuses_verbatim_only_and_long_names() {
        let root = crate::test_support::TestRuntimeRoot::new("terminal-cwd").unwrap();
        let ordinary = working_directory(root.path()).unwrap();
        assert!(!ordinary.as_os_str().to_string_lossy().starts_with(r"\\?\"));
        assert_eq!(
            crate::windows_fs::Identity::read(&ordinary).unwrap(),
            crate::windows_fs::Identity::read(root.path()).unwrap()
        );
        let special = root.join("trailing.");
        std::fs::create_dir(&special).unwrap();
        assert_eq!(
            working_directory(&special).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        let mut long = root.path().to_path_buf();
        while long.as_os_str().encode_wide().count() < 300 {
            long.push("long-directory-component");
        }
        std::fs::create_dir_all(&long).unwrap();
        assert_eq!(
            working_directory(&long).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }
}
