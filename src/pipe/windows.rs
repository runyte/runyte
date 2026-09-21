// SPDX-License-Identifier: MPL-2.0

//! Native PowerShell filters using owned jobs and overlapped pipe I/O.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{ffi::OsString, io, os::windows::ffi::OsStringExt, path::PathBuf, process::Command};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::NamedPipeServer,
};

const COMMAND_ENV: &str = "RUNYTE_INTERNAL_FILTER_COMMAND";
// Only this fixed bootstrap is placed on the Windows command line. The full
// authored command can occupy its existing 16 KiB budget without base64's
// UTF-16 expansion exceeding CreateProcess's command-line limit.
const BOOTSTRAP: &str = r#"
$ErrorActionPreference = 'Stop'
try {
    $utf8 = New-Object System.Text.UTF8Encoding($false, $true)
    [Console]::InputEncoding = $utf8
    [Console]::OutputEncoding = $utf8
    $OutputEncoding = $utf8
    $command = [Environment]::GetEnvironmentVariable('RUNYTE_INTERNAL_FILTER_COMMAND')
    [Environment]::SetEnvironmentVariable('RUNYTE_INTERNAL_FILTER_COMMAND', $null)
    $global:LASTEXITCODE = 0
    & ([ScriptBlock]::Create($command))
    $success = $?
    $nativeExit = $LASTEXITCODE
    if (-not $success) { exit 1 }
    if ($nativeExit -ne 0) { exit $nativeExit }
    exit 0
} catch {
    [Console]::Error.WriteLine($_.ToString())
    exit 1
}
"#;

fn shell() -> io::Result<PathBuf> {
    let mut units = vec![0u16; 32768];
    let length = unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
            units.as_mut_ptr(),
            units.len() as u32,
        )
    } as usize;
    if length == 0 {
        return Err(io::Error::last_os_error());
    }
    if length >= units.len() {
        return Err(io::Error::other("system directory is too long"));
    }
    Ok(PathBuf::from(OsString::from_wide(&units[..length]))
        .join("WindowsPowerShell/v1.0/powershell.exe"))
}

fn command(text: &str, directory: &Path) -> Result<Command> {
    ensure!(text.len() <= 16 * 1024, "pipe command exceeds 16 KiB");
    ensure!(!text.contains('\0'), "pipe command contains NUL");
    let encoded = STANDARD.encode(
        BOOTSTRAP
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut command = Command::new(shell()?);
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-InputFormat",
            "Text",
            "-OutputFormat",
            "Text",
            "-EncodedCommand",
            &encoded,
        ])
        .env(COMMAND_ENV, text)
        .current_dir(directory);
    Ok(command)
}

pub(super) fn run(
    text: &str,
    directory: &Path,
    inputs: Vec<String>,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Vec<String>> {
    let command = command(text, directory)?;
    run_command(&command, inputs, cancel, timeout)
}

fn run_command(
    command: &Command,
    inputs: Vec<String>,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Vec<String>> {
    let deadline = Instant::now() + timeout;
    // run() already belongs to the editor's background pipe worker. One runtime
    // owns every selection's overlapped I/O; no detached/blocking pipe workers.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let mut remaining = MAX_BYTES;
            let mut outputs = Vec::with_capacity(inputs.len());
            for input in inputs {
                let output = invoke(command, input.as_bytes(), cancel, deadline, remaining).await?;
                remaining -= output.len();
                outputs.push(
                    String::from_utf8(output)
                        .map_err(|_| anyhow::anyhow!("pipe stdout is not valid UTF-8"))?,
                );
            }
            Ok(outputs)
        })
}

async fn read(pipe: &mut NamedPipeServer, buffer: &mut [u8]) -> io::Result<usize> {
    match pipe.read(buffer).await {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(0),
        result => result,
    }
}

async fn invoke(
    command: &Command,
    input: &[u8],
    cancel: &AtomicBool,
    deadline: Instant,
    limit: usize,
) -> Result<Vec<u8>> {
    ensure!(!cancel.load(Ordering::Acquire), "pipe cancelled");
    ensure!(Instant::now() < deadline, "pipe timed out");
    let (stdin, child_stdin) = crate::windows_process::overlapped::pipe(false)?;
    let (mut stdout, child_stdout) = crate::windows_process::overlapped::pipe(true)?;
    let (mut stderr, child_stderr) = crate::windows_process::overlapped::pipe(true)?;
    let mut child = crate::windows_process::spawn_with_stdio(
        command,
        [child_stdin, child_stdout, child_stderr],
    )?;
    let mut stdin = Some(stdin);
    let mut errors = Vec::new();
    let mut truncated = false;
    let result = async {
        let mut output = Vec::new();
        let mut written = 0;
        let mut out_done = false;
        let mut err_done = false;
        let mut exited = None;
        let mut out_buffer = [0u8; 8192];
        let mut err_buffer = [0u8; 8192];
        loop {
            ensure!(!cancel.load(Ordering::Acquire), "pipe cancelled");
            ensure!(Instant::now() < deadline, "pipe timed out");
            if exited.is_none() {
                exited = child.try_wait()?; // Also terminates inherited-pipe descendants.
            }
            if written == input.len() || exited.is_some() { stdin = None; }
            if let Some(status) = exited && out_done && err_done {
                ensure!(status.success(), "pipe exited with {status}");
                return Ok(output);
            }
            tokio::select! {
                result = async { stdin.as_mut().unwrap().write(&input[written..input.len().min(written + 8192)]).await }, if stdin.is_some() => {
                    match result {
                        Ok(0) => bail!("pipe stdin stopped accepting input"),
                        Ok(n) => written += n,
                        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => stdin = None,
                        Err(error) => return Err(error.into()),
                    }
                }
                result = read(&mut stdout, &mut out_buffer), if !out_done => {
                    let n = result?;
                    out_done = n == 0;
                    ensure!(n <= limit.saturating_sub(output.len()), "pipe stdout exceeds the 8 MiB job limit");
                    output.extend_from_slice(&out_buffer[..n]);
                }
                result = read(&mut stderr, &mut err_buffer), if !err_done => {
                    let n = result?;
                    err_done = n == 0;
                    let take = n.min(STDERR_BYTES.saturating_sub(errors.len()));
                    errors.extend_from_slice(&err_buffer[..take]);
                    truncated |= take < n;
                }
                _ = tokio::time::sleep(Duration::from_millis(5)) => {}
            }
        }
    }.await;
    // Every return, cancellation and timeout drops the job and all overlapped
    // handles in this worker. No runtime shutdown depends on synchronous I/O.
    let _ = child.kill();
    result.map_err(|error: anyhow::Error| {
        if errors.is_empty() {
            error
        } else {
            anyhow::anyhow!(
                "{error}: {}{}",
                String::from_utf8_lossy(&errors),
                if truncated { " [stderr truncated]" } else { "" }
            )
        }
    })
}

#[cfg(test)]
mod tests;
