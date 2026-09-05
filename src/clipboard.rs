// SPDX-License-Identifier: MPL-2.0

//! Operating-system clipboard access without a GUI event-loop dependency.
//!
//! Terminal editors commonly run in environments where linking a particular
//! display-server client is either impossible or undesirable. Runyte therefore
//! uses the platform's standard clipboard helper, trying the native choices in
//! deterministic order. Commands are invoked directly, never through a shell.
//!
//! Every helper runs through [`run_helper`], which never blocks the editor on
//! anything but the helper's own exit, and bounds even that. The write helpers
//! on Wayland and X11 — `wl-copy`, `xclip -in`, `xsel --input` — fork a process
//! that owns the selection for as long as the copied value lives, and that
//! process inherits the pipes given to the command Runyte spawned. Reading such
//! a pipe to end of file therefore waits for the clipboard's lifetime rather
//! than the command's: the editor stays frozen until something else takes the
//! selection over. Pipes are consequently drained on their own threads and are
//! only ever waited on with a deadline.

use std::{
    ffi::OsString,
    io::{self, Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[cfg(unix)]
use crate::process_group;

const MAX_HELPER_STDERR_BYTES: usize = 1_024;
const MAX_CLIPBOARD_TEXT_BYTES: usize = 64 * 1024 * 1024;
const MAX_CLIPBOARD_IMAGE_BYTES: usize = 64 * 1024 * 1024;

/// How much of a helper's list of offered clipboard types is read.
///
/// A selection owner declares a handful of MIME types; anything approaching
/// this is a helper that has stopped describing a clipboard.
const MAX_CLIPBOARD_TYPE_LIST_BYTES: usize = 64 * 1024;

/// How long a clipboard helper may take to exit before Runyte kills it.
///
/// The editor calls the clipboard synchronously from its event loop, so this is
/// also the longest a keystroke can stall. It has to clear a cold
/// `powershell.exe` start on Windows with room to spare, and stay short enough
/// that a wedged display server reads as an error rather than a hang.
const HELPER_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the helper that reports what the clipboard holds may take.
///
/// Every press of the paste key pays this, including the presses that only
/// ever wanted text, so it is deliberately shorter than the timeout for work
/// somebody actually asked for. It is long enough for a healthy display server
/// to answer many times over, and short enough that a wedged one costs a pause
/// rather than a hang before the text paste goes ahead.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const IMAGE_PROBE_TIMEOUT: Duration = Duration::from_millis(1_500);

/// How long a helper's pipes are drained for once it has already exited.
const HELPER_PIPE_GRACE: Duration = Duration::from_millis(250);

const HELPER_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Testable clipboard boundary used by the editor.
pub trait SystemClipboard: Send {
    fn read(&mut self) -> Result<String>;
    fn write(&mut self, text: &str) -> Result<()>;

    /// The image the clipboard holds, if it holds one.
    ///
    /// `Ok(None)` is the ordinary answer for a clipboard carrying text, a
    /// file, or nothing at all, and is distinct from an error: a paste key
    /// asks this first and falls back to text, so "no image here" must not
    /// read as "the clipboard is broken". The bytes are handed over exactly as
    /// the platform produced them; nothing here decides what format they are
    /// in.
    ///
    /// Clipboards that cannot produce an image at all default to `None` rather
    /// than to a failure, which is what keeps the inert and in-memory
    /// clipboards used by tests and the headless facade honest without each
    /// of them restating it.
    fn read_image(&mut self) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

/// Clipboard backed by the host platform's standard command-line helpers.
#[derive(Default)]
pub struct CommandClipboard;

impl SystemClipboard for CommandClipboard {
    fn read(&mut self) -> Result<String> {
        read_with_candidates(read_candidates())
    }

    fn write(&mut self, text: &str) -> Result<()> {
        write_with_candidates(write_candidates(), text)
    }

    fn read_image(&mut self) -> Result<Option<Vec<u8>>> {
        read_clipboard_image()
    }
}

fn read_with_candidates(candidates: &[Candidate]) -> Result<String> {
    let mut failures = Vec::new();
    for candidate in candidates {
        match run_helper(
            candidate.program,
            &owned_arguments(candidate.args),
            None,
            Some(MAX_CLIPBOARD_TEXT_BYTES),
            HELPER_TIMEOUT,
        ) {
            Ok(output) if output.success => {
                return String::from_utf8(output.stdout)
                    .context("system clipboard did not contain UTF-8 text");
            }
            Ok(output) => failures.push(describe_failure(candidate.program, &output)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("{}: {error}", candidate.program)),
        }
    }
    unavailable("read", &failures)
}

fn write_with_candidates(candidates: &[Candidate], text: &str) -> Result<()> {
    let mut failures = Vec::new();
    for candidate in candidates {
        // The helper's own stdout is discarded: keeping a pipe it can hand to a
        // forked selection owner would leave a reader thread parked for as long
        // as the clipboard holds this value.
        match run_helper(
            candidate.program,
            &owned_arguments(candidate.args),
            Some(text),
            None,
            HELPER_TIMEOUT,
        ) {
            Ok(output) if output.success => return Ok(()),
            Ok(output) => failures.push(describe_failure(candidate.program, &output)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("{}: {error}", candidate.program)),
        }
    }
    unavailable("write", &failures)
}

fn owned_arguments(arguments: &[&str]) -> Vec<OsString> {
    arguments.iter().map(OsString::from).collect()
}

fn describe_failure(program: &str, output: &BoundedCommandOutput) -> String {
    let stderr = bounded_stderr(&output.stderr);
    if !stderr.is_empty() {
        return format!("{program}: {stderr}");
    }
    match output.code {
        Some(code) => format!("{program}: exited with status {code}"),
        None => format!("{program}: exited unsuccessfully"),
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    program: &'static str,
    args: &'static [&'static str],
}

#[cfg(target_os = "macos")]
fn read_candidates() -> &'static [Candidate] {
    &[Candidate {
        program: "pbpaste",
        args: &[],
    }]
}

#[cfg(target_os = "macos")]
fn write_candidates() -> &'static [Candidate] {
    &[Candidate {
        program: "pbcopy",
        args: &[],
    }]
}

#[cfg(target_os = "windows")]
fn read_candidates() -> &'static [Candidate] {
    &[Candidate {
        program: "powershell.exe",
        args: &["-NoProfile", "-Command", "Get-Clipboard -Raw"],
    }]
}

#[cfg(target_os = "windows")]
fn write_candidates() -> &'static [Candidate] {
    &[Candidate {
        program: "powershell.exe",
        args: &["-NoProfile", "-Command", "$input | Set-Clipboard"],
    }]
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_candidates() -> &'static [Candidate] {
    &[
        Candidate {
            program: "wl-paste",
            args: &["--no-newline"],
        },
        Candidate {
            program: "xclip",
            args: &["-selection", "clipboard", "-out"],
        },
        Candidate {
            program: "xsel",
            args: &["--clipboard", "--output"],
        },
    ]
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn write_candidates() -> &'static [Candidate] {
    &[
        Candidate {
            program: "wl-copy",
            args: &[],
        },
        Candidate {
            program: "xclip",
            args: &["-selection", "clipboard", "-in"],
        },
        Candidate {
            program: "xsel",
            args: &["--clipboard", "--input"],
        },
    ]
}

/// The image types Runyte asks a clipboard for, in the order it prefers them.
///
/// PNG leads because it is lossless and every helper on every platform can
/// produce it; the rest are accepted in whatever form the clipboard already
/// holds so that an image copied out of a browser is stored as it is rather
/// than re-encoded on the way in.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/webp",
    "image/jpeg",
    "image/jpg",
    "image/gif",
    "image/bmp",
];

/// Stands in for the chosen MIME type in a helper's read arguments.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const TYPE_PLACEHOLDER: &str = "{type}";

/// One helper that can list the clipboard's types and hand over one of them.
///
/// Reading an image takes two questions rather than one: a helper asked for a
/// type the clipboard does not hold fails the same way it fails when no
/// display server is running, and a paste key must be able to tell those
/// apart. Listing first makes "there is no image here" an answer rather than
/// an error to guess at.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[derive(Clone, Copy)]
struct ImageCandidate {
    program: &'static str,
    /// Arguments that list the types the clipboard is offered in.
    types: &'static [&'static str],
    /// Arguments that read one type, with [`TYPE_PLACEHOLDER`] where the
    /// chosen MIME type belongs.
    read: &'static [&'static str],
}

/// `xsel` is deliberately absent: it speaks only text, so a clipboard read
/// through it could never answer this question.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn image_candidates() -> &'static [ImageCandidate] {
    &[
        ImageCandidate {
            program: "wl-paste",
            types: &["--list-types"],
            read: &["--no-newline", "--type", TYPE_PLACEHOLDER],
        },
        ImageCandidate {
            program: "xclip",
            types: &["-selection", "clipboard", "-t", "TARGETS", "-o"],
            read: &["-selection", "clipboard", "-t", TYPE_PLACEHOLDER, "-o"],
        },
    ]
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_clipboard_image() -> Result<Option<Vec<u8>>> {
    read_image_with_candidates(image_candidates())
}

/// Reads an image through the first installed helper that answers.
///
/// The first helper to describe the clipboard is also the one trusted about
/// what is in it. Falling through to the next after a successful listing would
/// mean asking a second display server about a selection the first one already
/// answered for, and an empty answer from the working helper is the truth
/// rather than a reason to keep looking.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_image_with_candidates(candidates: &[ImageCandidate]) -> Result<Option<Vec<u8>>> {
    for candidate in candidates {
        // A listing that does not arrive is not an image that is not there,
        // but the difference does not change what this can do about it, and
        // every failure a listing can report is one an ordinary clipboard
        // produces: `wl-paste` exits non-zero on an empty selection, and a
        // display server that is not answering fails the same way for a
        // clipboard holding plain text. Reporting any of them would make the
        // paste key refuse to paste text on a machine where text pastes
        // perfectly well, so the probe stays silent and the text paste that
        // follows reports whatever is really wrong in its own words.
        let Ok(listing) = run_helper(
            candidate.program,
            &owned_arguments(candidate.types),
            None,
            Some(MAX_CLIPBOARD_TYPE_LIST_BYTES),
            IMAGE_PROBE_TIMEOUT,
        ) else {
            continue;
        };
        if !listing.success {
            continue;
        }
        let Some(offered) = preferred_image_type(&listing.stdout) else {
            // The first helper to describe the clipboard is also the one
            // trusted about what is in it: asking the next would be asking a
            // second display server about a selection the first already
            // answered for.
            return Ok(None);
        };
        let arguments = candidate
            .read
            .iter()
            .map(|argument| {
                OsString::from(if *argument == TYPE_PLACEHOLDER {
                    offered
                } else {
                    argument
                })
            })
            .collect::<Vec<_>>();
        // Past this point an image is known to be there, so a failure is worth
        // saying out loud: falling through to a text paste here would silently
        // paste something else instead of the picture that was copied.
        return match run_helper(
            candidate.program,
            &arguments,
            None,
            Some(MAX_CLIPBOARD_IMAGE_BYTES),
            HELPER_TIMEOUT,
        ) {
            Ok(output) if output.success => Ok(Some(output.stdout)),
            Ok(output) => image_unavailable(describe_failure(candidate.program, &output)),
            Err(error) => image_unavailable(format!("{}: {error}", candidate.program)),
        };
    }
    Ok(None)
}

/// Targets that mean the clipboard is carrying text somebody copied as text.
///
/// The X11 spelling of a text selection is a bare atom rather than a MIME
/// type, and both spellings appear on Wayland through compatibility layers, so
/// each one has to be recognised by name.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const TEXT_TYPES: &[&str] = &[
    "text/plain",
    "utf8_string",
    "string",
    "text",
    "compound_text",
];

/// The image type to ask for, given everything the clipboard offers.
///
/// A clipboard that offers text as well as a picture is carrying text. Copying
/// a range of spreadsheet cells or a formatted passage puts a rendered bitmap
/// beside the text precisely so that a "paste as picture" command has
/// something to use, and taking that bitmap would make the ordinary paste key
/// silently unable to paste the text somebody actually selected. Copying a
/// picture is the other case and looks different: an image target with no text
/// beside it, or with `text/html` only, which is markup describing the same
/// picture rather than a thing a person copied as words.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn preferred_image_type(listing: &[u8]) -> Option<&'static str> {
    let listing = String::from_utf8_lossy(listing);
    let offered = listing
        .lines()
        .map(|line| line.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let carries_text = offered.iter().any(|line| {
        // A parameterised `text/plain;charset=utf-8` is the same target as the
        // bare one, so the parameters are cut before comparing.
        let target = line.split(';').next().unwrap_or(line).trim();
        TEXT_TYPES.contains(&target)
    });
    if carries_text {
        return None;
    }
    IMAGE_TYPES
        .iter()
        .copied()
        .find(|wanted| offered.iter().any(|line| line == wanted))
}

/// macOS and Windows hand an image over as a file rather than on a pipe.
///
/// Neither platform has a helper that writes clipboard image bytes to standard
/// output. Both can be asked to save the image, so the editor names a scratch
/// file, reads it back, and removes it; the file is an implementation detail of
/// this boundary and never reaches the workspace.
///
/// The file is named inside a directory created for this one read and removed
/// with it. A predictable name directly in the shared temporary directory
/// would be a name something else can reach first, and the capture scripts
/// open their destination by path and follow symbolic links to it: on a host
/// whose temporary directory is shared, that is somebody else's file being
/// written. Owning the directory removes the race rather than narrowing it.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn read_clipboard_image() -> Result<Option<Vec<u8>>> {
    let directory = private_scratch_directory()?;
    let path = directory.join("clipboard.image");
    let captured = capture_clipboard_image(&path);
    let bytes = match captured {
        Ok(true) => Some(read_bounded_file(&path, MAX_CLIPBOARD_IMAGE_BYTES)),
        Ok(false) => None,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&directory);
            return Err(error);
        }
    };
    let _ = std::fs::remove_dir_all(&directory);
    bytes.transpose().map_err(Into::into)
}

/// A directory only this process can write into, for one clipboard read.
///
/// `create_dir` fails rather than succeeding when the name is taken, so the
/// directory handed back was made here; on Unix it is created 0700 in the same
/// call, leaving no window in which it exists with wider permissions.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn private_scratch_directory() -> Result<std::path::PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut builder = std::fs::DirBuilder::new();
    // Stated rather than left to the default, because the whole point of this
    // directory is that creating it fails when the name is already taken; a
    // recursive create would succeed on a name somebody else owns. It also
    // keeps the binding mutably used on Windows, where the `mode` call below
    // is compiled out.
    builder.recursive(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    let mut failure = None;
    // The counter alone can collide with a directory an earlier run left
    // behind after a crash, which is a name to step past rather than a reason
    // to refuse the paste.
    for _ in 0..16 {
        let path = std::env::temp_dir().join(format!(
            "runyte-clipboard-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(error) => failure = Some(error),
        }
    }
    match failure {
        Some(error) => Err(anyhow::Error::new(error)
            .context("cannot create a private directory for the clipboard image")),
        None => bail!("cannot create a private directory for the clipboard image"),
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn read_bounded_file(path: &std::path::Path, limit: usize) -> io::Result<Vec<u8>> {
    read_bounded(std::fs::File::open(path)?, limit)
}

/// The environment variable a capture script takes its destination from.
///
/// The path is handed over out of band rather than pasted into the script
/// text, so a temporary directory containing a quote or a backslash cannot
/// change what the script does.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const IMAGE_DESTINATION: &str = "RUNYTE_CLIPBOARD_IMAGE";

/// What a capture script prints when the clipboard holds no image.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const NO_IMAGE: &str = "none";

#[cfg(target_os = "macos")]
const MACOS_CAPTURE: &str = r#"
set destination to system attribute "RUNYTE_CLIPBOARD_IMAGE"
try
    set payload to the clipboard as «class PNGf»
on error
    return "none"
end try
set handle to open for access (POSIX file destination) with write permission
try
    set eof handle to 0
    write payload to handle
on error message
    try
        close access handle
    end try
    error message
end try
close access handle
return "image"
"#;

#[cfg(target_os = "windows")]
const WINDOWS_CAPTURE: &str = "\
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$image = [System.Windows.Forms.Clipboard]::GetImage()
if ($null -eq $image) { 'none' } else {
  try {
    $image.Save($env:RUNYTE_CLIPBOARD_IMAGE, [System.Drawing.Imaging.ImageFormat]::Png)
  } finally { $image.Dispose() }
  'image'
}";

/// Asks the platform to save its clipboard image at `path`.
///
/// `Ok(false)` means the clipboard holds no image, which the script reports in
/// its output rather than through an exit status: a script that failed because
/// there was nothing to convert and one that failed because the platform
/// refused would otherwise be indistinguishable.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn capture_clipboard_image(path: &std::path::Path) -> Result<bool> {
    #[cfg(target_os = "macos")]
    let (program, arguments) = (
        "osascript",
        vec![OsString::from("-e"), OsString::from(MACOS_CAPTURE)],
    );
    #[cfg(target_os = "windows")]
    let (program, arguments) = (
        "powershell.exe",
        vec![
            OsString::from("-NoProfile"),
            OsString::from("-Sta"),
            OsString::from("-Command"),
            OsString::from(WINDOWS_CAPTURE),
        ],
    );

    match run_helper_with_environment(
        program,
        &arguments,
        &[(IMAGE_DESTINATION, path.as_os_str().to_owned())],
        None,
        Some(MAX_CLIPBOARD_TYPE_LIST_BYTES),
        HELPER_TIMEOUT,
    ) {
        Ok(output) if output.success => {
            Ok(String::from_utf8_lossy(&output.stdout).trim() != NO_IMAGE)
        }
        Ok(output) => image_unavailable(describe_failure(program, &output)),
        // The platform's own scripting host is missing, or did not answer in
        // time. Text pasting is unaffected either way, so the paste key falls
        // through to it rather than refusing to paste at all over a picture
        // nobody has established is there.
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::TimedOut
            ) =>
        {
            Ok(false)
        }
        Err(error) => image_unavailable(format!("{program}: {error}")),
    }
}

fn image_unavailable<T>(failure: String) -> Result<T> {
    bail!("cannot read an image from the system clipboard: {failure}")
}

fn unavailable<T>(operation: &str, failures: &[String]) -> Result<T> {
    if failures.is_empty() {
        bail!("cannot {operation} the system clipboard: install wl-clipboard, xclip, or xsel");
    }
    bail!(
        "cannot {operation} the system clipboard: {}",
        failures.join("; ")
    )
}

fn bounded_stderr(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.len() <= MAX_HELPER_STDERR_BYTES {
        return text.to_owned();
    }
    let mut end = MAX_HELPER_STDERR_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[derive(Debug)]
struct BoundedCommandOutput {
    success: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Runs one clipboard helper to completion under a deadline.
///
/// Nothing here waits on a pipe that a forked selection owner may still hold:
/// stdin, stdout, and stderr each get their own thread, the helper's exit is
/// polled against the deadline, and a helper that outlives the deadline is
/// killed. `stdout_limit` of `None` discards the helper's output rather than
/// giving it a pipe at all.
fn run_helper(
    program: &str,
    arguments: &[OsString],
    stdin_text: Option<&str>,
    stdout_limit: Option<usize>,
    timeout: Duration,
) -> io::Result<BoundedCommandOutput> {
    run_helper_with_environment(program, arguments, &[], stdin_text, stdout_limit, timeout)
}

/// As [`run_helper`], with variables added to the helper's environment.
///
/// Only the image capture scripts need this: they take their destination path
/// from the environment so that it never passes through script text, where a
/// directory name containing a quote would be read as syntax.
fn run_helper_with_environment(
    program: &str,
    arguments: &[OsString],
    environment: &[(&str, OsString)],
    stdin_text: Option<&str>,
    stdout_limit: Option<usize>,
    timeout: Duration,
) -> io::Result<BoundedCommandOutput> {
    let mut command = Command::new(program);
    for (name, value) in environment {
        command.env(name, value);
    }
    command
        .args(arguments)
        .stdin(if stdin_text.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if stdout_limit.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;
    // Set once the helper's exit has been observed without reaping it, which
    // is what keeps its PID — and the private process group given to it above
    // — reserved to this process for as long as cleanup may need to address
    // them.
    let completed = std::cell::Cell::new(false);
    let outcome = (|| -> io::Result<BoundedCommandOutput> {
        let written = match (stdin_text, child.stdin.take()) {
            (Some(text), Some(mut stdin)) => {
                let text = text.to_owned();
                // The pipe closes when the thread drops it, which is what tells the
                // helper the clipboard value is complete.
                Some(detached(move || stdin.write_all(text.as_bytes())))
            }
            (Some(_), None) => {
                return Err(io::Error::other("clipboard helper stdin is unavailable"));
            }
            (None, _) => None,
        };
        let captured = match (stdout_limit, child.stdout.take()) {
            (Some(limit), Some(stdout)) => Some(detached(move || read_bounded(stdout, limit))),
            (Some(_), None) => {
                return Err(io::Error::other("clipboard helper stdout is unavailable"));
            }
            (None, _) => None,
        };
        let diagnostics = child.stderr.take().map(|stderr| {
            detached(move || read_and_discard_after_limit(stderr, MAX_HELPER_STDERR_BYTES))
        });

        let Some(status) = wait_until(&mut child, deadline, &completed)? else {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{program} did not exit within {} second(s)",
                    timeout.as_secs_f32()
                ),
            ));
        };

        if let Some(written) = written
            && let Err(error) = collect(written, deadline)
            && status.success()
        {
            // A helper that took less than the whole value yet reported success
            // would otherwise leave a silently truncated clipboard behind. When it
            // reported failure, its own diagnostics explain more than this does.
            return Err(error);
        }

        let grace = deadline.max(Instant::now() + HELPER_PIPE_GRACE);
        let stdout = match captured {
            Some(captured) => collect(captured, grace)?.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{program} output was still open after it exited"),
                )
            })?,
            None => Vec::new(),
        };

        // Diagnostics are only worth waiting for when there is a failure to
        // explain, and even then only briefly: on the success path the pipe belongs
        // to a selection owner that outlives this call by design.
        let stderr = if status.success() {
            Vec::new()
        } else {
            diagnostics
                .and_then(|diagnostics| collect(diagnostics, grace).ok().flatten())
                .unwrap_or_default()
        };

        Ok(BoundedCommandOutput {
            success: status.success(),
            code: status.code(),
            stdout,
            stderr,
        })
    })();
    if !matches!(&outcome, Ok(output) if output.success) {
        terminate_helper(&mut child, completed.get());
    } else {
        // A successful helper is left alone — its forked selection owner is
        // supposed to outlive this call — but the leader itself was never
        // reaped, so it is collected here.
        let _ = child.wait();
    }
    outcome
}

#[cfg(unix)]
const CLEANUP: process_group::Site = process_group::Site::new("clipboard", "terminate_helper");

/// Stops an unsuccessful helper and everything it started.
///
/// Helpers get a private process group before spawn, and the group is what
/// cleanup addresses: killing only the direct child leaves descendants that
/// inherited its pipes running and their reader threads parked. That number
/// is only Runyte's for as long as the leader is unreaped, so `completed`
/// says which proof cleanup holds — the leader is either still running, or
/// exited and deliberately not yet collected. If neither holds, no group is
/// addressed at all, because the number may already name a stranger.
fn terminate_helper(child: &mut Child, completed: bool) {
    #[cfg(unix)]
    if completed {
        process_group::claim_anchored_group(
            CLEANUP,
            child.id() as libc::pid_t,
            process_group::GroupAnchor::UnreapedLeader,
        )
        .signal(libc::SIGKILL);
    } else {
        process_group::signal_child_group(CLEANUP, child, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = completed;
        let _ = child.kill();
    }
    let _ = child.wait();
}

/// Runs `work` on its own thread and hands back a receiver for its result.
///
/// The thread is never joined. A pipe held open by a forked selection owner
/// leaves one parked read behind, which ends as soon as that owner releases the
/// clipboard; joining it instead is what froze the editor.
fn detached<T: Send + 'static>(
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> Receiver<io::Result<T>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(work());
    });
    receiver
}

/// `Ok(None)` means the thread was still working when the deadline passed.
fn collect<T>(receiver: Receiver<io::Result<T>>, deadline: Instant) -> io::Result<Option<T>> {
    match receiver.recv_timeout(remaining(deadline)) {
        Ok(Ok(value)) => Ok(Some(value)),
        Ok(Err(error)) => Err(error),
        Err(RecvTimeoutError::Timeout) => Ok(None),
        Err(RecvTimeoutError::Disconnected) => {
            Err(io::Error::other("clipboard helper pipe reader panicked"))
        }
    }
}

/// `Ok(None)` means the helper was still running when the deadline passed.
///
/// Completion is observed without reaping so that the helper's PID, and the
/// process group Runyte created for it, stay reserved until cleanup is done
/// with them. `try_wait` reports the same status but releases both at once,
/// which would leave a later group signal naming whichever process the kernel
/// handed the number to next.
fn wait_until(
    child: &mut Child,
    deadline: Instant,
    completed: &std::cell::Cell<bool>,
) -> io::Result<Option<ExitStatus>> {
    loop {
        if let Some(status) = observe_exit(child)? {
            completed.set(true);
            return Ok(Some(status));
        }
        let left = remaining(deadline);
        if left.is_zero() {
            return Ok(None);
        }
        thread::sleep(HELPER_POLL_INTERVAL.min(left));
    }
}

#[cfg(unix)]
fn observe_exit(child: &mut Child) -> io::Result<Option<ExitStatus>> {
    process_group::completed_without_reaping(child)
}

#[cfg(not(unix))]
fn observe_exit(child: &mut Child) -> io::Result<Option<ExitStatus>> {
    child.try_wait()
}

fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

fn read_bounded(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    // One byte past the limit is enough to detect the overflow, and reading on
    // past it keeps the helper from blocking on a pipe nobody is draining.
    let kept = read_and_discard_after_limit(reader, limit.saturating_add(1))?;
    if kept.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("clipboard helper output exceeds {limit} byte(s)"),
        ));
    }
    Ok(kept)
}

fn read_and_discard_after_limit(mut reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut kept = Vec::with_capacity(limit.min(64 * 1024));
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(kept.len());
        kept.extend_from_slice(&chunk[..read.min(remaining)]);
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_silent_failure_is_reported_with_its_exit_status() {
        let quiet = BoundedCommandOutput {
            success: false,
            code: Some(3),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        assert_eq!(
            describe_failure("wl-copy", &quiet),
            "wl-copy: exited with status 3"
        );

        let noisy = BoundedCommandOutput {
            success: false,
            code: None,
            stdout: Vec::new(),
            stderr: b"  no display  ".to_vec(),
        };
        assert_eq!(describe_failure("wl-copy", &noisy), "wl-copy: no display");
    }

    /// `wl-copy`, `xclip -in`, and `xsel --input` all fork a process that owns
    /// the selection until something else takes the clipboard over, and it
    /// inherits the pipes of the command Runyte spawned. Waiting for those
    /// pipes to reach end of file froze the editor after every yank.
    #[cfg(unix)]
    #[test]
    fn a_forked_selection_owner_does_not_freeze_the_write() {
        static CANDIDATES: &[Candidate] = &[Candidate {
            program: "sh",
            args: &["-c", "cat >/dev/null; sleep 10 & exit 0"],
        }];

        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(write_with_candidates(CANDIDATES, "yanked"));
        });
        let outcome = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the write outlived the helper it spawned");
        assert!(outcome.is_ok(), "{outcome:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_helper_that_never_exits_is_killed_rather_than_waited_on() {
        let started = Instant::now();
        let error = run_helper(
            "sh",
            &owned_arguments(&["-c", "sleep 30"]),
            None,
            Some(64),
            Duration::from_millis(200),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// Cleanup after a helper that is already complete and already reaped
    /// must address no process group at all.
    ///
    /// A negative PID is recycled as soon as the group's leader is reaped, so
    /// a signal sent past that point lands on whichever process the kernel
    /// handed the number to next — successfully, and with the damage showing
    /// up somewhere else entirely.
    #[cfg(unix)]
    #[test]
    fn completed_helper_cleanup_signals_no_recycled_group() {
        let mut helper = Command::new("sh")
            .args(["-c", "exit 1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        helper.wait().unwrap();

        assert!(
            matches!(
                process_group::claim_child_group(CLEANUP, &mut helper),
                process_group::Claim::AlreadyComplete
            ),
            "a reaped helper still claimed ownership of its group"
        );
        // Cleanup has no claim to sign a target with, so it sends nothing.
        terminate_helper(&mut helper, false);
    }

    /// An exited helper is still the owner of its group until it is reaped,
    /// which is what lets cleanup retire the descendants holding its pipes.
    #[cfg(unix)]
    #[test]
    fn a_completed_but_unreaped_helper_still_owns_its_group() {
        let mut helper = Command::new("sh")
            .args(["-c", "sleep 30 & exit 0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = helper.id() as libc::pid_t;
        let completed = std::cell::Cell::new(false);

        let status = loop {
            if let Some(status) =
                wait_until(&mut helper, Instant::now() + HELPER_TIMEOUT, &completed).unwrap()
            {
                break status;
            }
        };
        assert!(status.success());
        assert!(
            completed.get(),
            "the helper's exit was observed through a reap"
        );

        terminate_helper(&mut helper, completed.get());

        // The group emptying is the kernel's to schedule once its members are
        // signalled and orphaned, so it is observed rather than assumed.
        let deadline = Instant::now() + Duration::from_secs(5);
        while group_exists(pid) {
            assert!(
                Instant::now() < deadline,
                "the helper's process group {pid} outlived its termination"
            );
            thread::sleep(HELPER_POLL_INTERVAL);
        }
    }

    #[cfg(unix)]
    fn group_exists(pid: libc::pid_t) -> bool {
        // SAFETY: signal 0 performs the existence and permission checks
        // without delivering anything.
        let result = unsafe { libc::kill(-pid, 0) };
        result == 0 || io::Error::last_os_error().kind() == io::ErrorKind::PermissionDenied
    }

    #[cfg(unix)]
    #[test]
    fn timing_out_a_helper_also_kills_its_descendants() {
        let root = std::env::temp_dir().join(format!(
            "runyte-clipboard-descendants-{}-{:?}",
            std::process::id(),
            Instant::now()
        ));
        fs::create_dir_all(&root).unwrap();
        let marker = root.join("leaked");
        let error = run_helper(
            "sh",
            &[
                "-c".into(),
                "(sleep 0.4; printf leaked > \"$1\") & wait".into(),
                "sh".into(),
                marker.as_os_str().to_owned(),
            ],
            None,
            Some(64),
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        std::thread::sleep(Duration::from_millis(500));
        assert!(!marker.exists(), "a timed-out helper descendant survived");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_successful_parent_with_stuck_output_cleans_up_its_descendant() {
        let root = std::env::temp_dir().join(format!(
            "runyte-clipboard-stuck-output-{}-{:?}",
            std::process::id(),
            Instant::now()
        ));
        fs::create_dir_all(&root).unwrap();
        let marker = root.join("leaked");
        let error = run_helper(
            "sh",
            &[
                "-c".into(),
                "(sleep 0.5; printf leaked > \"$1\") & exit 0".into(),
                "sh".into(),
                marker.as_os_str().to_owned(),
            ],
            None,
            Some(64),
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        std::thread::sleep(Duration::from_millis(600));
        assert!(!marker.exists(), "a stuck output descendant survived");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn helper_input_is_delivered_and_output_is_bounded() {
        let output = run_helper(
            "sh",
            &owned_arguments(&["-c", "cat"]),
            Some("yanked"),
            Some(64),
            HELPER_TIMEOUT,
        )
        .unwrap();
        assert!(output.success);
        assert_eq!(output.stdout, b"yanked");

        let error = run_helper(
            "sh",
            &owned_arguments(&["-c", "printf yanked"]),
            None,
            Some(3),
            HELPER_TIMEOUT,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_helper_is_skipped_and_a_failing_one_is_reported() {
        static CANDIDATES: &[Candidate] = &[
            Candidate {
                program: "runyte-clipboard-helper-that-does-not-exist",
                args: &[],
            },
            Candidate {
                program: "sh",
                args: &["-c", "echo 'no display' >&2; exit 3"],
            },
        ];

        let error = write_with_candidates(CANDIDATES, "yanked")
            .unwrap_err()
            .to_string();
        assert!(error.contains("no display"), "{error}");
        assert!(!error.contains("does-not-exist"), "{error}");
    }

    /// The listing decides which type is asked for, and the type reaches the
    /// read arguments in place of the placeholder.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn an_image_read_asks_for_the_preferred_type_the_clipboard_offers() {
        static CANDIDATES: &[ImageCandidate] = &[
            ImageCandidate {
                program: "runyte-clipboard-helper-that-does-not-exist",
                types: &[],
                read: &[],
            },
            ImageCandidate {
                program: "sh",
                types: &["-c", "printf 'TARGETS\nimage/jpeg\nimage/png\n'"],
                // Echoing the chosen type back is how the substituted
                // argument is observed at all: these are the bytes the
                // clipboard is taken to hold.
                read: &["-c", "printf %s \"$0\"", TYPE_PLACEHOLDER],
            },
        ];

        let image = read_image_with_candidates(CANDIDATES).unwrap().unwrap();
        assert_eq!(
            String::from_utf8(image).unwrap(),
            "image/png",
            "PNG is preferred over the JPEG listed before it"
        );
    }

    /// A clipboard holding text is not a failure to report. The paste key
    /// falls through to a text paste on this answer, so an error here would
    /// turn every ordinary Ctrl-v into a complaint.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_clipboard_offering_no_image_reads_as_no_image() {
        static CANDIDATES: &[ImageCandidate] = &[ImageCandidate {
            program: "sh",
            types: &["-c", "printf 'TARGETS\nUTF8_STRING\ntext/plain\n'"],
            // Reaching this would mean the listing was not believed.
            read: &["-c", "exit 9", TYPE_PLACEHOLDER],
        }];

        assert!(read_image_with_candidates(CANDIDATES).unwrap().is_none());
    }

    /// A machine with no image-capable helper installed can still paste text,
    /// so the absence of one reads as an absent image rather than as a broken
    /// clipboard.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn no_installed_image_helper_reads_as_no_image() {
        static CANDIDATES: &[ImageCandidate] = &[ImageCandidate {
            program: "runyte-clipboard-helper-that-does-not-exist",
            types: &[],
            read: &[],
        }];

        assert!(read_image_with_candidates(CANDIDATES).unwrap().is_none());
    }

    /// Asking what the clipboard holds is a probe every paste pays, including
    /// the ones that only wanted text, so a probe that fails says nothing and
    /// lets the text paste speak for itself. `wl-paste` exits non-zero on an
    /// empty selection, so reporting this would make an ordinary empty
    /// clipboard an error.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_failing_probe_falls_through_to_text_rather_than_refusing() {
        static LISTING_FAILS: &[ImageCandidate] = &[ImageCandidate {
            program: "sh",
            types: &["-c", "echo 'no display' >&2; exit 3"],
            read: &["-c", "exit 9", TYPE_PLACEHOLDER],
        }];
        assert!(read_image_with_candidates(LISTING_FAILS).unwrap().is_none());

        // A probe that hangs is bounded and reads the same way, so a wedged
        // display server costs a pause rather than the paste key itself.
        static LISTING_HANGS: &[ImageCandidate] = &[ImageCandidate {
            program: "sh",
            types: &["-c", "sleep 30"],
            read: &["-c", "exit 9", TYPE_PLACEHOLDER],
        }];
        let started = Instant::now();
        assert!(read_image_with_candidates(LISTING_HANGS).unwrap().is_none());
        assert!(
            started.elapsed() < HELPER_TIMEOUT,
            "the probe outlived its own shorter deadline"
        );
    }

    /// Once the clipboard has said it holds an image, failing to fetch it is
    /// worth saying out loud: pasting text instead would silently substitute
    /// something else for the picture that was copied.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_promised_image_that_cannot_be_read_is_reported() {
        static READ_FAILS: &[ImageCandidate] = &[ImageCandidate {
            program: "sh",
            types: &["-c", "printf 'image/png\n'"],
            read: &[
                "-c",
                "echo 'conversion failed' >&2; exit 4",
                TYPE_PLACEHOLDER,
            ],
        }];
        let error = read_image_with_candidates(READ_FAILS)
            .unwrap_err()
            .to_string();
        assert!(error.contains("conversion failed"), "{error}");
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn the_preferred_image_type_is_chosen_from_what_is_offered() {
        assert_eq!(
            preferred_image_type(b"TARGETS\nimage/gif\nimage/png\n"),
            Some("image/png")
        );
        assert_eq!(
            preferred_image_type(b"TARGETS\nimage/gif\nimage/webp\n"),
            Some("image/webp")
        );
        // Helpers differ in case and in surrounding whitespace.
        assert_eq!(preferred_image_type(b"  IMAGE/PNG  \n"), Some("image/png"));
        assert_eq!(preferred_image_type(b"TARGETS\nUTF8_STRING\n"), None);
        assert_eq!(preferred_image_type(b""), None);
        // A type that merely begins like one of ours is not one of ours.
        assert_eq!(preferred_image_type(b"image/png-sequence\n"), None);
    }

    /// Copying spreadsheet cells or a formatted passage offers a rendered
    /// bitmap beside the text, so that a "paste as picture" command has
    /// something to use. Taking it would leave the ordinary paste key unable
    /// to paste the text somebody actually selected.
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn text_offered_beside_a_picture_means_the_clipboard_carries_text() {
        for listing in [
            &b"TARGETS\ntext/plain\nimage/png\n"[..],
            // The X11 spelling of the same thing.
            &b"TARGETS\nUTF8_STRING\nimage/png\n"[..],
            &b"TARGETS\nSTRING\nimage/bmp\n"[..],
            // Parameters do not make it a different target.
            &b"text/plain;charset=utf-8\nimage/png\n"[..],
        ] {
            assert_eq!(
                preferred_image_type(listing),
                None,
                "{}",
                String::from_utf8_lossy(listing)
            );
        }

        // Copying a picture looks different: no text target beside it, or
        // `text/html` only, which is markup describing that same picture
        // rather than something a person copied as words.
        assert_eq!(
            preferred_image_type(b"TARGETS\nimage/png\n"),
            Some("image/png")
        );
        assert_eq!(
            preferred_image_type(b"TARGETS\ntext/html\nimage/png\n"),
            Some("image/png")
        );
    }

    /// The environment is how a destination path reaches a capture script
    /// without passing through its text.
    #[cfg(unix)]
    #[test]
    fn helper_environment_variables_reach_the_helper() {
        let output = run_helper_with_environment(
            "sh",
            &owned_arguments(&["-c", "printf %s \"$RUNYTE_TEST_VALUE\""]),
            &[("RUNYTE_TEST_VALUE", OsString::from("a \"quoted\" path"))],
            None,
            Some(64),
            HELPER_TIMEOUT,
        )
        .unwrap();
        assert!(output.success);
        assert_eq!(output.stdout, b"a \"quoted\" path");
    }

    #[cfg(unix)]
    #[test]
    fn reading_uses_the_first_helper_that_is_installed() {
        static CANDIDATES: &[Candidate] = &[
            Candidate {
                program: "runyte-clipboard-helper-that-does-not-exist",
                args: &[],
            },
            Candidate {
                program: "sh",
                args: &["-c", "printf 'from the clipboard'"],
            },
        ];

        assert_eq!(
            read_with_candidates(CANDIDATES).unwrap(),
            "from the clipboard"
        );
    }
}
