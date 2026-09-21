// SPDX-License-Identifier: MPL-2.0

//! CF_UNICODETEXT without a shell or code-page conversion. Native delayed
//! rendering can block for thirty seconds; at most one worker may be in the
//! clipboard API, and the editor waits at most one second for its result.

use anyhow::{Context, Result, bail};
use std::{
    io, ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HGLOBAL, HWND},
    System::{DataExchange::*, Memory::*},
    UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE},
};

const UNICODE_TEXT: u32 = 13;
static BUSY: AtomicBool = AtomicBool::new(false);
const DEADLINE: Duration = Duration::from_secs(1);

pub(super) fn read() -> Result<String> {
    run(read_native)
}
pub(super) fn write(text: &str) -> Result<()> {
    let units = encode(text)?;
    run(move || write_native(&units))
}

fn run<T: Send + 'static>(operation: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    if BUSY
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        bail!("Windows clipboard is still busy with the preceding request");
    }
    struct Busy;
    impl Drop for Busy {
        fn drop(&mut self) {
            BUSY.store(false, Ordering::Release);
        }
    }
    let guard = Busy;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("clipboard".into())
        .spawn(move || {
            let result = operation();
            drop(guard);
            let _ = sender.send(result);
        })
        .context("cannot start Windows clipboard worker")?;
    receiver
        .recv_timeout(DEADLINE)
        .context("Windows clipboard timed out; a copy may still finish")?
}

struct Clipboard {
    window: HWND,
}
impl Clipboard {
    fn open() -> Result<Self> {
        // A real owner is required by EmptyClipboard / SetClipboardData. This
        // message-only system-class window never appears on the desktop.
        let class = [83u16, 84, 65, 84, 73, 67, 0];
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if window.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let end = Instant::now() + Duration::from_millis(100);
        loop {
            if unsafe { OpenClipboard(window) } != 0 {
                return Ok(Self { window });
            }
            let error = io::Error::last_os_error();
            if Instant::now() >= end {
                unsafe {
                    DestroyWindow(window);
                }
                return Err(error).context("cannot open Windows clipboard");
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
            DestroyWindow(self.window);
        }
    }
}
struct Allocation(HGLOBAL);
impl Drop for Allocation {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                GlobalFree(self.0);
            }
        }
    }
}
struct Locked(HGLOBAL);
impl Drop for Locked {
    fn drop(&mut self) {
        unsafe {
            GlobalUnlock(self.0);
        }
    }
}

fn encode(text: &str) -> Result<Vec<u16>> {
    if text.len() > super::MAX_CLIPBOARD_TEXT_BYTES {
        bail!("clipboard text exceeds 64 MiB");
    }
    if text.contains('\0') {
        bail!("Windows clipboard text cannot contain a NUL character");
    }
    Ok(text.encode_utf16().chain(Some(0)).collect())
}
fn decode(units: &[u16]) -> Result<String> {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .context("Windows clipboard text has no NUL terminator")?;
    let text =
        String::from_utf16(&units[..end]).context("Windows clipboard contains invalid UTF-16")?;
    if text.len() > super::MAX_CLIPBOARD_TEXT_BYTES {
        bail!("clipboard text exceeds 64 MiB");
    }
    Ok(text)
}
fn read_native() -> Result<String> {
    let _clipboard = Clipboard::open()?;
    if unsafe { IsClipboardFormatAvailable(UNICODE_TEXT) } == 0 {
        bail!("Windows clipboard does not contain text");
    }
    let memory = unsafe { GetClipboardData(UNICODE_TEXT) };
    if memory.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    let bytes = unsafe { GlobalSize(memory) };
    if bytes < 2 || bytes % 2 != 0 || bytes > (super::MAX_CLIPBOARD_TEXT_BYTES + 1) * 2 {
        bail!("Windows clipboard text has an invalid or excessive size");
    }
    let pointer = unsafe { GlobalLock(memory) };
    if pointer.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    let _locked = Locked(memory);
    // The clipboard remains open and the global allocation locked throughout
    // this bounded read. Never take ownership of the clipboard's allocation.
    decode(unsafe { std::slice::from_raw_parts(pointer.cast::<u16>(), bytes / 2) })
}
fn write_native(units: &[u16]) -> Result<()> {
    let mut memory =
        Allocation(unsafe { GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of_val(units)) });
    if memory.0.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    let pointer = unsafe { GlobalLock(memory.0) };
    if pointer.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    {
        let _locked = Locked(memory.0);
        unsafe {
            ptr::copy_nonoverlapping(units.as_ptr(), pointer.cast::<u16>(), units.len());
        }
    }
    // Prepare and validate everything before clearing the existing clipboard.
    let _clipboard = Clipboard::open()?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    if unsafe { SetClipboardData(UNICODE_TEXT, memory.0) }.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    memory.0 = ptr::null_mut(); // Ownership transferred to Windows.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timed_out_work_does_not_accumulate_workers() {
        let (release, wait) = mpsc::channel();
        let result = run(move || {
            wait.recv().unwrap();
            Ok(())
        });
        assert!(result.unwrap_err().to_string().contains("timed out"));
        assert!(
            run(|| Ok(()))
                .unwrap_err()
                .to_string()
                .contains("still busy")
        );
        release.send(()).unwrap();
        let end = Instant::now() + Duration::from_secs(2);
        while BUSY.load(Ordering::Acquire) && Instant::now() < end {
            thread::yield_now();
        }
        assert!(!BUSY.load(Ordering::Acquire));
        assert_eq!(run(|| Ok(42)).unwrap(), 42);
    }
    #[test]
    fn native_clipboard_round_trip() {
        const CHILD: &str = "RUNYTE_ISOLATED_CLIPBOARD_TEST";
        if std::env::var_os(CHILD).is_none() {
            let root = crate::test_support::TestRuntimeRoot::new("clipboard-child").unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "clipboard::windows::tests::native_clipboard_round_trip",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("XDG_CONFIG_HOME", root.path().join("config"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        // A private, noninteractive window station has its own clipboard.
        // Never preserve/replace the person's clipboard as a test fixture.
        use windows_sys::Win32::System::StationsAndDesktops::*;
        unsafe {
            let station = CreateWindowStationW(ptr::null(), 0, 0x37f, ptr::null());
            assert!(!station.is_null(), "{}", io::Error::last_os_error());
            assert_ne!(
                SetProcessWindowStation(station),
                0,
                "{}",
                io::Error::last_os_error()
            );
            let name = [116u16, 101, 115, 116, 0];
            let desktop = CreateDesktopW(
                name.as_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                0x1ff,
                ptr::null(),
            );
            assert!(!desktop.is_null(), "{}", io::Error::last_os_error());
            assert_ne!(
                SetThreadDesktop(desktop),
                0,
                "{}",
                io::Error::last_os_error()
            );
        }
        // Native operations stay on this thread and isolated desktop. The
        // child process owns and releases station/desktop handles on exit.
        for text in ["", "café 中文 😀", "line\n", "line\r\n\r\n"] {
            write_native(&encode(text).unwrap()).unwrap();
            assert_eq!(read_native().unwrap(), text);
        }
    }
    #[test]
    fn unicode_and_line_endings_round_trip_without_normalization() {
        for text in ["", "café 中文 😀", "line\n", "line\r\n\r\n", "\t\r\n"] {
            assert_eq!(decode(&encode(text).unwrap()).unwrap(), text);
        }
    }
    #[test]
    fn malformed_and_unrepresentable_text_is_refused() {
        assert!(encode("before\0after").is_err());
        assert!(decode(&[65]).is_err());
        assert!(decode(&[0xD800, 0]).is_err());
        assert_eq!(decode(&[65, 0, 66]).unwrap(), "A");
    }
}
