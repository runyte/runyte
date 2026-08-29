// SPDX-License-Identifier: MPL-2.0

//! The one place Runyte signals a process group it created.
//!
//! A negative PID names a process group, and the kernel recycles that number
//! as soon as the group's leader is reaped. Every subsystem that spawns a
//! child into its own group — Git commands, clipboard helpers, terminal
//! sessions — therefore has the same way to go wrong: reap the direct child,
//! then signal `-pid` and hit whatever process happens to hold that number
//! next. The signal is delivered successfully and nothing local looks amiss,
//! so the damage surfaces as an unrelated process dying by a signal it never
//! should have received.
//!
//! Ownership of a numeric group identity is proven in exactly two ways here,
//! and there is no third:
//!
//! - [`claim_child_group`] probes the direct child first. A child that is
//!   still running anchors the number, so the group may be signalled. A child
//!   that has already been reaped does not, so no claim is issued.
//! - [`claim_anchored_group`] is for a caller that has deliberately observed
//!   its child's exit *without* reaping it, keeping the identity anchored on
//!   purpose. That caller states the anchor rather than re-probing, because
//!   probing would reap the leader and destroy the very thing it relies on.
//!
//! An `Err` from the probe proves nothing either way, so it issues no claim.
//! A claim covers a whole teardown sequence: nothing between its signals
//! reaps the leader, so one proof still holds for the last of them.
//!
//! # Auditing
//!
//! Cross-process signal defects are invisible from inside the process that
//! suffers them: the victim only sees its own death. The audit journal exists
//! so a diagnostic run can name the sender. It is written only when
//! [`AUDIT_PATH_VARIABLE`] is set in the environment, records at most
//! [`MAX_AUDIT_BYTES`] per process, appends one bounded line per event, and
//! never influences whether a signal is sent. Normal application use sets no
//! such variable and writes nothing.

use std::{
    io::Write,
    path::PathBuf,
    process::Child,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Environment variable naming the file a diagnostic run appends its audit to.
pub const AUDIT_PATH_VARIABLE: &str = "RUNYTE_PROCESS_AUDIT";

/// How much one process may append before it stops recording.
///
/// The journal is a diagnostic aid, not a log: a run that would otherwise fill
/// a disk must lose records instead.
pub const MAX_AUDIT_BYTES: usize = 512 * 1024;

/// Where a signal is being sent from, in terms a reader can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Site {
    /// The owning subsystem, matching the logging subsystem names.
    pub subsystem: &'static str,
    /// The function or path within that subsystem.
    pub call_site: &'static str,
}

impl Site {
    pub const fn new(subsystem: &'static str, call_site: &'static str) -> Self {
        Self {
            subsystem,
            call_site,
        }
    }
}

/// What a caller relies on to keep a numeric group identity theirs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupAnchor {
    /// The direct child is still running.
    RunningLeader,
    /// The direct child has exited and has deliberately not been reaped, so
    /// its PID and process group are still reserved to this process.
    UnreapedLeader,
}

impl GroupAnchor {
    fn as_str(self) -> &'static str {
        match self {
            Self::RunningLeader => "running_leader",
            Self::UnreapedLeader => "unreaped_leader",
        }
    }
}

/// The result of asking for a process group to be signalled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupSignalOutcome {
    /// Ownership was proven and the signal was sent.
    Sent,
    /// The direct child was already complete, so no group was addressed.
    AlreadyComplete,
    /// The child's state could not be established, so nothing was sent.
    OwnershipUnproven,
}

/// A process group whose numeric identity is proven to still be Runyte's.
///
/// Holding one is the only way to address a group here. It borrows nothing, so
/// a caller may keep it across the reap that ends the ownership it represents;
/// what it cannot do is come into existence without a proof.
#[derive(Clone, Copy, Debug)]
pub struct OwnedGroup {
    site: Site,
    pid: libc::pid_t,
    anchor: GroupAnchor,
}

impl OwnedGroup {
    /// The group's identifier, which is its leader's PID.
    pub fn leader(&self) -> libc::pid_t {
        self.pid
    }

    /// Sends one signal to every member of the group.
    pub fn signal(&self, signal: libc::c_int) {
        self.signal_with(signal, deliver);
    }

    /// [`OwnedGroup::signal`] with the delivery step supplied, for tests.
    pub fn signal_with(
        &self,
        signal: libc::c_int,
        mut deliver: impl FnMut(libc::pid_t, libc::c_int),
    ) {
        record(Event::Signal {
            site: self.site,
            signal,
            child_pid: self.pid.max(0) as u32,
            anchor: Some(self.anchor),
            outcome: GroupSignalOutcome::Sent,
        });
        deliver(-self.pid, signal);
    }
}

/// What asking for ownership of a child's process group established.
#[derive(Debug)]
pub enum Claim {
    /// The number still names the group Runyte created.
    Owned(OwnedGroup),
    /// The direct child was already complete, so the number is recyclable.
    AlreadyComplete,
    /// The child's state could not be established.
    Unproven,
}

/// Claims the private process group anchored by a still-running `child`.
///
/// The probe reaps a child that has exited, which is what makes the answer
/// authoritative: after `Ok(Some(_))` the number is no longer Runyte's to
/// address, and after `Ok(None)` the live child still holds it. One claim
/// covers every signal in a teardown sequence, because nothing between them
/// reaps the leader. Callers that must keep an *exited* child unreaped use
/// [`claim_anchored_group`] instead.
pub fn claim_child_group(site: Site, child: &mut Child) -> Claim {
    let pid = child.id();
    match child.try_wait() {
        Ok(Some(_)) => {
            record(Event::Signal {
                site,
                signal: 0,
                child_pid: pid,
                anchor: None,
                outcome: GroupSignalOutcome::AlreadyComplete,
            });
            Claim::AlreadyComplete
        }
        Ok(None) => Claim::Owned(OwnedGroup {
            site,
            pid: pid as libc::pid_t,
            anchor: GroupAnchor::RunningLeader,
        }),
        Err(error) => {
            crate::log_warn!(
                site.subsystem,
                "could not prove process-group ownership before signalling";
                "call_site" => site.call_site,
                "pid" => pid,
                "error" => error,
            );
            record(Event::Signal {
                site,
                signal: 0,
                child_pid: pid,
                anchor: None,
                outcome: GroupSignalOutcome::OwnershipUnproven,
            });
            Claim::Unproven
        }
    }
}

/// Claims a process group whose leader is deliberately still unreaped.
///
/// The caller states why the number is still theirs. Nothing is probed,
/// because probing an exited child reaps it and releases exactly the identity
/// this claim depends on.
pub fn claim_anchored_group(site: Site, pid: libc::pid_t, anchor: GroupAnchor) -> OwnedGroup {
    OwnedGroup { site, pid, anchor }
}

/// Signals a child's private process group when it is still owned.
pub fn signal_child_group(
    site: Site,
    child: &mut Child,
    signal: libc::c_int,
) -> GroupSignalOutcome {
    match claim_child_group(site, child) {
        Claim::Owned(group) => {
            group.signal(signal);
            GroupSignalOutcome::Sent
        }
        Claim::AlreadyComplete => GroupSignalOutcome::AlreadyComplete,
        Claim::Unproven => GroupSignalOutcome::OwnershipUnproven,
    }
}

fn deliver(target: libc::pid_t, signal: libc::c_int) {
    // SAFETY: the caller has proven that `target` names a process group this
    // process created and still owns, and none of the signals sent here
    // requires a userspace handler.
    unsafe {
        libc::kill(target, signal);
    }
}

/// Records that a subsystem started a child in its own process group.
pub fn record_spawn(subsystem: &'static str, description: &str, child_pid: u32) {
    record(Event::Spawn {
        subsystem,
        description,
        child_pid,
    });
}

/// Records an authoritative completion status observed without reaping.
pub fn record_completion(
    subsystem: &'static str,
    source: &'static str,
    child_pid: u32,
    code: Option<i32>,
    signal: Option<i32>,
) {
    record(Event::Completion {
        subsystem,
        source,
        child_pid,
        code,
        signal,
    });
}

enum Event<'a> {
    Signal {
        site: Site,
        signal: libc::c_int,
        child_pid: u32,
        anchor: Option<GroupAnchor>,
        outcome: GroupSignalOutcome,
    },
    Spawn {
        subsystem: &'static str,
        description: &'a str,
        child_pid: u32,
    },
    Completion {
        subsystem: &'static str,
        source: &'static str,
        child_pid: u32,
        code: Option<i32>,
        signal: Option<i32>,
    },
}

static AUDIT_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
static AUDIT_BYTES: AtomicUsize = AtomicUsize::new(0);

fn audit_path() -> Option<&'static PathBuf> {
    AUDIT_PATH
        .get_or_init(|| {
            std::env::var_os(AUDIT_PATH_VARIABLE)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .as_ref()
}

fn record(event: Event<'_>) {
    let Some(path) = audit_path() else {
        return;
    };
    append_record(path, &format_record(&event), &AUDIT_BYTES);
}

/// Appends one record while `budget` still allows it.
///
/// The budget is charged before the write and is never refunded, so a process
/// that reaches the limit stops recording rather than truncating a record or
/// growing without bound. One `write` of an append-only regular file keeps a
/// record whole when several Runyte processes share one journal.
fn append_record(path: &std::path::Path, line: &str, budget: &AtomicUsize) {
    let used = budget.fetch_add(line.len(), Ordering::Relaxed);
    if used + line.len() > MAX_AUDIT_BYTES {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

fn format_record(event: &Event<'_>) -> String {
    let mut line = String::with_capacity(256);
    // SAFETY: `getpid` reads this process's own identity and cannot fail.
    let sender = unsafe { libc::getpid() };
    line.push_str(&format!("sender_pid={sender}"));
    match event {
        Event::Signal {
            site,
            signal,
            child_pid,
            anchor,
            outcome,
        } => {
            let target = -(*child_pid as libc::pid_t);
            crate::log::append_field(&mut line, "event", &"signal");
            crate::log::append_field(&mut line, "subsystem", &site.subsystem);
            crate::log::append_field(&mut line, "call_site", &site.call_site);
            crate::log::append_field(&mut line, "child_pid", child_pid);
            crate::log::append_field(&mut line, "target", &target);
            crate::log::append_field(&mut line, "signal", signal);
            crate::log::append_field(
                &mut line,
                "child_state",
                &match (anchor, outcome) {
                    (_, GroupSignalOutcome::AlreadyComplete) => "reaped",
                    (_, GroupSignalOutcome::OwnershipUnproven) => "unknown",
                    (Some(anchor), _) => anchor.as_str(),
                    (None, _) => "unknown",
                },
            );
            crate::log::append_field(
                &mut line,
                "outcome",
                &match outcome {
                    GroupSignalOutcome::Sent => "sent",
                    GroupSignalOutcome::AlreadyComplete => "already_complete",
                    GroupSignalOutcome::OwnershipUnproven => "ownership_unproven",
                },
            );
            append_identity(&mut line, *child_pid);
        }
        Event::Spawn {
            subsystem,
            description,
            child_pid,
        } => {
            crate::log::append_field(&mut line, "event", &"spawn");
            crate::log::append_field(&mut line, "subsystem", subsystem);
            crate::log::append_field(&mut line, "child_pid", child_pid);
            crate::log::append_field(&mut line, "command", &bounded(description));
            append_identity(&mut line, *child_pid);
        }
        Event::Completion {
            subsystem,
            source,
            child_pid,
            code,
            signal,
        } => {
            crate::log::append_field(&mut line, "event", &"completion");
            crate::log::append_field(&mut line, "subsystem", subsystem);
            crate::log::append_field(&mut line, "source", source);
            crate::log::append_field(&mut line, "child_pid", child_pid);
            crate::log::append_field(&mut line, "code", &format_args!("{code:?}"));
            crate::log::append_field(&mut line, "signal", &format_args!("{signal:?}"));
        }
    }
    line.retain(|character| character != '\n' && character != '\r');
    line.push('\n');
    line
}

/// Adds what the kernel currently says about the target's group and session.
///
/// A group number that has been recycled answers these differently from the
/// child Runyte spawned, which is what lets a reader separate a stale signal
/// from a legitimate one after the fact.
fn append_identity(line: &mut String, child_pid: u32) {
    let Ok(pid) = libc::pid_t::try_from(child_pid) else {
        return;
    };
    // SAFETY: both calls only read process identity for `pid` and report
    // `-1` with `errno` set when it names nothing.
    let (group, session) = unsafe { (libc::getpgid(pid), libc::getsid(pid)) };
    crate::log::append_field(line, "getpgid", &group);
    crate::log::append_field(line, "getsid", &session);
}

fn bounded(text: &str) -> String {
    const LIMIT: usize = 120;
    if text.len() <= LIMIT {
        return text.to_owned();
    }
    let mut end = LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    const TEST_SITE: Site = Site::new("test", "tests");

    /// A child in a private process group, as every caller of this module
    /// spawns one. Without the group of its own, `-pid` would name the test
    /// harness's group instead of the child's.
    fn running_child() -> Child {
        use std::os::unix::process::CommandExt as _;

        Command::new("/bin/sh")
            .args(["-c", "while :; do sleep 1; done"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("a shell can be started in a test environment")
    }

    #[test]
    fn a_reaped_child_never_yields_a_claim_on_its_recycled_group() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        child.wait().unwrap();

        assert!(matches!(
            claim_child_group(TEST_SITE, &mut child),
            Claim::AlreadyComplete
        ));
        assert_eq!(
            signal_child_group(TEST_SITE, &mut child, libc::SIGKILL),
            GroupSignalOutcome::AlreadyComplete
        );
    }

    #[test]
    fn a_running_child_anchors_its_group_and_is_signalled() {
        let mut child = running_child();
        let pid = child.id() as libc::pid_t;

        let Claim::Owned(group) = claim_child_group(TEST_SITE, &mut child) else {
            panic!("a running child anchors its group");
        };
        assert_eq!(group.leader(), pid);

        let mut sent = Vec::new();
        group.signal_with(libc::SIGKILL, |target, signal| {
            sent.push((target, signal));
            deliver(target, signal);
        });

        assert_eq!(sent, [(-pid, libc::SIGKILL)]);
        child.wait().unwrap();
    }

    #[test]
    fn one_claim_covers_a_whole_teardown_sequence() {
        let mut child = running_child();
        let pid = child.id() as libc::pid_t;

        let Claim::Owned(group) = claim_child_group(TEST_SITE, &mut child) else {
            panic!("a running child anchors its group");
        };
        let mut sent = Vec::new();
        for signal in [libc::SIGHUP, libc::SIGKILL] {
            group.signal_with(signal, |target, signal| {
                sent.push((target, signal));
                deliver(target, signal);
            });
        }

        assert_eq!(sent, [(-pid, libc::SIGHUP), (-pid, libc::SIGKILL)]);
        child.wait().unwrap();
    }

    #[test]
    fn an_anchored_group_is_signalled_without_reaping_its_leader() {
        let mut child = running_child();
        let pid = child.id() as libc::pid_t;

        let group = claim_anchored_group(TEST_SITE, pid, GroupAnchor::UnreapedLeader);
        let mut sent = Vec::new();
        group.signal_with(libc::SIGKILL, |target, signal| {
            sent.push((target, signal));
            deliver(target, signal);
        });

        assert_eq!(sent, [(-pid, libc::SIGKILL)]);
        // The leader is still waitable here, which is the point of the anchor.
        assert_eq!(child.wait().unwrap().code(), None);
    }

    #[test]
    fn a_record_stays_on_one_line_and_names_its_sender_and_target() {
        let line = format_record(&Event::Signal {
            site: Site::new("git", "stop_child_tree"),
            signal: libc::SIGKILL,
            child_pid: 4321,
            anchor: Some(GroupAnchor::UnreapedLeader),
            outcome: GroupSignalOutcome::Sent,
        });

        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.ends_with('\n'));
        // SAFETY: `getpid` reads this process's own identity.
        let sender = unsafe { libc::getpid() };
        assert!(line.starts_with(&format!("sender_pid={sender} ")), "{line}");
        for field in [
            "event=signal",
            "subsystem=git",
            "call_site=stop_child_tree",
            "child_pid=4321",
            "target=-4321",
            "child_state=unreaped_leader",
            "outcome=sent",
            "getpgid=",
            "getsid=",
        ] {
            assert!(line.contains(field), "{field} missing from {line}");
        }
    }

    #[test]
    fn a_spawn_record_bounds_the_command_it_describes() {
        let line = format_record(&Event::Spawn {
            subsystem: "git",
            description: &"argument ".repeat(40),
            child_pid: 7,
        });

        assert!(line.contains('\u{2026}'), "{line}");
        assert!(line.len() < 400, "{line}");
    }

    #[test]
    fn records_are_appended_whole_and_stop_at_the_journal_budget() {
        let directory = std::env::temp_dir().join(format!(
            "runyte-process-audit-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("audit.log");
        let budget = AtomicUsize::new(0);

        let line = format_record(&Event::Spawn {
            subsystem: "git",
            description: "rev-parse --show-toplevel",
            child_pid: 99,
        });
        append_record(&path, &line, &budget);
        append_record(&path, &line, &budget);
        let after_two = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after_two.lines().count(), 2);
        assert!(
            after_two.contains("command=rev-parse --show-toplevel"),
            "{after_two}"
        );

        // Everything past the budget is dropped rather than half-written.
        budget.store(MAX_AUDIT_BYTES, Ordering::Relaxed);
        append_record(&path, &line, &budget);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), after_two);

        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn no_journal_is_opened_when_the_audit_variable_is_unset() {
        // The integration suites set the variable in the environment of the
        // processes they start, never in this one.
        assert!(
            std::env::var_os(AUDIT_PATH_VARIABLE).is_none(),
            "the audit variable must not be set for ordinary test runs"
        );
        assert!(audit_path().is_none());
    }
}
