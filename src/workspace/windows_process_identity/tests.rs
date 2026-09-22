// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use std::{
    io::{Read, Write},
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
};
use windows_sys::Win32::{
    Foundation::{ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE},
    Security::{CreateWellKnownSid, SECURITY_MAX_SID_SIZE, WinNullSid},
    System::Threading::CREATE_NO_WINDOW,
};

#[test]
fn actual_peer_pin_captures_one_live_process_identity_without_termination_authority() {
    let peer = PinnedProcess::open_peer(std::process::id()).unwrap();
    assert_eq!(peer.identity(), ProcessIdentity::current().unwrap());
    assert!(peer.is_alive().unwrap());
    assert_eq!(
        identity_for_handle(peer.as_handle().as_raw_handle(), std::process::id()).unwrap(),
        peer.identity()
    );
    assert_eq!(
        PinnedProcess::open_peer(0).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn current_identity_round_trips_and_pins_the_same_process() {
    let identity = ProcessIdentity::current().unwrap();
    let encoded = serde_json::to_vec(&identity).unwrap();
    assert_eq!(
        serde_json::from_slice::<ProcessIdentity>(&encoded).unwrap(),
        identity
    );
    assert!(
        serde_json::from_str::<ProcessIdentity>(r#"{"pid":1,"creation_time":2,"unexpected":3}"#)
            .is_err()
    );
    let PinResult::Pinned(process) = PinnedProcess::open(identity).unwrap() else {
        panic!("current process was not pinned");
    };
    assert_eq!(process.identity(), identity);
    assert!(process.is_alive().unwrap());
    assert_eq!(
        identity_for_handle(process.as_handle().as_raw_handle(), identity.pid).unwrap(),
        identity
    );
    let altered = ProcessIdentity {
        creation_time: identity.creation_time ^ 1,
        ..identity
    };
    assert!(matches!(
        PinnedProcess::open(altered).unwrap(),
        PinResult::Reused
    ));
}

#[test]
fn invalid_identity_and_ambiguous_native_errors_never_report_gone() {
    for identity in [
        ProcessIdentity {
            pid: 0,
            creation_time: 1,
        },
        ProcessIdentity {
            pid: 1,
            creation_time: 0,
        },
    ] {
        assert_eq!(
            PinnedProcess::open(identity).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    assert!(matches!(
        classify_open_error(io::Error::from_raw_os_error(ERROR_INVALID_PARAMETER as i32)).unwrap(),
        PinResult::Gone
    ));
    for code in [ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE] {
        assert_eq!(
            classify_open_error(io::Error::from_raw_os_error(code as i32))
                .unwrap_err()
                .raw_os_error(),
            Some(code as i32)
        );
    }
    assert!(
        !classify_wait(WAIT_OBJECT_0, || panic!(
            "successful wait queried last error"
        ))
        .unwrap()
    );
    assert!(
        classify_wait(WAIT_TIMEOUT, || panic!(
            "successful wait queried last error"
        ))
        .unwrap()
    );
    assert_eq!(
        classify_wait(WAIT_FAILED, || io::Error::from_raw_os_error(
            ERROR_ACCESS_DENIED as i32
        ))
        .unwrap_err()
        .raw_os_error(),
        Some(ERROR_ACCESS_DENIED as i32)
    );
    assert!(classify_wait(123, || panic!("unexpected wait queried last error")).is_err());
}

#[test]
fn owner_comparison_accepts_current_user_and_refuses_a_foreign_sid() {
    let current = User::for_process(unsafe { GetCurrentProcess() }).unwrap();
    require_same_sid(current.sid(), current.sid()).unwrap();
    let mut other = vec![0usize; (SECURITY_MAX_SID_SIZE as usize).div_ceil(size_of::<usize>())];
    let mut length = SECURITY_MAX_SID_SIZE;
    assert_ne!(
        unsafe {
            CreateWellKnownSid(
                WinNullSid,
                ptr::null_mut(),
                other.as_mut_ptr().cast(),
                &mut length,
            )
        },
        0
    );
    assert_eq!(
        require_same_sid(current.sid(), other.as_mut_ptr().cast())
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        require_same_sid(current.sid(), ptr::null_mut())
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}

struct FixtureChild(Child);
impl Drop for FixtureChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        unsafe { WaitForSingleObject(self.0.as_raw_handle(), 5000) };
    }
}

#[test]
#[ignore = "compiled child fixture invoked by native process identity tests"]
fn process_fixture() {
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte).unwrap();
    assert_eq!(byte, [b'q']);
}

#[test]
fn pinned_child_handle_observes_exit_without_reopening_its_pid() {
    let root = TestRuntimeRoot::new("workspace-process-identity").unwrap();
    let mut child = FixtureChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "workspace::windows_process_identity::tests::process_fixture",
                "--ignored",
            ])
            .env("XDG_CONFIG_HOME", root.join("config"))
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let identity = identity_for_handle(child.0.as_raw_handle(), child.0.id()).unwrap();
    let PinResult::Pinned(process) = PinnedProcess::open(identity).unwrap() else {
        panic!("live fixture process was not pinned");
    };
    assert!(process.is_alive().unwrap());
    child.0.stdin.take().unwrap().write_all(b"q").unwrap();
    assert_eq!(
        unsafe { WaitForSingleObject(process.as_handle().as_raw_handle(), 10000) },
        WAIT_OBJECT_0,
        "compiled fixture did not exit"
    );
    assert!(child.0.try_wait().unwrap().unwrap().success());
    drop(child);
    assert!(!process.is_alive().unwrap());
    assert!(PinnedProcess::open_peer(identity.pid).is_err());
    assert_eq!(process.identity(), identity);
    assert!(matches!(
        PinnedProcess::open(identity).unwrap(),
        PinResult::Gone
    ));
}
