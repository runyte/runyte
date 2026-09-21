// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(10);
const FIXTURE: &str = "terminal::pty::descriptor_tests::executed_descriptor_probe";

// All Linux masters can have the same stat identity (/dev/ptmx). TIOCGPTN
// distinguishes their peer numbers; st_dev also distinguishes devpts mounts.
fn identity(fd: RawFd) -> Option<String> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } < 0 {
        return None;
    }
    let stat = unsafe { stat.assume_init() };
    if stat.st_mode & libc::S_IFMT != libc::S_IFCHR {
        return None;
    }
    let mut peer: libc::c_uint = 0;
    let peer = if unsafe { libc::ioctl(fd, libc::TIOCGPTN, &mut peer) } == 0 {
        format!("master:{peer}")
    } else {
        "slave".to_owned()
    };
    Some(format!(
        "{}:{}:{}:{peer}",
        stat.st_dev, stat.st_ino, stat.st_rdev
    ))
}

fn packet(stream: &mut UnixStream, bytes: &[u8]) {
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(bytes).unwrap();
}

fn receive(stream: &mut UnixStream) -> Vec<u8> {
    let mut length = [0; 4];
    stream.read_exact(&mut length).unwrap();
    let length = u32::from_be_bytes(length) as usize;
    assert!(length <= 4096, "bounded fixture packet");
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).unwrap();
    bytes
}

fn deadline(stream: &UnixStream) {
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    stream.set_write_timeout(Some(TIMEOUT)).unwrap();
}

struct Probe(Child);
impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "compiled subprocess fixture, invoked by descriptor regression"]
fn executed_descriptor_probe() {
    assert_eq!(
        std::env::var("RUNYTE_PTY_DESCRIPTOR_PROBE").as_deref(),
        Ok("1")
    );
    // Command deliberately maps one CLOEXEC socket endpoint onto fd 0. The
    // fixture owns it after exec; no globally inheritable control fd is used.
    let mut stream = unsafe { UnixStream::from_raw_fd(0) };
    deadline(&stream);
    packet(&mut stream, b"executed");
    let requested = String::from_utf8(receive(&mut stream)).unwrap();
    let identities: Vec<_> = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .map(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_str()
                .unwrap()
                .parse::<RawFd>()
                .unwrap()
        })
        .filter_map(identity)
        .collect();
    let observed: Vec<u8> = requested
        .lines()
        .map(|expected| u8::from(identities.iter().any(|actual| actual == expected)))
        .collect();
    packet(&mut stream, &observed);
    assert_eq!(receive(&mut stream), b"release");
}

// Allocation cannot advance until the executed fixture has acknowledged its
// observation. This forces the launch opportunity without scheduling sleeps.
fn inherited_at_checkpoint(master: RawFd, slave: Option<RawFd>) -> Vec<u8> {
    let requested = [Some(master), slave]
        .into_iter()
        .flatten()
        .map(|fd| identity(fd).expect("PTY identity"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut control, child_control) = UnixStream::pair().unwrap();
    deadline(&control);
    let config = crate::test_support::TestRuntimeRoot::new("pty-probe").unwrap();
    let mut probe = Probe(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
            .env("RUNYTE_PTY_DESCRIPTOR_PROBE", "1")
            .env("XDG_CONFIG_HOME", config.path())
            .stdin(Stdio::from(OwnedFd::from(child_control)))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    assert_eq!(receive(&mut control), b"executed");
    packet(&mut control, requested.as_bytes());
    let observed = receive(&mut control);
    packet(&mut control, b"release");
    let end = Instant::now() + TIMEOUT;
    loop {
        if let Some(status) = probe.0.try_wait().unwrap() {
            assert!(status.success(), "probe status {status}");
            break;
        }
        assert!(Instant::now() < end, "probe did not exit after release");
        thread::sleep(Duration::from_millis(5));
    }
    observed
}

#[test]
fn allocation_endpoints_do_not_survive_unrelated_exec() {
    let mut checkpoints = 0;
    let _pair = open_pair_with_checkpoints(40, 10, |master, slave| {
        let flags: Vec<_> = [Some(master), slave]
            .into_iter()
            .flatten()
            .map(|fd| unsafe { libc::fcntl(fd, libc::F_GETFD) })
            .collect();
        let inherited = inherited_at_checkpoint(master, slave);
        assert_eq!(
            inherited,
            vec![0; flags.len()],
            "PTY endpoints survived exec; F_GETFD={flags:?}"
        );
        assert!(
            flags
                .iter()
                .all(|flag| *flag >= 0 && flag & libc::FD_CLOEXEC != 0)
        );
        checkpoints += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        checkpoints, 2,
        "master-only and paired allocation boundaries"
    );
}

#[test]
fn allocation_failure_closes_every_owned_endpoint() {
    for fail_with_slave in [false, true] {
        let mut allocated = Vec::new();
        let mut witness = None;
        let error = open_pair_with_checkpoints(40, 10, |master, slave| {
            if slave.is_some() == fail_with_slave {
                allocated = [Some(master), slave]
                    .into_iter()
                    .flatten()
                    .map(|fd| (fd, identity(fd).unwrap()))
                    .collect();
                // Keep the PTY identity reserved while checking cleanup. Other
                // concurrently running tests can reuse the descriptor numbers.
                witness = Some(duplicate(master)?);
                return Err(io::Error::from_raw_os_error(libc::EMFILE));
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EMFILE));
        assert_eq!(allocated.len(), if fail_with_slave { 2 } else { 1 });
        for (fd, original) in allocated {
            assert_ne!(
                identity(fd).as_ref(),
                Some(&original),
                "leaked endpoint {fd}"
            );
        }
        drop(witness);
    }
}

#[test]
fn endpoint_identity_distinguishes_simultaneous_ptys() {
    let (first_master, first_slave) = open_pair(40, 10).unwrap();
    let (second_master, second_slave) = open_pair(40, 10).unwrap();
    let identities: Vec<_> = [&first_master, &first_slave, &second_master, &second_slave]
        .into_iter()
        .map(|fd| identity(fd.as_raw_fd()).unwrap())
        .collect();
    for (index, identity) in identities.iter().enumerate() {
        assert!(!identities[..index].contains(identity));
    }
}
