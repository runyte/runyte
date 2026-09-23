// SPDX-License-Identifier: MPL-2.0

//! Native text and bounded image reads without a shell. Native delayed
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
const DIB: u32 = 8;
const DIB_V5: u32 = 17;
static BUSY: AtomicBool = AtomicBool::new(false);
const DEADLINE: Duration = Duration::from_secs(1);
mod image;

#[cfg(test)]
static TEST_DESKTOP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(super) fn read() -> Result<String> {
    run(read_native)
}
pub(super) fn write(text: &str) -> Result<()> {
    let units = encode(text)?;
    run(move || write_native(&units))
}
pub(super) fn read_image() -> Result<Option<Vec<u8>>> {
    run(read_image_native)
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
            #[cfg(test)]
            let result = prepare_test_worker().and_then(|()| operation());
            #[cfg(not(test))]
            let result = operation();
            drop(guard);
            let _ = sender.send(result);
        })
        .context("cannot start Windows clipboard worker")?;
    receiver
        .recv_timeout(DEADLINE)
        .context("Windows clipboard timed out; a copy may still finish")?
}

#[cfg(test)]
fn prepare_test_worker() -> Result<()> {
    // Native fixtures use a private station. Attach the worker before its
    // first HWND is created; changing only the fixture thread is insufficient.
    let desktop = TEST_DESKTOP.load(Ordering::Acquire);
    if desktop != 0
        && unsafe {
            windows_sys::Win32::System::StationsAndDesktops::SetThreadDesktop(desktop as _)
        } == 0
    {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
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

fn png_format() -> Result<u32> {
    let name = [80u16, 78, 71, 0];
    let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    if format == 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(format)
}

fn read_image_native() -> Result<Option<Vec<u8>>> {
    let (format, bytes) = {
        let _clipboard = Clipboard::open()?;
        // Windows advertises synthesized Unicode text for CF_TEXT/OEMTEXT.
        // Rich-text applications also offer rendered bitmaps; ordinary paste
        // must retain their text instead of silently pasting that rendering.
        if unsafe { IsClipboardFormatAvailable(UNICODE_TEXT) } != 0 {
            return Ok(None);
        }
        let png = png_format()?;
        let Some(format) = [png, DIB_V5, DIB]
            .into_iter()
            .find(|format| unsafe { IsClipboardFormatAvailable(*format) } != 0)
        else {
            return Ok(None);
        };
        let memory = unsafe { GetClipboardData(format) };
        if memory.is_null() {
            return Err(io::Error::last_os_error()).context("cannot read clipboard image");
        }
        let length = unsafe { GlobalSize(memory) };
        if length == 0 || length > crate::pasted_image::MAX_IMAGE_BYTES {
            bail!("Windows clipboard image has an invalid size or exceeds 64 MiB");
        }
        let pointer = unsafe { GlobalLock(memory) };
        if pointer.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let _locked = Locked(memory);
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length)?;
        // Borrow only while the clipboard is open and its allocation locked.
        bytes
            .extend_from_slice(unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) });
        (format, bytes)
    };
    // Encoding never holds the system clipboard open. Its allocations and
    // CPU work remain in the same single, deadline-bounded worker.
    if format == DIB || format == DIB_V5 {
        image::dib_to_png(&bytes, format == DIB_V5).map(Some)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok(Some(bytes))
    } else {
        bail!("Windows clipboard PNG data has an invalid signature")
    }
}

#[cfg(test)]
fn test_station_name(station: windows_sys::Win32::System::StationsAndDesktops::HWINSTA) -> String {
    use windows_sys::Win32::System::StationsAndDesktops::{GetUserObjectInformationW, UOI_NAME};
    let mut name = [0u16; 128];
    let mut needed = 0;
    assert_ne!(
        unsafe {
            GetUserObjectInformationW(
                station,
                UOI_NAME,
                name.as_mut_ptr().cast(),
                std::mem::size_of_val(&name) as u32,
                &mut needed,
            )
        },
        0,
        "{}",
        io::Error::last_os_error()
    );
    let end = name.iter().position(|unit| *unit == 0).unwrap();
    String::from_utf16(&name[..end]).unwrap()
}

#[cfg(test)]
fn isolate_test_desktop() -> String {
    // The compiled child owns these handles until process exit. Its private
    // station must also be distinct from other fixture children: a NULL name
    // reopens the logon session's shared station, not a fresh unnamed object.
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };
    use windows_sys::Win32::System::StationsAndDesktops::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CWF_CREATE_ONLY, WINSTA_ACCESSCLIPBOARD, WINSTA_ACCESSGLOBALATOMS, WINSTA_CREATEDESKTOP,
        WINSTA_READATTRIBUTES,
    };
    let mut nonce = [0u8; 16];
    let status = unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            nonce.as_mut_ptr(),
            nonce.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    assert!(
        status >= 0,
        "clipboard fixture randomness failed: {status:#x}"
    );
    let nonce = nonce
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let station_name = format!("runyte-clipboard-test-{}-{nonce}", std::process::id());
    let wide: Vec<_> = station_name.encode_utf16().chain(Some(0)).collect();
    let station = crate::windows_fs::with_private_security(|security| {
        let access = (WINSTA_ACCESSCLIPBOARD
            | WINSTA_ACCESSGLOBALATOMS
            | WINSTA_CREATEDESKTOP
            | WINSTA_READATTRIBUTES) as u32;
        let station =
            unsafe { CreateWindowStationW(wide.as_ptr(), CWF_CREATE_ONLY, access, security) };
        if station.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(station)
        }
    })
    .expect(
        "cannot create a distinct private clipboard fixture station; no shared-station fallback",
    );
    assert_eq!(test_station_name(station), station_name);
    unsafe {
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
        TEST_DESKTOP.store(desktop as usize, Ordering::Release);
        assert_eq!(test_station_name(GetProcessWindowStation()), station_name);
    }
    station_name
}

#[cfg(test)]
#[path = "windows/image_tests.rs"]
mod image_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "required Windows CI acceptance; creating private window stations requires administrator privileges"]
    fn concurrent_fixture_children_keep_distinct_clipboards() {
        use std::{fs, path::Path, process::Command};
        const ROLE: &str = "RUNYTE_CLIPBOARD_ISOLATION_ROLE";
        const ROOT: &str = "RUNYTE_CLIPBOARD_ISOLATION_ROOT";

        fn wait_for(path: &Path) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !path.exists() {
                assert!(
                    Instant::now() < deadline,
                    "fixture barrier timed out: {path:?}"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }

        if let Some(role) = std::env::var_os(ROLE) {
            let role = role.to_str().unwrap();
            assert!(matches!(role, "first" | "second"));
            let root = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
            let name = isolate_test_desktop();
            fs::write(root.join(format!("{role}.station")), name).unwrap();
            if role == "second" {
                wait_for(&root.join("first.published"));
            }
            write_native(&encode(role).unwrap()).unwrap();
            fs::write(root.join(format!("{role}.published")), []).unwrap();
            if role == "first" {
                wait_for(&root.join("second.published"));
            }
            // The second child has now replaced its clipboard. With a shared
            // station, the first child deterministically reads "second" here.
            assert_eq!(read_native().unwrap(), role);
            return;
        }

        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let root = crate::test_support::TestRuntimeRoot::new("clipboard-isolation").unwrap();
        let children = ["first", "second"].map(|role| {
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "clipboard::windows::tests::concurrent_fixture_children_keep_distinct_clipboards",
                    "--ignored",
                    "--nocapture",
                ])
                .env(ROLE, role)
                .env(ROOT, root.path())
                .env("XDG_CONFIG_HOME", root.path().join(role).join("config"))
                .env("XDG_CACHE_HOME", root.path().join(role).join("cache"))
                .stdout(fs::File::create(root.path().join(format!("{role}.stdout"))).unwrap())
                .stderr(fs::File::create(root.path().join(format!("{role}.stderr"))).unwrap())
                .spawn()
                .unwrap();
            (role, Child(child))
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        for (role, mut child) in children {
            let status = loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    break status;
                }
                assert!(
                    Instant::now() < deadline,
                    "clipboard fixture child timed out: {role}"
                );
                thread::sleep(Duration::from_millis(5));
            };
            assert!(
                status.success(),
                "{role}: {}\n{}",
                fs::read_to_string(root.path().join(format!("{role}.stdout"))).unwrap(),
                fs::read_to_string(root.path().join(format!("{role}.stderr"))).unwrap()
            );
        }
        let first = fs::read_to_string(root.path().join("first.station")).unwrap();
        let second = fs::read_to_string(root.path().join("second.station")).unwrap();
        assert_ne!(first, second, "fixture children share a window station");
    }

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
    #[ignore = "required Windows CI acceptance; creating private window stations requires administrator privileges"]
    fn native_clipboard_round_trip() {
        const CHILD: &str = "RUNYTE_ISOLATED_CLIPBOARD_TEST";
        if std::env::var_os(CHILD).is_none() {
            let root = crate::test_support::TestRuntimeRoot::new("clipboard-child").unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "clipboard::windows::tests::native_clipboard_round_trip",
                    "--ignored",
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
        isolate_test_desktop();
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
