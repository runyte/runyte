# Concurrent pathname reads during a Windows save

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
reporting. Do not replace the save algorithm solely on this single observation.
