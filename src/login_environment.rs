// SPDX-License-Identifier: MPL-2.0

//! The environment a login shell gives a program the window server started.
//!
//! A program started from a shell inherits whatever that shell's startup files
//! set. One started by macOS LaunchServices (Finder, the Dock, `open`) inherits
//! launchd's environment instead, which never ran `~/.zprofile` or
//! `/etc/zprofile`, so `PATH` entries and variables set there are missing from
//! the editor, its terminals, its language servers, and Git discovery alike.
//!
//! The macOS application launcher therefore asks the account's shell, started
//! as an interactive login shell, for its environment once, and starts the
//! editor with it. Startup files are arbitrary programs: they can hang, prompt,
//! or print anything. The shell runs with null stdin and stderr in a process
//! group of its own, its output is bounded and read only between two random
//! delimiters, and it is killed at a deadline. Any failure leaves the inherited
//! environment as it was. No value is ever logged.

use std::{
    ffi::{OsStr, OsString},
    io::{self, Read},
    os::unix::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// One environment, in the order a process reports it.
pub type Variables = Vec<(OsString, OsString)>;

/// How long the login shell may take before its environment is abandoned.
///
/// The window does not appear until the capture ends, so this bounds the delay
/// a slow or hanging startup file can add to a launch.
pub const TIMEOUT: Duration = Duration::from_secs(3);

/// The most the shell may write to stdout, startup-file noise included.
///
/// macOS limits arguments and environment together to 1 MiB, so an
/// environment that could be passed on to the editor fits well within this.
pub const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;

/// How long a shell that has reported its environment may take to exit, for
/// example while running `~/.zlogout`, before it is killed.
const EXIT_GRACE: Duration = Duration::from_millis(250);

/// Set to `1` while the login shell is being asked for its environment, so a
/// startup file can skip work that only an interactive session needs.
pub const RESOLVING_VARIABLE: &str = "RUNYTE_RESOLVING_ENVIRONMENT";

/// Why a login shell's environment was not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureError {
    /// The shell's command-line conventions are not known to accept
    /// `-l -i -c`, so it was not started.
    UnsupportedShell,
    /// The shell could not be started.
    Spawn(io::ErrorKind),
    /// The shell did not report an environment before the deadline.
    TimedOut,
    /// The shell wrote more than [`MAX_OUTPUT_BYTES`] before finishing.
    OutputTooLarge,
    /// The shell exited without reporting an environment, for example because
    /// a startup file replaced it with another program or exited early.
    NoEnvironment,
    /// The reported environment was not a sequence of `NAME=value` entries.
    Malformed,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedShell => {
                formatter.write_str("the shell is not a supported login shell")
            }
            Self::Spawn(kind) => write!(formatter, "the shell could not be started ({kind})"),
            Self::TimedOut => write!(
                formatter,
                "the shell did not finish within {} seconds",
                TIMEOUT.as_secs()
            ),
            Self::OutputTooLarge => formatter.write_str("the shell wrote too much output"),
            Self::NoEnvironment => formatter.write_str("the shell exited without an environment"),
            Self::Malformed => formatter.write_str("the shell reported a malformed environment"),
        }
    }
}

/// What preparing a graphical launch's environment established.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The login shell's environment is in use.
    Captured,
    /// No usable shell was found, so the inherited environment is in use.
    NoShell,
    /// The login shell failed, so the inherited environment is in use.
    Unavailable(CaptureError),
}

/// The environment a window-server launch should start the editor with.
///
/// `directory` is where the shell starts, normally the account home. When the
/// shell cannot be asked, `inherited` is returned with only `SHELL` filled in,
/// so a terminal still starts the account's shell rather than `/bin/sh`.
pub fn graphical_environment(inherited: Variables, directory: &Path) -> (Variables, Outcome) {
    let Some(shell) = login_shell(&inherited, crate::user_paths::system_login_shell) else {
        return (inherited, Outcome::NoShell);
    };
    match Capture::new(&shell, directory, &inherited).run() {
        Ok(captured) => (merge(&inherited, captured, &shell), Outcome::Captured),
        Err(error) => {
            let mut environment = inherited;
            name_shell(&mut environment, &shell);
            (environment, Outcome::Unavailable(error))
        }
    }
}

/// The account's shell: `$SHELL` when it names an executable, otherwise the
/// shell recorded for the account.
pub fn login_shell(
    inherited: &[(OsString, OsString)],
    account: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    let usable = |path: &Path| path.is_absolute() && crate::terminal::pty::is_executable(path);
    lookup(inherited, "SHELL")
        .map(PathBuf::from)
        .filter(|path| usable(path))
        .or_else(|| account().filter(|path| usable(path)))
}

/// The arguments that make `shell` an interactive login shell running one
/// command, for the shells known to accept them.
///
/// The C shells accept `-l` only as their sole argument, and an unknown shell
/// may give `-c` another meaning, so neither is started.
fn login_arguments(shell: &Path) -> Option<[&'static str; 3]> {
    let name = shell.file_name()?.to_str()?;
    matches!(
        name,
        "sh" | "bash" | "zsh" | "ksh" | "ksh93" | "mksh" | "oksh" | "dash" | "yash" | "fish"
    )
    .then_some(["-l", "-i", "-c"])
}

/// One bounded request for a login shell's environment.
pub struct Capture<'a> {
    shell: &'a Path,
    directory: &'a Path,
    inherited: &'a [(OsString, OsString)],
    timeout: Duration,
    limit: usize,
}

impl<'a> Capture<'a> {
    pub fn new(
        shell: &'a Path,
        directory: &'a Path,
        inherited: &'a [(OsString, OsString)],
    ) -> Self {
        Self {
            shell,
            directory,
            inherited,
            timeout: TIMEOUT,
            limit: MAX_OUTPUT_BYTES,
        }
    }

    /// The same request with a shorter deadline and output bound, for tests.
    pub fn bounded(mut self, timeout: Duration, limit: usize) -> Self {
        self.timeout = timeout;
        self.limit = limit;
        self
    }

    /// Starts the shell and returns the environment it reports.
    ///
    /// The shell starts with exactly `inherited` plus [`RESOLVING_VARIABLE`],
    /// never this process's own environment.
    pub fn run(&self) -> Result<Variables, CaptureError> {
        let arguments = login_arguments(self.shell).ok_or(CaptureError::UnsupportedShell)?;
        let delimiter = delimiter();
        // Absolute programs, so neither an alias nor the profile's PATH can
        // stand in for them.
        let script = format!(
            "/usr/bin/printf %s {delimiter}; /usr/bin/env -0; /usr/bin/printf %s {delimiter}"
        );
        let deadline = Instant::now() + self.timeout;
        let mut child = Command::new(self.shell)
            .args(arguments)
            .arg(script)
            .env_clear()
            .envs(self.inherited.iter().map(|(name, value)| (name, value)))
            .env(RESOLVING_VARIABLE, "1")
            .current_dir(self.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|error| CaptureError::Spawn(error.kind()))?;
        crate::process_group::record_spawn("login_environment", "login shell", child.id());
        let stdout = child.stdout.take().expect("piped stdout");
        let (sender, receiver) = mpsc::channel();
        let limit = self.limit;
        let reader_delimiter = delimiter.clone();
        // The reader stops at the closing delimiter rather than at end of
        // file: a daemon a startup file left behind may hold stdout open.
        let reader = std::thread::Builder::new()
            .name("runyte-login-environment".into())
            .spawn(move || {
                let _ = sender.send(read_delimited(stdout, reader_delimiter.as_bytes(), limit));
            });
        if reader.is_err() {
            finish(&mut child, Instant::now());
            return Err(CaptureError::Spawn(io::ErrorKind::Other));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let result = match receiver.recv_timeout(remaining) {
            Ok(Ok(output)) => parse(&output, delimiter.as_bytes()),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(CaptureError::TimedOut),
        };
        // A shell that reported its environment normally exits at once. One
        // still running after a short grace, or one that failed, is killed
        // with the rest of its group: its environment is already in hand.
        finish(
            &mut child,
            if result.is_ok() {
                deadline.min(Instant::now() + EXIT_GRACE)
            } else {
                Instant::now()
            },
        );
        result
    }
}

/// Waits for the shell until `deadline`, then kills its group and reaps it.
fn finish(child: &mut Child, deadline: Instant) {
    const SITE: crate::process_group::Site =
        crate::process_group::Site::new("login_environment", "finish");
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            _ => break,
        }
    }
    crate::process_group::signal_child_group(SITE, child, libc::SIGKILL);
    let _ = child.wait();
}

/// A token the shell's startup files cannot know in advance.
fn delimiter() -> String {
    use std::hash::{BuildHasher, RandomState};
    let state = RandomState::new();
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let first = state.hash_one((std::process::id(), time));
    let second = RandomState::new().hash_one(first);
    format!("RUNYTE-ENVIRONMENT-{first:016x}{second:016x}")
}

/// Reads until two delimiters have been seen, end of file, or `limit` bytes.
///
/// Each read is searched only where a delimiter could newly have completed,
/// so noisy startup files cost time linear in what they print.
fn read_delimited(
    mut source: impl Read,
    delimiter: &[u8],
    limit: usize,
) -> Result<Vec<u8>, CaptureError> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 16 * 1024];
    let mut opened = false;
    let mut searched = 0;
    loop {
        let read = match source.read(&mut chunk) {
            Ok(0) => return Ok(output),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Ok(output),
        };
        if output.len() + read > limit {
            return Err(CaptureError::OutputTooLarge);
        }
        output.extend_from_slice(&chunk[..read]);
        while let Some(at) = find(&output[searched..], delimiter) {
            if opened {
                return Ok(output);
            }
            opened = true;
            searched += at + delimiter.len();
        }
        searched = searched.max((output.len() + 1).saturating_sub(delimiter.len()));
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The bytes between the first two occurrences of `delimiter`.
fn delimited<'a>(output: &'a [u8], delimiter: &[u8]) -> Option<&'a [u8]> {
    let start = find(output, delimiter)? + delimiter.len();
    let end = start + find(&output[start..], delimiter)?;
    Some(&output[start..end])
}

/// The `NAME=value` entries `env -0` wrote between the delimiters.
fn parse(output: &[u8], delimiter: &[u8]) -> Result<Variables, CaptureError> {
    let body = delimited(output, delimiter).ok_or(CaptureError::NoEnvironment)?;
    let mut variables = Variables::new();
    for entry in body
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let separator = entry
            .iter()
            .position(|byte| *byte == b'=')
            .filter(|separator| *separator > 0)
            .ok_or(CaptureError::Malformed)?;
        variables.push((
            OsStr::from_bytes(&entry[..separator]).to_owned(),
            OsStr::from_bytes(&entry[separator + 1..]).to_owned(),
        ));
    }
    if variables.is_empty() {
        return Err(CaptureError::Malformed);
    }
    Ok(variables)
}

/// Whether a variable keeps the value this process inherited, set or unset,
/// whatever the login shell reported.
///
/// The editor sets the terminal variables itself, and `XDG_RUNTIME_DIR`
/// selects where a persistent session host's endpoint is found. The rest
/// describe the shell process that reported them, not the editor.
fn keeps_inherited(name: &OsStr) -> bool {
    matches!(
        name.as_bytes(),
        b"TERM" | b"COLORTERM" | b"XDG_RUNTIME_DIR" | b"PWD" | b"OLDPWD" | b"SHLVL" | b"_"
    ) || name == RESOLVING_VARIABLE
}

/// `RUNYTE_*` variables carry the launch's own state, so an inherited one is
/// never replaced. One that only a startup file sets is kept, as it would be
/// for an editor started from that shell.
fn is_runyte(name: &OsStr) -> bool {
    name.as_bytes().starts_with(b"RUNYTE_")
}

/// The captured environment, with the variables the editor manages taken from
/// `inherited` instead, and `SHELL` naming `shell` when nothing else does.
pub fn merge(inherited: &[(OsString, OsString)], captured: Variables, shell: &Path) -> Variables {
    let inherited_wins = |name: &OsStr| {
        keeps_inherited(name) || (is_runyte(name) && lookup_os(inherited, name).is_some())
    };
    let mut environment = captured
        .into_iter()
        .filter(|(name, _)| !inherited_wins(name))
        .collect::<Variables>();
    environment.extend(
        inherited
            .iter()
            .filter(|(name, _)| keeps_inherited(name) || is_runyte(name))
            .cloned(),
    );
    name_shell(&mut environment, shell);
    environment
}

fn name_shell(environment: &mut Variables, shell: &Path) {
    if lookup(environment, "SHELL").is_none_or(OsStr::is_empty) {
        environment.retain(|(name, _)| name != "SHELL");
        environment.push(("SHELL".into(), shell.as_os_str().to_owned()));
    }
}

fn lookup<'a>(environment: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsStr> {
    lookup_os(environment, OsStr::new(name))
}

fn lookup_os<'a>(environment: &'a [(OsString, OsString)], name: &OsStr) -> Option<&'a OsStr> {
    environment
        .iter()
        .rev()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_os_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestRuntimeRoot, user_environment};

    /// Links the checked-in stand-in under `name`, sourcing `behavior`.
    fn stand_in(root: &Path, name: &str, behavior: &str) -> PathBuf {
        let directory = root.join("bin");
        std::fs::create_dir_all(&directory).unwrap();
        let link = directory.join(name);
        std::os::unix::fs::symlink(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
            &link,
        )
        .unwrap();
        std::fs::write(directory.join(format!("{name}.behavior")), behavior).unwrap();
        link
    }

    fn fixture_environment(root: &Path) -> Variables {
        let mut environment = user_environment(root)
            .into_iter()
            .map(|(name, value)| (OsString::from(name), value.into_os_string()))
            .collect::<Variables>();
        std::fs::create_dir_all(root.join("home")).unwrap();
        environment.push(("PATH".into(), "/usr/bin:/bin".into()));
        environment
    }

    fn value<'a>(environment: &'a Variables, name: &str) -> Option<&'a str> {
        lookup(environment, name).map(|value| value.to_str().unwrap())
    }

    /// Runs the request's command line as a login shell would, after
    /// startup-file noise on both sides of it.
    const LOGIN_SHELL: &str = r#"
[ "$1 $2 $3" = "-l -i -c" ] || exit 9
[ "$RUNYTE_RESOLVING_ENVIRONMENT" = 1 ] || exit 8
printf 'Last login: noise %s\n' RUNYTE-ENVIRONMENT-
export RUNYTE_LOGIN_PROBE=1 PROFILE_PATH_ADDED=yes
export MULTILINE='first
second=still value'
export TERM=dumb SHLVL=7 RUNYTE_STARTUP_FIXTURE_MODE=profile
PATH="/profile/bin:$PATH"; export PATH
eval "$4"
printf 'zlogout noise\n'
"#;

    #[test]
    fn login_shell_environment_is_read_between_delimiters_and_merged() {
        let root = TestRuntimeRoot::new("login-env-capture").unwrap();
        let shell = stand_in(&root, "zsh", LOGIN_SHELL);
        let mut inherited = fixture_environment(&root);
        inherited.push(("TERM".into(), "xterm-kitty".into()));
        inherited.push(("RUNYTE_STARTUP_FIXTURE_MODE".into(), "kept".into()));
        let captured = Capture::new(&shell, &root.join("home"), &inherited)
            .run()
            .unwrap();
        assert_eq!(value(&captured, "RUNYTE_LOGIN_PROBE"), Some("1"));
        assert_eq!(value(&captured, RESOLVING_VARIABLE), Some("1"));
        assert_eq!(
            value(&captured, "MULTILINE"),
            Some("first\nsecond=still value")
        );
        assert_eq!(value(&captured, "PATH"), Some("/profile/bin:/usr/bin:/bin"));
        // The shell started in the requested directory with the given
        // environment, not this test process's own.
        assert_eq!(
            value(&captured, "PWD").map(Path::new),
            Some(root.join("home").canonicalize().unwrap().as_path())
        );
        assert_eq!(
            value(&captured, "HOME").map(Path::new),
            Some(root.join("home").as_path())
        );

        let merged = merge(&inherited, captured, &shell);
        assert_eq!(value(&merged, "RUNYTE_LOGIN_PROBE"), Some("1"));
        assert_eq!(value(&merged, "PATH"), Some("/profile/bin:/usr/bin:/bin"));
        assert_eq!(value(&merged, "TERM"), Some("xterm-kitty"));
        assert_eq!(value(&merged, "RUNYTE_STARTUP_FIXTURE_MODE"), Some("kept"));
        assert_eq!(
            value(&merged, "XDG_RUNTIME_DIR").map(Path::new),
            Some(root.join("runtime").as_path())
        );
        for unset in [RESOLVING_VARIABLE, "SHLVL", "PWD", "_", "OLDPWD"] {
            assert_eq!(value(&merged, unset), None, "{unset}");
        }
        assert_eq!(
            value(&merged, "SHELL").map(Path::new),
            Some(shell.as_path())
        );
    }

    #[test]
    fn graphical_environment_uses_the_inherited_shell_and_falls_back_intact() {
        let root = TestRuntimeRoot::new("login-env-graphical").unwrap();
        let shell = stand_in(&root, "bash", LOGIN_SHELL);
        let mut inherited = fixture_environment(&root);
        inherited.push(("SHELL".into(), shell.clone().into_os_string()));
        let (environment, outcome) = graphical_environment(inherited.clone(), &root.join("home"));
        assert_eq!(outcome, Outcome::Captured);
        assert_eq!(value(&environment, "PROFILE_PATH_ADDED"), Some("yes"));

        let failing = stand_in(&root, "dash", "exit 0\n");
        let mut inherited = fixture_environment(&root);
        inherited.push(("SHELL".into(), failing.clone().into_os_string()));
        let (environment, outcome) = graphical_environment(inherited.clone(), &root.join("home"));
        assert_eq!(outcome, Outcome::Unavailable(CaptureError::NoEnvironment));
        assert_eq!(environment, inherited);
    }

    #[test]
    fn a_hanging_startup_file_is_killed_at_the_deadline_with_its_group() {
        let root = TestRuntimeRoot::new("login-env-hang").unwrap();
        let marker = root.join("background.pid");
        let shell = stand_in(
            &root,
            "zsh",
            &format!("sleep 30 &\necho $! > '{}'\nwait\n", marker.display()),
        );
        let inherited = fixture_environment(&root);
        let started = Instant::now();
        let result = Capture::new(&shell, &root.join("home"), &inherited)
            .bounded(Duration::from_millis(300), MAX_OUTPUT_BYTES)
            .run();
        assert_eq!(result, Err(CaptureError::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(10));
        let background: libc::pid_t = std::fs::read_to_string(&marker)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // The group's background member was killed, not left running. Once
        // its parent was reaped it is reparented and reaped in turn.
        let gone = Instant::now() + Duration::from_secs(10);
        // SAFETY: signal 0 only probes whether the process exists.
        while unsafe { libc::kill(background, 0) } == 0 {
            assert!(Instant::now() < gone, "background member still running");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_shell_that_lingers_after_reporting_still_contributes_its_environment() {
        let root = TestRuntimeRoot::new("login-env-linger").unwrap();
        let shell = stand_in(
            &root,
            "zsh",
            "export RUNYTE_LOGIN_PROBE=1\neval \"$4\"\nsleep 30\n",
        );
        let inherited = fixture_environment(&root);
        let started = Instant::now();
        let captured = Capture::new(&shell, &root.join("home"), &inherited)
            .bounded(Duration::from_millis(300), MAX_OUTPUT_BYTES)
            .run()
            .unwrap();
        assert_eq!(value(&captured, "RUNYTE_LOGIN_PROBE"), Some("1"));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn unbounded_noise_missing_output_and_unknown_shells_are_refused() {
        let root = TestRuntimeRoot::new("login-env-refusals").unwrap();
        let inherited = fixture_environment(&root);
        let home = root.join("home");
        let noisy = stand_in(
            &root,
            "zsh",
            "while :; do printf '%s\\n' 0123456789abcdef0123456789abcdef; done\n",
        );
        assert_eq!(
            Capture::new(&noisy, &home, &inherited)
                .bounded(Duration::from_secs(10), 4096)
                .run(),
            Err(CaptureError::OutputTooLarge)
        );
        let replaced = stand_in(&root, "fish", "printf 'tmux: not a terminal\\n'\nexit 1\n");
        assert_eq!(
            Capture::new(&replaced, &home, &inherited).run(),
            Err(CaptureError::NoEnvironment)
        );
        let csh = stand_in(&root, "tcsh", "touch started\n");
        assert_eq!(
            Capture::new(&csh, &home, &inherited).run(),
            Err(CaptureError::UnsupportedShell)
        );
        assert!(!home.join("started").exists());
        assert_eq!(
            Capture::new(&root.join("bin/missing"), &home, &inherited).run(),
            Err(CaptureError::UnsupportedShell)
        );
        assert!(matches!(
            Capture::new(&root.join("bin/sh"), &home, &inherited).run(),
            Err(CaptureError::Spawn(_))
        ));
    }

    #[test]
    fn delimited_output_must_hold_name_value_entries() {
        let delimiter = b"MARK";
        assert_eq!(
            parse(b"noise MARKA=1\0B=x=y\0MARK after", delimiter).unwrap(),
            vec![("A".into(), "1".into()), ("B".into(), "x=y".into())]
        );
        assert_eq!(
            parse(b"MARKA=1\0", delimiter),
            Err(CaptureError::NoEnvironment)
        );
        assert_eq!(
            parse(b"no marks", delimiter),
            Err(CaptureError::NoEnvironment)
        );
        assert_eq!(parse(b"MARKMARK", delimiter), Err(CaptureError::Malformed));
        assert_eq!(
            parse(b"MARK=1\0MARK", delimiter),
            Err(CaptureError::Malformed)
        );
        assert_eq!(
            parse(b"MARKA\0MARK", delimiter),
            Err(CaptureError::Malformed)
        );
        assert_ne!(delimiter_pair().0, delimiter_pair().1);
        assert_eq!(
            read_delimited(&b"xMARKA=1\0MARKtrailing"[..], delimiter, 64).unwrap(),
            b"xMARKA=1\0MARKtrailing"
        );
        // Delimiters split across reads are still found, and reading stops
        // at the closing one rather than waiting for end of file.
        let pieces = [&b"noiseMA"[..], b"RKA=1\0MA", b"R", b"K", b"never read"];
        let split = pieces
            .iter()
            .fold(Box::new(io::empty()) as Box<dyn Read>, |source, piece| {
                Box::new(source.chain(*piece))
            });
        assert_eq!(
            read_delimited(OneByteAtATime(split), delimiter, 64).unwrap(),
            b"noiseMARKA=1\0MARK"
        );
        assert_eq!(
            read_delimited(&[b'x'; 65][..], delimiter, 64),
            Err(CaptureError::OutputTooLarge)
        );
    }

    /// A source that hands out one byte per read, as a slow pipe might.
    struct OneByteAtATime<R>(R);

    impl<R: Read> Read for OneByteAtATime<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let end = buffer.len().min(1);
            self.0.read(&mut buffer[..end])
        }
    }

    fn delimiter_pair() -> (String, String) {
        (delimiter(), delimiter())
    }

    #[test]
    fn the_shell_comes_from_shell_then_the_account_record() {
        let root = TestRuntimeRoot::new("login-env-shell").unwrap();
        let zsh = stand_in(&root, "zsh", "");
        let account = stand_in(&root, "bash", "");
        let with = |shell: &OsStr| vec![(OsString::from("SHELL"), shell.to_owned())];
        assert_eq!(
            login_shell(&with(zsh.as_os_str()), || panic!("account not needed")),
            Some(zsh.clone())
        );
        for unusable in [
            OsStr::new(""),
            OsStr::new("zsh"),
            root.join("bin/none").as_os_str(),
        ] {
            assert_eq!(
                login_shell(&with(unusable), || Some(account.clone())),
                Some(account.clone())
            );
        }
        assert_eq!(
            login_shell(&[], || Some(account.clone())),
            Some(account.clone())
        );
        assert_eq!(
            login_shell(&[], || Some(root.join("bin/zsh.behavior"))),
            None
        );
        assert_eq!(login_shell(&[], || None), None);

        let mut environment = vec![(OsString::from("SHELL"), OsString::new())];
        name_shell(&mut environment, &zsh);
        assert_eq!(environment, with(zsh.as_os_str()));
        name_shell(&mut environment, &account);
        assert_eq!(environment, with(zsh.as_os_str()));
    }

    /// The issue's reproduction with the real shells, confined to a fixture
    /// home: a variable exported only from a login startup file arrives.
    #[test]
    fn real_login_shells_read_their_login_only_startup_files() {
        let root = TestRuntimeRoot::new("login-env-real").unwrap();
        let home = root.join("home");
        for (shell, startup) in [("/bin/zsh", ".zprofile"), ("/bin/bash", ".bash_profile")] {
            let shell = Path::new(shell);
            if !shell.exists() {
                continue;
            }
            let mut inherited = fixture_environment(&root);
            inherited.push(("ZDOTDIR".into(), home.clone().into_os_string()));
            std::fs::write(home.join(startup), "export RUNYTE_LOGIN_PROBE=1\n").unwrap();
            assert_eq!(value(&inherited, "RUNYTE_LOGIN_PROBE"), None);
            let captured = Capture::new(shell, &home, &inherited).run().unwrap();
            let merged = merge(&inherited, captured, shell);
            assert_eq!(
                value(&merged, "RUNYTE_LOGIN_PROBE"),
                Some("1"),
                "{}",
                shell.display()
            );
            assert_eq!(value(&merged, "SHELL").map(Path::new), Some(shell));
        }
    }
}
