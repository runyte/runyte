// SPDX-License-Identifier: MPL-2.0
#![cfg(windows)]

use runyte::{
    buffer::{Buffer, SaveOutcome},
    test_support::TestRuntimeRoot,
    text::Transaction,
};
use std::{
    fs,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const FIXTURE: &str = "concurrent_pathname_reads_fixture";

#[test]
fn successful_saves_have_complete_contents_after_acknowledgement() {
    let root = TestRuntimeRoot::new("save-visibility").unwrap();
    let log = fs::File::create(root.join("reads.log")).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
        .env("RUNYTE_SAVE_VISIBILITY_ROOT", root.path())
        .env("XDG_CONFIG_HOME", root.join("config"))
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let report = fs::read_to_string(root.join("reads.log")).unwrap();
            println!("{report}");
            assert!(status.success(), "save visibility fixture failed: {status}");
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("save visibility fixture exceeded 30 seconds");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "reexecuted with private storage and a process deadline"]
fn concurrent_pathname_reads_fixture() {
    let root = std::path::PathBuf::from(std::env::var_os("RUNYTE_SAVE_VISIBILITY_ROOT").unwrap());
    let file = root.join("document.txt");
    let versions = [
        "old complete contents\r\n".repeat(1024),
        "new complete contents\n".repeat(1024),
    ];
    fs::write(&file, &versions[0]).unwrap();
    let mut buffer = Buffer::open(&file).unwrap();
    let stop = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_secs(20);
    let (ready, started) = mpsc::sync_channel(0);
    thread::scope(|scope| {
        let reader = scope.spawn(|| {
            let mut complete = 0_u64;
            let mut errors = std::collections::BTreeMap::<i32, u64>::new();
            ready.send(()).unwrap();
            while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
                match fs::read(&file) {
                    Ok(bytes) => {
                        assert!(
                            versions.iter().any(|v| v.as_bytes() == bytes),
                            "reader saw partial or unknown contents"
                        );
                        complete += 1;
                    }
                    Err(error) => {
                        let code = error.raw_os_error().expect("native read error");
                        // Characterize transient missing names, denied opens,
                        // sharing and lock violations; never accept these after save.
                        assert!(
                            matches!(code, 2 | 5 | 32 | 33),
                            "unexpected pathname read: {error}"
                        );
                        *errors.entry(code).or_default() += 1;
                    }
                }
            }
            (complete, errors)
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        for index in 0..128 {
            assert!(
                Instant::now() < deadline,
                "save characterization exceeded its budget"
            );
            let expected = &versions[(index + 1) % 2];
            assert!(buffer.apply(&Transaction::change(
                0,
                buffer.len_chars(),
                expected.clone()
            )));
            assert_eq!(buffer.save(false).unwrap(), SaveOutcome::Durable);
            assert!(!buffer.dirty);
            assert_eq!(
                fs::read(&file).unwrap(),
                expected.as_bytes(),
                "completed save {index}"
            );
        }
        stop.store(true, Ordering::Release);
        let (complete, errors) = reader.join().unwrap();
        assert!(complete > 0, "reader did not sample complete contents");
        println!(
            "128 durable saves; concurrent complete reads={complete}; native open errors={errors:?}; all 128 post-completion reads exact"
        );
    });
}
