// SPDX-License-Identifier: MPL-2.0

//! Authorized desktop handoffs have an independent lifetime. Unlike LSP,
//! filters and integrated terminals, their processes never enter an owned job.

use super::{LaunchTicket, system};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::{OsStr, OsString},
    io,
    mem::{size_of, zeroed},
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::{Component, Path, PathBuf, Prefix},
    ptr,
    sync::{Arc, atomic::AtomicUsize},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    System::{
        Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        Threading::{CREATE_NO_WINDOW, CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW},
    },
    UI::{
        Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            ShellExecuteExW,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

const STARTUP_BUDGET: Duration = Duration::from_secs(5);
type Work = Box<dyn FnOnce() + Send>;

pub(super) fn dispatch(program: &str, path: &Path) -> Result<LaunchTicket> {
    let program = program.trim();
    if program.is_empty() {
        return dispatch_target(path.as_os_str().to_owned(), false);
    }
    let words =
        crate::terminal::split_command(program).context("invalid native program command")?;
    ensure!(
        !words.is_empty() && !words[0].is_empty(),
        "no program was given"
    );
    let target = path.to_owned();
    start(move || {
        let executable = crate::windows_executable::resolve(
            Path::new(&words[0]),
            std::env::var_os("PATH").as_deref(),
            std::env::var_os("PATHEXT").as_deref(),
        )
        .context(
            "native external program was not found; choose an installed .exe or .com program",
        )?;
        let target = target
            .canonicalize()
            .context("cannot resolve external file")?;
        let mut arguments: Vec<OsString> = words[1..].iter().map(OsString::from).collect();
        // Native programs receive a path as one final argument, never shell
        // source. Keep extended paths available for programs that support them.
        let target = match ordinary_target(&target) {
            Ok(ordinary) => ordinary,
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|error| error.kind() == io::ErrorKind::Unsupported) =>
            {
                target
            }
            Err(error) => return Err(error),
        };
        arguments.push(target.into_os_string());
        explicit(&executable, &arguments)
    })
}

pub(super) fn dispatch_browser(url: &str) -> Result<LaunchTicket> {
    system::validate_url(url)?;
    dispatch_target(OsString::from(url), true)
}

pub(super) fn dispatch_target(target: OsString, is_url: bool) -> Result<LaunchTicket> {
    start(move || {
        let argument = if is_url {
            system::validate_url(target.to_str().ok_or(system::Error::InvalidTarget)?)?;
            target
        } else {
            ordinary_target(Path::new(&target))?.into_os_string()
        };
        shell(&argument)
    })
}

fn start(operation: impl FnOnce() -> Result<()> + Send + 'static) -> Result<LaunchTicket> {
    start_using(operation, system::active_openers(), |work| {
        std::thread::Builder::new()
            .name("system-opener".into())
            .spawn(work)
            .map(|_| ())
    })
}

fn start_using(
    operation: impl FnOnce() -> Result<()> + Send + 'static,
    active: Arc<AtomicUsize>,
    start_thread: impl FnOnce(Work) -> io::Result<()>,
) -> Result<LaunchTicket> {
    let slot = system::Slot::reserve(active)?;
    let (sender, ticket) = LaunchTicket::channel(Instant::now() + STARTUP_BUDGET);
    start_thread(Box::new(move || {
        let result = operation();
        // A successful handoff needs no per-viewer waiting thread on Windows.
        // This slot bounds actual outstanding API calls, even if their ticket
        // was dropped or timed out while a shell extension was still working.
        drop(slot);
        let _ = sender.send(result);
    }))
    .map_err(|error| anyhow::Error::new(system::Error::Unavailable).context(error))?;
    Ok(ticket)
}

/// Shell handlers do not uniformly support extended path syntax. Offer only
/// an identity-equivalent ordinary local/UNC spelling below MAX_PATH.
fn ordinary_target(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .context("cannot resolve external target")?;
    let mut parts = canonical.components();
    let mut ordinary = match parts.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(drive) | Prefix::Disk(drive) => {
                PathBuf::from(format!("{}:", drive as char))
            }
            Prefix::VerbatimUNC(server, share) | Prefix::UNC(server, share) => {
                let mut value = OsString::from(r"\\");
                value.push(server);
                value.push(r"\");
                value.push(share);
                PathBuf::from(value)
            }
            _ => anyhow::bail!("system opening requires an ordinary Windows path"),
        },
        _ => anyhow::bail!("system opening requires an absolute Windows path"),
    };
    for part in parts {
        if let Component::Normal(name) = part {
            crate::windows_fs::validate_relative(Path::new(name))?;
        }
        ordinary.push(part.as_os_str());
    }
    if ordinary.as_os_str().encode_wide().count() >= 260 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "system opening requires an ordinary Windows path shorter than 260 UTF-16 units",
        )
        .into());
    }
    ensure!(
        crate::windows_fs::Identity::read(&ordinary)?
            == crate::windows_fs::Identity::read(&canonical)?,
        "external target changed while resolving its Windows spelling"
    );
    Ok(ordinary)
}

fn wide(value: &OsStr) -> Result<Vec<u16>> {
    let mut units: Vec<_> = value.encode_wide().collect();
    ensure!(
        !units.contains(&0) && units.len() < 32767,
        "external argument contains NUL or exceeds the Windows limit"
    );
    units.push(0);
    Ok(units)
}

struct Apartment;
impl Apartment {
    fn enter() -> Result<Self> {
        // Each operation owns a new thread, so a conflicting apartment is a
        // failure rather than a reason to run shell extensions in the MTA.
        let result = unsafe {
            CoInitializeEx(
                ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        };
        ensure!(
            result >= 0,
            "cannot initialize the Windows shell apartment ({result:#x})"
        );
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn shell(argument: &OsStr) -> Result<()> {
    shell_using(argument, |info| {
        if unsafe { ShellExecuteExW(info) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    })
}

fn shell_using(
    argument: &OsStr,
    execute: impl FnOnce(&mut SHELLEXECUTEINFOW) -> io::Result<()>,
) -> Result<()> {
    let _apartment = Apartment::enter()?;
    let target = wide(argument)?;
    let verb = [b'o' as u16, b'p' as u16, b'e' as u16, b'n' as u16, 0];
    let mut info: SHELLEXECUTEINFOW = unsafe { zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    // NOASYNC is required for a worker without a message loop. NO_UI suppresses
    // ordinary error dialogs, not Windows security prompts. Do not request
    // environment substitution, console inheritance, elevation or zone bypass.
    info.fMask = SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = target.as_ptr();
    info.nShow = SW_SHOWNORMAL;
    execute(&mut info).context("system application handoff failed")?;
    if !info.hProcess.is_null() {
        // The returned handle is ours; the launched application is not.
        drop(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(info.hProcess) });
    }
    // Existing application instances/DDE can accept a handoff with no handle.
    Ok(())
}

fn explicit(executable: &Path, arguments: &[OsString]) -> Result<()> {
    let application = wide(executable.as_os_str())?;
    let mut line = crate::windows_process::command_line(
        executable.as_os_str(),
        arguments.iter().map(OsString::as_os_str),
    )?;
    let mut startup: STARTUPINFOW = unsafe { zeroed() };
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
    // No job assignment and no inherited handles, console or standard streams.
    // CREATE_NO_WINDOW affects console executables without hiding GUI windows.
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_NO_WINDOW,
            ptr::null(),
            ptr::null(),
            &startup,
            &mut process,
        )
    } == 0
    {
        return Err(io::Error::last_os_error()).context("native external program could not start");
    }
    drop(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(process.hThread) });
    drop(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(process.hProcess) });
    Ok(())
}

#[cfg(test)]
mod tests;
