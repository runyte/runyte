// SPDX-License-Identifier: MPL-2.0

//! Opening files Runyte should not try to be an editor for.
//!
//! A binary file loaded as text is a screenful of replacement characters and a
//! buffer that cannot be saved back without destroying it. Runyte instead asks
//! which program should have the file and hands it over. The platform opener is
//! selected initially, while explicit answers are remembered and can be made
//! the default for later files.
//!
//! Nothing here knows about buffers, panes, or drawing: detection is a
//! predicate over bytes, the cache is a list of strings on disk, and launching
//! is one process spawn.

use std::{
    fs,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

/// How much of a file is examined before calling it text.
///
/// The same prefix Git inspects. A file whose first eight kilobytes are clean
/// text is text for every practical purpose, and reading further to be sure
/// would mean reading files Runyte is about to hand to someone else anyway.
const PREFIX_BYTES: usize = 8192;

/// How many programs the cache keeps.
///
/// Long enough that the tools someone actually uses stay in it, short enough
/// that the hint list stays readable.
const MAX_PROGRAMS: usize = 16;

const CACHE_FILE: &str = "recent-programs";
const DEFAULT_FILE: &str = "default-program";

// Non-host variants are constructed by the platform-mapping tests.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OpenPlatform {
    Linux,
    MacOs,
    Unsupported,
}

impl OpenPlatform {
    #[cfg(target_os = "linux")]
    const CURRENT: Self = Self::Linux;
    #[cfg(target_os = "macos")]
    const CURRENT: Self = Self::MacOs;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    const CURRENT: Self = Self::Unsupported;
}

fn system_default_program_for(platform: OpenPlatform) -> Option<&'static str> {
    match platform {
        OpenPlatform::Linux => Some("xdg-open"),
        OpenPlatform::MacOs => Some("open"),
        OpenPlatform::Unsupported => None,
    }
}

/// The command that asks this platform to use the file's preferred app.
pub fn system_default_program() -> Option<&'static str> {
    system_default_program_for(OpenPlatform::CURRENT)
}

fn launch_program_for(program: &str, platform: OpenPlatform) -> Result<&str> {
    let explicit = program.trim();
    if !explicit.is_empty() {
        return Ok(explicit);
    }
    system_default_program_for(platform)
        .context("no system default opener is available on this platform; type a program")
}

/// Whether a prefix of a file's bytes reads as binary.
///
/// A NUL byte settles it, and so does invalid UTF-8: Runyte's buffers are
/// UTF-8, so text it cannot decode is text it cannot edit either.
///
/// `complete` says whether `prefix` is the whole file. When it is not, a
/// decoding error in the last few bytes is a character split across the
/// boundary rather than a binary file, so only an error that ends before them
/// counts.
pub fn is_binary(prefix: &[u8], complete: bool) -> bool {
    if prefix.contains(&0) {
        return true;
    }
    match std::str::from_utf8(prefix) {
        Ok(_) => false,
        Err(error) => complete || error.error_len().is_some(),
    }
}

/// Whether the file at `path` should be handed to another program.
///
/// A path that cannot be read is not binary: the caller's own open reports
/// that failure with the message it wants.
pub fn looks_binary(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    // One byte past the prefix, which is what tells a file of exactly
    // `PREFIX_BYTES` from a longer one. Without it a file that ends on the
    // boundary would be read as truncated, and a genuinely invalid trailing
    // sequence would be forgiven as a character split by the cut.
    let mut prefix = vec![0; PREFIX_BYTES + 1];
    let mut filled = 0;
    while filled < prefix.len() {
        match file.read(&mut prefix[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(_) => return false,
        }
    }
    let complete = filled <= PREFIX_BYTES;
    prefix.truncate(filled.min(PREFIX_BYTES));
    is_binary(&prefix, complete)
}

/// The platform cache directory for Runyte's regenerable per-user state.
///
/// An explicit `XDG_CACHE_HOME` wins on every platform. Otherwise Linux and
/// other Unix systems use `<account-home>/.cache/runyte`, macOS uses
/// `<account-home>/Library/Caches/runyte`, and Windows uses
/// `%LOCALAPPDATA%/runyte/cache`. Unix account home comes from the effective
/// user's account record rather than inherited `$HOME`, so a privileged
/// invocation cannot leave its files in another user's default cache.
pub fn cache_root() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let environment_home = std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(unix)]
    let account_home = crate::user_paths::system_home_directory();
    #[cfg(not(unix))]
    let account_home = None;
    cache_root_for(
        CachePlatform::CURRENT,
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
        environment_home,
        account_home,
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
    )
}

// Non-host variants are constructed by the platform-mapping tests.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CachePlatform {
    MacOs,
    Windows,
    Unix,
}

impl CachePlatform {
    #[cfg(target_os = "macos")]
    const CURRENT: Self = Self::MacOs;
    #[cfg(target_os = "windows")]
    const CURRENT: Self = Self::Windows;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    const CURRENT: Self = Self::Unix;
}

fn cache_root_for(
    platform: CachePlatform,
    xdg_cache_home: Option<PathBuf>,
    _environment_home: Option<PathBuf>,
    account_home: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(root) = xdg_cache_home.filter(|path| path.is_absolute()) {
        return Some(root.join("runyte"));
    }
    match platform {
        CachePlatform::MacOs => Some(account_home?.join("Library/Caches/runyte")),
        CachePlatform::Windows => Some(local_app_data?.join("runyte/cache")),
        CachePlatform::Unix => Some(account_home?.join(".cache/runyte")),
    }
}

/// Programs recently chosen for binary files, most recent first.
#[derive(Clone, Debug, Default)]
pub struct ProgramCache {
    root: Option<PathBuf>,
    programs: Vec<String>,
    default_program: Option<String>,
}

impl ProgramCache {
    /// Reads the cache, treating any failure as an empty one.
    ///
    /// A cache that cannot be read is worth no error: it costs the reader a
    /// hint, not their file.
    pub fn load(root: Option<PathBuf>) -> Self {
        let mut programs: Vec<String> = root
            .as_ref()
            .and_then(|root| fs::read_to_string(root.join(CACHE_FILE)).ok())
            .map(|contents| {
                contents
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .take(MAX_PROGRAMS)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let default_program = root
            .as_ref()
            .and_then(|root| fs::read_to_string(root.join(DEFAULT_FILE)).ok())
            .map(|program| program.trim().to_owned())
            .filter(|program| !program.is_empty());
        if let Some(program) = &default_program
            && !programs.contains(program)
        {
            programs.insert(0, program.clone());
            programs.truncate(MAX_PROGRAMS);
        }
        Self {
            root,
            programs,
            default_program,
        }
    }

    pub fn programs(&self) -> &[String] {
        &self.programs
    }

    /// A custom default, or `None` when the platform opener is the default.
    pub fn default_program(&self) -> Option<&str> {
        self.default_program.as_deref()
    }

    /// The cached programs a partly typed name could still become, in cache
    /// order so the most recently used one is offered first.
    pub fn matching(&self, prefix: &str) -> Vec<&str> {
        let prefix = prefix.trim();
        self.programs
            .iter()
            .filter(|program| prefix.is_empty() || program.starts_with(prefix))
            .map(String::as_str)
            .collect()
    }

    /// Moves a program to the front, writing the cache back.
    ///
    /// Returns the write error rather than swallowing it, so a cache directory
    /// that cannot be created is reported once instead of silently forgetting
    /// every choice.
    pub fn remember(&mut self, program: &str) -> Result<()> {
        let program = program.trim();
        if program.is_empty() {
            return Ok(());
        }
        self.programs.retain(|known| known != program);
        self.programs.insert(0, program.to_owned());
        self.programs.truncate(MAX_PROGRAMS);
        self.persist_programs()
    }

    /// Makes `program` the default choice, or restores the platform default.
    pub fn set_default(&mut self, program: Option<&str>) -> Result<()> {
        let program = program.map(str::trim).filter(|program| !program.is_empty());
        if let Some(program) = program {
            self.remember(program)?;
        }
        self.default_program = program.map(str::to_owned);
        self.persist_default()
    }

    /// Removes a remembered program and restores the platform default if needed.
    pub fn forget(&mut self, program: &str) -> Result<bool> {
        let program = program.trim();
        let before = self.programs.len();
        self.programs.retain(|known| known != program);
        let removed = self.programs.len() != before;
        let default_removed = self.default_program.as_deref() == Some(program);
        if default_removed {
            self.default_program = None;
        }
        if removed {
            self.persist_programs()?;
        }
        if default_removed {
            self.persist_default()?;
        }
        Ok(removed)
    }

    fn persist_programs(&self) -> Result<()> {
        let Some(root) = self.root.clone() else {
            return Ok(());
        };
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        let mut contents = self.programs.join("\n");
        contents.push('\n');
        fs::write(root.join(CACHE_FILE), contents)
            .with_context(|| format!("failed to write {}", root.join(CACHE_FILE).display()))?;
        Ok(())
    }

    fn persist_default(&self) -> Result<()> {
        let Some(root) = self.root.clone() else {
            return Ok(());
        };
        let path = root.join(DEFAULT_FILE);
        let Some(program) = &self.default_program else {
            return match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
                Err(error) => {
                    Err(error).with_context(|| format!("failed to remove {}", path.display()))
                }
            };
        };
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        fs::write(&path, format!("{program}\n"))
            .with_context(|| format!("failed to write {}", path.display()))
    }
}

/// Hands `path` to `program` and returns without waiting for it.
///
/// Detached with no stdio of its own, because Runyte owns the terminal in raw
/// mode: a child writing to the same screen would corrupt it, and a child
/// reading from the same keyboard would fight the editor for every keystroke.
/// The trade-off is that a terminal program cannot take the screen over; the
/// intended targets are viewers and GUI applications.
///
/// An empty `program` uses `xdg-open` on Linux and `open` on macOS. An explicit
/// program may carry arguments, split on whitespace, so `feh` and `code -w`
/// are both spellable. The path is always passed as one final argument.
pub fn launch(program: &str, path: &Path) -> Result<()> {
    let program = launch_program_for(program, OpenPlatform::CURRENT)?;
    let mut words = program.split_whitespace();
    let executable = words.next().context("no program was given")?;
    let mut command = Command::new(executable);
    command
        .args(words)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .spawn()
        .with_context(|| format!("failed to run {executable}"))?;
    // Reaped on a thread of its own. Nothing waits on the exit status — the
    // whole point is not to block the editor — but a child nobody waits for
    // stays in the process table until Runyte itself exits, and a session
    // spent opening images should not accumulate one zombie per image.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_default_openers_follow_desktop_platform_conventions() {
        assert_eq!(
            system_default_program_for(OpenPlatform::Linux),
            Some("xdg-open")
        );
        assert_eq!(
            system_default_program_for(OpenPlatform::MacOs),
            Some("open")
        );
        assert_eq!(system_default_program_for(OpenPlatform::Unsupported), None);
    }

    #[test]
    fn an_empty_choice_uses_the_system_default_but_an_explicit_one_wins() {
        assert_eq!(
            launch_program_for("", OpenPlatform::Linux).unwrap(),
            "xdg-open"
        );
        assert_eq!(
            launch_program_for("   ", OpenPlatform::MacOs).unwrap(),
            "open"
        );
        assert_eq!(
            launch_program_for("  code -w  ", OpenPlatform::Linux).unwrap(),
            "code -w"
        );
        assert!(launch_program_for("", OpenPlatform::Unsupported).is_err());
    }

    #[test]
    fn cache_paths_follow_effective_account_conventions_and_honor_xdg() {
        let environment_home = PathBuf::from("/home/invoking");
        let account_home = PathBuf::from("/home/effective");
        let local = PathBuf::from("C:/Users/example/AppData/Local");

        assert_eq!(
            cache_root_for(
                CachePlatform::Unix,
                None,
                Some(environment_home.clone()),
                Some(account_home.clone()),
                None,
            ),
            Some(account_home.join(".cache/runyte"))
        );
        assert_eq!(
            cache_root_for(
                CachePlatform::MacOs,
                None,
                Some(environment_home.clone()),
                Some(account_home.clone()),
                None,
            ),
            Some(account_home.join("Library/Caches/runyte"))
        );
        assert_eq!(
            cache_root_for(
                CachePlatform::Windows,
                None,
                Some(environment_home.clone()),
                None,
                Some(local.clone()),
            ),
            Some(local.join("runyte/cache"))
        );
        assert_eq!(
            cache_root_for(
                CachePlatform::MacOs,
                Some(PathBuf::from("/custom/cache")),
                Some(environment_home),
                Some(account_home),
                None,
            ),
            Some(PathBuf::from("/custom/cache/runyte"))
        );
    }

    #[test]
    fn relative_xdg_cache_home_is_ignored() {
        assert_eq!(
            cache_root_for(
                CachePlatform::Unix,
                Some(PathBuf::from("relative")),
                Some(PathBuf::from("/home/invoking")),
                Some(PathBuf::from("/home/example")),
                None,
            ),
            Some(PathBuf::from("/home/example/.cache/runyte"))
        );
    }

    #[test]
    fn text_is_not_binary_and_nul_bytes_are() {
        assert!(!is_binary(b"", true));
        assert!(!is_binary("hello \u{1f600} world\n".as_bytes(), true));
        assert!(is_binary(b"ELF\0\x01\x02", true));
        assert!(is_binary(b"\xff\xfe not utf-8", true));
    }

    /// A prefix can end mid-character, and cutting a valid file in half must
    /// not turn it binary.
    #[test]
    fn a_character_split_by_the_prefix_boundary_is_still_text() {
        let text = "aaa\u{1f600}".as_bytes();
        let truncated = &text[..text.len() - 2];
        assert!(!is_binary(truncated, false));
        assert!(is_binary(truncated, true), "a truncated file really is bad");
    }

    #[test]
    fn a_binary_file_on_disk_is_detected_and_a_text_one_is_not() {
        let directory = std::env::temp_dir().join(format!("runyte-binary-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let text = directory.join("notes.txt");
        let binary = directory.join("image.png");
        fs::write(&text, "plain text\n").unwrap();
        fs::write(&binary, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x00]).unwrap();

        assert!(!looks_binary(&text));
        assert!(looks_binary(&binary));
        assert!(!looks_binary(&directory.join("missing")));

        fs::remove_dir_all(directory).unwrap();
    }

    /// A file that ends exactly on the prefix boundary is a whole file, not a
    /// truncated one, so an invalid sequence at its end is really invalid.
    #[test]
    fn a_file_ending_on_the_prefix_boundary_is_read_as_complete() {
        let directory =
            std::env::temp_dir().join(format!("runyte-boundary-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();

        let exact = directory.join("exact.bin");
        let mut bytes = vec![b'a'; PREFIX_BYTES - 2];
        // The first two bytes of a four-byte character, and nothing after.
        bytes.extend_from_slice(&"\u{1f600}".as_bytes()[..2]);
        assert_eq!(bytes.len(), PREFIX_BYTES);
        fs::write(&exact, &bytes).unwrap();
        assert!(looks_binary(&exact));

        // The same cut inside a longer, valid file is just a split character.
        let longer = directory.join("longer.txt");
        let mut valid = vec![b'a'; PREFIX_BYTES - 2];
        valid.extend_from_slice("\u{1f600}".as_bytes());
        valid.extend_from_slice(b"trailing text\n");
        fs::write(&longer, &valid).unwrap();
        assert!(!looks_binary(&longer));

        // And a whole file of exactly the prefix length that is valid text.
        let clean = directory.join("clean.txt");
        fs::write(&clean, vec![b'a'; PREFIX_BYTES]).unwrap();
        assert!(!looks_binary(&clean));

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn remembering_moves_a_program_to_the_front_and_survives_a_reload() {
        let root = std::env::temp_dir().join(format!("runyte-programs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let mut cache = ProgramCache::load(Some(root.clone()));
        assert!(cache.programs().is_empty());
        cache.remember("feh").unwrap();
        cache.remember("xdg-open").unwrap();
        cache.remember("feh").unwrap();
        assert_eq!(cache.programs(), ["feh", "xdg-open"]);

        let reloaded = ProgramCache::load(Some(root.clone()));
        assert_eq!(reloaded.programs(), ["feh", "xdg-open"]);
        assert_eq!(reloaded.matching("x"), ["xdg-open"]);
        assert_eq!(reloaded.matching(""), ["feh", "xdg-open"]);
        assert!(reloaded.matching("z").is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_custom_default_and_program_deletion_survive_reload() {
        let root =
            std::env::temp_dir().join(format!("runyte-program-default-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let mut cache = ProgramCache::load(Some(root.clone()));
        cache.remember("feh").unwrap();
        cache.set_default(Some("loupe")).unwrap();
        assert_eq!(cache.programs(), ["loupe", "feh"]);
        assert_eq!(cache.default_program(), Some("loupe"));

        let mut reloaded = ProgramCache::load(Some(root.clone()));
        assert_eq!(reloaded.programs(), ["loupe", "feh"]);
        assert_eq!(reloaded.default_program(), Some("loupe"));
        assert!(reloaded.forget("loupe").unwrap());
        assert_eq!(reloaded.programs(), ["feh"]);
        assert_eq!(reloaded.default_program(), None);

        let reloaded = ProgramCache::load(Some(root.clone()));
        assert_eq!(reloaded.programs(), ["feh"]);
        assert_eq!(reloaded.default_program(), None);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_cache_is_bounded_and_ignores_empty_choices() {
        let root = std::env::temp_dir().join(format!("runyte-programs-cap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let mut cache = ProgramCache::load(Some(root.clone()));
        cache.remember("   ").unwrap();
        assert!(cache.programs().is_empty());
        for index in 0..MAX_PROGRAMS + 4 {
            cache.remember(&format!("program{index}")).unwrap();
        }
        assert_eq!(cache.programs().len(), MAX_PROGRAMS);
        assert_eq!(cache.programs()[0], format!("program{}", MAX_PROGRAMS + 3));

        fs::remove_dir_all(root).unwrap();
    }

    /// Without a home directory there is nowhere to write, but the cache must
    /// still work for the session it is in.
    #[test]
    fn a_cache_with_no_home_still_remembers_in_memory() {
        let mut cache = ProgramCache::load(None);
        cache.remember("feh").unwrap();
        assert_eq!(cache.programs(), ["feh"]);
    }

    /// Installs a program at `program` whose behavior is `behavior`.
    ///
    /// A test must never run a file it wrote itself. The write leaves a
    /// descriptor open, a concurrent fork elsewhere in this binary inherits
    /// it, and the exec is then refused with `ETXTBSY` — but only when the
    /// machine is loaded enough for the two to overlap, which is why it
    /// surfaces as an unrelated flake. So the runnable file is checked in at
    /// `src/fixtures/stand-in`, this only links to it, and the behavior
    /// travels beside the link in an ordinary data file that nothing execs.
    #[cfg(unix)]
    fn install_stand_in(program: &Path, behavior: &str) {
        let mut data = program.as_os_str().to_owned();
        data.push(".behavior");
        fs::write(PathBuf::from(data), behavior).unwrap();
        std::os::unix::fs::symlink(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
            program,
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn launched_program_has_a_process_group_separate_from_the_editor() {
        let root = std::env::temp_dir().join(format!(
            "runyte-detached-open-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let helper = root.join("record-group");
        let output = root.join("group");
        install_stand_in(
            &helper,
            "#!/bin/sh\necho $$ > \"$1.tmp\"\nmv \"$1.tmp\" \"$1\"\nwhile [ ! -f \"$1.release\" ]; do sleep 0.01; done\n",
        );

        launch(helper.to_str().unwrap(), &output).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let child_pid = loop {
            if let Ok(pid) = fs::read_to_string(&output)
                && let Ok(pid) = pid.trim().parse::<libc::pid_t>()
            {
                break pid;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "detached helper did not report its process group"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        // SAFETY: the helper remains alive until the release file is written.
        let child_group = unsafe { libc::getpgid(child_pid) };
        assert_ne!(
            child_group, -1,
            "the detached helper still has a process group"
        );
        assert_eq!(child_group, child_pid);
        // SAFETY: `getpgrp` has no preconditions.
        assert_ne!(child_group, unsafe { libc::getpgrp() });
        fs::write(
            crate::test_support::marker_path(&output, std::ffi::OsStr::new(".release")),
            [],
        )
        .unwrap();
        loop {
            // SAFETY: signal zero only probes whether the private process
            // group still exists and cannot change child state.
            if unsafe { libc::kill(-child_group, 0) } == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "detached helper was not reaped"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        fs::remove_dir_all(root).unwrap();
    }
}
