---
title: "Concurrent pathname reads can fail during a Windows save"
status: resolved
reported: 2026-09-21
resolved: 2026-09-25
commit: 5295d2e
---

## Resolution

Commit `5295d2e` (Require Windows restart acceptance and characterize save
visibility) adds bounded native characterization around ordinary `Buffer::save`
replacement. The existing ConPTY fixture already waits for `wrote` before
inspecting saved bytes; that acknowledgement barrier is retained.

A private subprocess alternates two complete documents across 128 saves while
a concurrent reader opens the pathname. It records missing-file, denied-open,
sharing and lock errors separately from complete reads, rejects partial or
unknown contents, and requires durable completion and exact bytes immediately
after every save. Its subprocess deadline bounds both writer and reader even
if native I/O stalls. CI explicitly requires this acceptance case and retains
its captured characterization output.

At source revision `5295d2e`, Windows 11 Home build 26200 with Rust 1.97.1 on
`x86_64-pc-windows-msvc` observed 34,108 complete concurrent reads, 1,501
missing-file errors (2), and 1,823 sharing violations (32). All 128 saves were
durable and all 128 post-completion reads were exact. Counts are scheduling
observations, not thresholds. This confirms concurrent pathname opens can fail
during replacement on this environment; it does not establish the cause of the
original isolated failure. The user guide and atomic-save record distinguish
complete-content replacement from uninterrupted Windows pathname visibility.
No save algorithm, DACL/metadata or stream preservation, conflict check,
recovery backup, or error reporting was changed.

Native formatting, denied-warning all-target Clippy, the full Rust suite and
the exact CI commands passed. Subagent review had no actionable findings.
Regression coverage is
`successful_saves_have_complete_contents_after_acknowledgement` (with its owned
`concurrent_pathname_reads_fixture`) in `tests/windows_save_visibility.rs`, and
`real_editor_paste_and_save` in `tests/windows_acceptance.rs`.

Known limitation: this bounded run does not certify every filesystem or process
policy. Concurrent pathname errors remain possible before completion. Remote
cross-platform CI for this fix is pending; the Windows delivery record tracks it.

## Report

A native Windows ConPTY acceptance run on 2026-09-21 observed one
`std::fs::read_to_string` failure while polling an existing document immediately
after sending `:write`, before receiving the editor's completion status:

```text
Os { code: 2, kind: NotFound, message: "The system cannot find the file specified." }
```

The fixture creates a temporary text file, pastes Unicode text with LF alongside
its existing CRLF, sends `:write`, then checks the saved bytes. The failure was
in the pre-completion polling loop in `tests/windows_acceptance.rs`; it occurred
before the subsequent shell-filter command. The same save-and-paste fixture
also passed other native runs. There is no established deterministic
reproduction, missing file after acknowledged completion, or data-loss evidence.

Windows `buffer::replace_file` uses `ReplaceFileW` with a recovery backup and
checks the displaced file before removing that backup. Runyte does not
explicitly unlink the destination before that call. Microsoft's
[ReplaceFileW contract](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)
describes combined replacement steps and partial failure states without an
explicit concurrent-reader visibility guarantee. A transient namespace gap is
one hypothesis; the observed read error alone does not establish its cause.

The completed-save acceptance should wait for the editor's successful write
acknowledgement before asserting bytes. Separately, a bounded native test should
characterize concurrent pathname opens during successful replacement, including
whether readers can see `NotFound` or sharing errors and whether the document
always exists with complete contents after completion. Existing atomic-save
records describe atomic installation; any confirmed platform limit should be
reconciled with that description.

Any replacement change must retain metadata and DACL preservation, named
streams, conflict detection, recoverable displaced contents and failure
reporting. This single observation alone does not justify replacing the save
algorithm.
