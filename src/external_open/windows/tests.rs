// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::{
    fs,
    os::windows::io::{AsRawHandle, OwnedHandle},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
use windows_sys::Win32::{
    Foundation::{GetHandleInformation, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType},
    System::{
        Com::{APTTYPE_MAINSTA, APTTYPE_STA, CoGetApartmentType},
        Console::{GetConsoleWindow, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE},
        Threading::{
            CreateEventW, OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, SetEvent,
            TerminateProcess, WaitForSingleObject,
        },
    },
};

const FIXTURE: &str = "external_open::windows::tests::native_fixture";

fn until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "opener fixture did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn shell_arguments_flags_apartment_and_optional_process_handle_are_native() {
    std::thread::spawn(|| {
        let target = OsString::from("https://example.test/a?x=$(literal)&y=%PATH%#fragment");
        let mut returned_handle = 0usize;
        shell_using(&target, |info| {
            assert_eq!(info.cbSize, size_of::<SHELLEXECUTEINFOW>() as u32);
            assert_eq!(
                info.fMask,
                SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOCLOSEPROCESS
            );
            assert_eq!(info.nShow, SW_SHOWNORMAL);
            assert!(
                info.hwnd.is_null() && info.lpParameters.is_null() && info.lpDirectory.is_null()
            );
            let mut apartment = 0;
            let mut qualifier = 0;
            assert_eq!(
                unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) },
                0
            );
            assert!(apartment == APTTYPE_STA || apartment == APTTYPE_MAINSTA);
            let decode = |pointer: *const u16| {
                let mut length = 0;
                while unsafe { *pointer.add(length) } != 0 {
                    length += 1;
                }
                String::from_utf16(unsafe { std::slice::from_raw_parts(pointer, length) }).unwrap()
            };
            assert_eq!(decode(info.lpFile), target.to_str().unwrap());
            assert_eq!(decode(info.lpVerb), "open");
            let handle = unsafe { CreateEventW(ptr::null(), 0, 0, ptr::null()) };
            assert!(!handle.is_null());
            returned_handle = handle as usize;
            info.hProcess = handle;
            Ok(())
        })
        .unwrap();
        let mut flags = 0;
        assert_eq!(
            unsafe { GetHandleInformation(returned_handle as HANDLE, &mut flags) },
            0,
            "returned launch handle was retained"
        );
        shell_using(&target, |info| {
            assert!(info.hProcess.is_null());
            Ok(())
        })
        .unwrap();
        assert!(shell_using(&target, |_| Err(io::Error::from_raw_os_error(1155))).is_err());
        assert!(
            shell_using(OsStr::new("bad\0target"), |_| panic!(
                "invalid target reached shell"
            ))
            .is_err()
        );
    })
    .join()
    .unwrap();
}

#[test]
fn stalled_launches_retain_admission_after_ticket_timeout_or_drop() {
    for abandon in [false, true] {
        let active = Arc::new(AtomicUsize::new(0));
        let (release, wait) = mpsc::channel();
        let completed = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&completed);
        let mut ticket = start_using(
            move || {
                wait.recv().unwrap();
                observed.store(true, Ordering::Release);
                Ok(())
            },
            active.clone(),
            |work| std::thread::Builder::new().spawn(work).map(|_| ()),
        )
        .unwrap();
        assert_eq!(active.load(Ordering::Acquire), 1);
        if !abandon {
            let deadline = ticket.deadline;
            assert_eq!(
                ticket
                    .poll(deadline)
                    .unwrap()
                    .unwrap_err()
                    .downcast_ref::<system::Error>(),
                Some(&system::Error::OutcomeUnknown)
            );
        }
        drop(ticket);
        assert_eq!(active.load(Ordering::Acquire), 1);
        assert!(!completed.load(Ordering::Acquire));
        release.send(()).unwrap();
        until(|| active.load(Ordering::Acquire) == 0);
        assert!(completed.load(Ordering::Acquire));
    }
}

#[test]
fn admission_and_thread_failure_never_start_native_work() {
    let active = Arc::new(AtomicUsize::new(16));
    let error = start_using(
        || panic!("busy call executed"),
        active.clone(),
        |_| panic!("busy call started a thread"),
    )
    .err()
    .unwrap();
    assert_eq!(
        error.downcast_ref::<system::Error>(),
        Some(&system::Error::Busy)
    );
    assert_eq!(active.load(Ordering::Acquire), 16);
    active.store(0, Ordering::Release);
    let error = start_using(
        || panic!("failed thread executed"),
        active.clone(),
        |work| {
            drop(work);
            Err(io::Error::other("injected thread creation failure"))
        },
    )
    .err()
    .unwrap();
    assert_eq!(
        error.downcast_ref::<system::Error>(),
        Some(&system::Error::Unavailable)
    );
    assert_eq!(active.load(Ordering::Acquire), 0);
}

#[test]
fn default_paths_preserve_identity_and_refuse_unrepresentable_spelling() {
    let root = crate::test_support::TestRuntimeRoot::new("opener-path").unwrap();
    let path = root.join("caf\u{e9} [a]&.bin");
    fs::write(&path, [0, 255]).unwrap();
    let ordinary = ordinary_target(&path.canonicalize().unwrap()).unwrap();
    assert!(!ordinary.as_os_str().to_string_lossy().starts_with(r"\\?\"));
    assert_eq!(
        ordinary.canonicalize().unwrap(),
        path.canonicalize().unwrap()
    );
    assert!(ordinary_target(root.path()).unwrap().is_dir());
    assert!(ordinary_target(&root.join("missing")).is_err());
    let directory = root.join("x".repeat(110)).join("y".repeat(110));
    fs::create_dir_all(&directory).unwrap();
    let long = directory.join("target.bin");
    fs::write(&long, []).unwrap();
    assert!(
        ordinary_target(&long)
            .unwrap_err()
            .to_string()
            .contains("260")
    );
    assert!(wide(OsStr::new("nul\0path")).is_err());
    assert!(wide(OsStr::new(&"x".repeat(32767))).is_err());
}

#[test]
fn invalid_explicit_programs_and_urls_fail_without_a_shell_handoff() {
    let root = crate::test_support::TestRuntimeRoot::new("opener-refusal").unwrap();
    let target = root.join("file.bin");
    fs::write(&target, [0]).unwrap();
    for program in [
        "runyte_definitely_missing_external_program.exe",
        "runyte_missing.cmd",
    ] {
        let ticket = dispatch(program, &target).unwrap();
        assert!(ticket.wait().is_err());
    }
    assert!(dispatch("bad\0program", &target).is_err());
    for target in [
        "file:///C:/a",
        "https://user:secret@example.test",
        "https://",
        "https://example.test/a b",
    ] {
        assert!(dispatch_browser(target).is_err());
    }
    assert!(explicit(&root.join("absent.exe"), &[]).is_err());
}

struct ChildProcess(OwnedHandle);
impl Drop for ChildProcess {
    fn drop(&mut self) {
        unsafe {
            TerminateProcess(self.0.as_raw_handle(), 1);
            WaitForSingleObject(self.0.as_raw_handle(), 5000);
        }
    }
}

#[test]
fn native_arguments_and_handle_isolation_survive_the_launching_process() {
    let root = crate::test_support::TestRuntimeRoot::new("opener-native").unwrap();
    let target = root.join("caf\u{e9} &[x].bin");
    fs::write(&target, [0, 255]).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            FIXTURE,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
            "--",
            "__opener_launcher",
        ])
        .env("RUNYTE_OPENER_ROOT", root.path())
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("viewer.json")).unwrap()).unwrap();
    let handle = unsafe {
        OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
            0,
            record["pid"].as_u64().unwrap() as u32,
        )
    };
    assert!(!handle.is_null(), "viewer did not survive its launcher");
    let child = ChildProcess(unsafe { OwnedHandle::from_raw_handle(handle) });
    assert_eq!(
        unsafe { WaitForSingleObject(child.0.as_raw_handle(), 0) },
        WAIT_TIMEOUT
    );
    assert_eq!(
        record["args"],
        serde_json::json!([
            "",
            "caf\u{e9} \u{1f600}",
            "a\"b",
            r"C:\folder with spaces\",
            "&|%PATH%$(literal)",
            ordinary_target(&target).unwrap()
        ])
    );
    assert_eq!(record["inherited_event"], false);
    assert_eq!(record["console"], false);
    assert_eq!(record["piped_stdout"], false);
    assert_eq!(record["piped_stderr"], false);
    fs::write(root.join("release"), []).unwrap();
    assert_eq!(
        unsafe { WaitForSingleObject(child.0.as_raw_handle(), 5000) },
        WAIT_OBJECT_0
    );
}

#[test]
#[ignore = "compiled external-opener launcher/viewer fixture"]
fn native_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_OPENER_ROOT").unwrap());
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME").unwrap(),
        root.join("config")
    );
    assert_eq!(
        std::env::var_os("XDG_CACHE_HOME").unwrap(),
        root.join("cache")
    );
    let arguments: Vec<_> = std::env::args().collect();
    if let Some(viewer) = arguments
        .iter()
        .position(|argument| argument == "__opener_viewer")
    {
        let handle: usize = arguments[viewer + 1].parse().unwrap();
        let record = serde_json::json!({
            "pid": std::process::id(),
            "args": &arguments[viewer + 2..],
            "inherited_event": unsafe { SetEvent(handle as HANDLE) } != 0,
            "console": !unsafe { GetConsoleWindow() }.is_null(),
            "piped_stdout": unsafe { GetFileType(GetStdHandle(STD_OUTPUT_HANDLE)) } == FILE_TYPE_PIPE,
            "piped_stderr": unsafe { GetFileType(GetStdHandle(STD_ERROR_HANDLE)) } == FILE_TYPE_PIPE,
        });
        fs::write(
            root.join("viewer.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !root.join("release").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        return;
    }
    assert!(
        arguments
            .iter()
            .any(|argument| argument == "__opener_launcher")
    );
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    let event = unsafe { CreateEventW(&security, 1, 0, ptr::null()) };
    assert!(!event.is_null());
    let event = unsafe { OwnedHandle::from_raw_handle(event) };
    let mut words: Vec<OsString> = [
        "--exact",
        FIXTURE,
        "--ignored",
        "--nocapture",
        "--test-threads=1",
        "--",
        "__opener_viewer",
    ]
    .map(OsString::from)
    .into();
    words.push((event.as_raw_handle() as usize).to_string().into());
    words.extend(
        [
            "",
            "caf\u{e9} \u{1f600}",
            "a\"b",
            r"C:\folder with spaces\",
            "&|%PATH%$(literal)",
        ]
        .map(OsString::from),
    );
    let executable = std::env::current_exe().unwrap();
    let line = crate::windows_process::command_line(
        executable.as_os_str(),
        words.iter().map(OsString::as_os_str),
    )
    .unwrap();
    let program = String::from_utf16(&line[..line.len() - 1]).unwrap();
    dispatch(&program, &root.join("caf\u{e9} &[x].bin"))
        .unwrap()
        .wait()
        .unwrap();
    until(|| {
        fs::read(root.join("viewer.json"))
            .is_ok_and(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).is_ok())
    });
    assert_eq!(
        unsafe { WaitForSingleObject(event.as_raw_handle(), 0) },
        WAIT_TIMEOUT
    );
}
