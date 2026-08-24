---
title: "Yanking to the system clipboard freezes the editor until another program takes the clipboard over"
status: resolved
reported: 2026-08-10
resolved: 2026-08-10
legacy_commit: 95f5a38
---

## Resolution

Fixed by 95f5a38 "Stop a yank from freezing on the clipboard's selection
owner".

Nothing was wrong with the yank itself, which is why the copied text was
always there to paste elsewhere. `CommandClipboard::write` ended with
`child.wait_with_output()`, and that call reads the helper's stdout and stderr
to end of file. On Wayland and X11 the write helpers — `wl-copy`,
`xclip -selection clipboard -in`, `xsel --clipboard --input` — do not serve the
clipboard from the process Runyte spawned. They fork a process that owns the
selection for as long as the copied value lives and then exit immediately,
status 0, and the forked owner inherits the pipes given to its parent. Waiting
for those pipes to reach end of file therefore waited for the clipboard's
lifetime rather than the command's. The reporter's two-instance experiment is
that mechanism stated exactly: yanking in B makes B's helper the selection
owner, which ends A's owner, which finally closes A's pipes and lets A's event
loop resume.

The write path was the only one that could hang this way, but the fix is at the
boundary rather than in that one call, because the shape of the mistake — a
blocking wait on something the helper does not control — is available to every
one of them. `run_helper` is now the single way a clipboard helper is invoked,
by the text read and write paths and by the image capture the runner trait
wraps. Each of stdin, stdout, and stderr is handled on its own thread whose
result arrives over a channel with a deadline, and none of those threads is
ever joined: a pipe held open by a selection owner leaves one parked read
behind, which ends by itself when that owner releases the clipboard, and
joining it is precisely what froze the editor. The helper's own exit is polled
against the deadline with `try_wait`, and a helper still running when the
deadline passes is killed and reported as a timeout. Because the editor calls
the clipboard synchronously from its event loop, that deadline is also the
longest a keystroke can stall, on any platform: five seconds, chosen to clear a
cold `powershell.exe` start with room to spare while keeping a wedged display
server an error rather than a hang.

Two details are deliberate. Diagnostics are collected only when the helper
reported failure, because a helper that failed left no selection owner holding
its stderr and answers at once, while a successful one would cost the grace
period on every yank for a message nobody reads. And the write path gives the
helper `Stdio::null()` for stdout rather than a pipe, so there is one less
inherited descriptor for a forked owner to hold. `pbcopy` and `powershell.exe`
never daemonize and so were never affected on macOS or Windows; they gain the
timeout guard alone.

Tests: `a_forked_selection_owner_does_not_freeze_the_write` is the regression
test — a stand-in helper that consumes the value, forks a process holding the
inherited pipes, and exits — with
`a_helper_that_never_exits_is_killed_rather_than_waited_on`,
`helper_input_is_delivered_and_output_is_bounded`,
`a_missing_helper_is_skipped_and_a_failing_one_is_reported`,
`reading_uses_the_first_helper_that_is_installed`, and
`a_silent_failure_is_reported_with_its_exit_status`, all in
`src/clipboard.rs`. The five that drive a helper are `cfg(unix)`, since the
daemonizing behavior they stand in for is a Wayland and X11 one.

Known limitation: on Wayland and X11 each successful yank leaves one thread
parked on the selection owner's stderr pipe until the clipboard is replaced, so
the steady state is about one such thread and the next yank releases the
previous one. Clipboard reads are now bounded at 64 MiB, where they were
previously unbounded; a larger paste is refused rather than truncated. A helper
that hangs still costs the full five seconds before the editor reports it,
because the clipboard is called synchronously from the event loop — making it
asynchronous would be a change to how commands reach the editor, not to this
module.

## Report

`Space c y` froze the editor. The behavior needed to hold across Fedora,
Ubuntu, macOS, and Windows.

Yanking to the system clipboard itself succeeded: text yanked from a frozen
instance could still be pasted into another instance or any other application.
Only the instance the yank came from froze.

With two instances running in separate terminals, A and B: `Space c y` in A
froze A, and `Space c y` in B then released A and froze B.
