# A `--wait` client can survive terminal loss and consume a CPU core

## Observed behavior

A `runyte --wait note.txt` process remained alive after the terminal and
development worktree that launched it were no longer present. The process was
runnable at approximately 100% CPU, meaning one complete logical core, and had
accumulated more than a day of CPU time. No editor request was still being
actively used.

This is not an ordinary Unix zombie: the process continues to execute and
consume CPU. It is an abandoned `--wait` client whose lifecycle no longer has
a person or terminal capable of completing the request.

The normal `--wait` loops request status at 100 ms intervals and should be
nearly idle. The observed full-core load is consistent with terminal input
repeatedly becoming ready after hangup without producing an event. Runyte uses
Crossterm 0.29 `EventStream`; its Unix reader does not currently turn a
zero-byte terminal read into stream termination. A live stack or syscall trace
has not yet confirmed that path for the reported process, so the diagnosis
must be verified while reproducing the failure.

## Expected behavior

A `--wait` client should exist only while it owns a reachable pending wait
request or a terminal attachment that can complete it. Losing the controlling
terminal must cancel or release the wait request according to the existing
failure semantics, restore any terminal state that remains reachable, and exit
nonzero within a bounded interval. It must not spin after EOF, hangup, host
failure, or loss of its launching process.

Normal pending waits must remain inexpensive. Status polling, terminal input,
and transport reads should block between real events rather than continuously
waking a runtime or helper thread.

## Reproduction

A controlled reproduction should:

1. start `runyte --wait note.txt` in a disposable PTY with a real test-scoped
   persistent host;
2. leave the wait request pending and abruptly close the PTY master, without
   sending Runyte a normal detach or quit command;
3. assert that the client exits within a short deadline and that the host no
   longer retains its wait request; and
4. retain enough process diagnostics to distinguish a blocked client from a
   thread repeatedly polling and reading the closed terminal.

The failure may depend on whether the host already has an interactive TUI and
whether the waiting invocation takes over after that TUI detaches. Both paths
need coverage.

## Constraints

- Preserve the documented handoff in which a pending `--wait` invocation takes
  over the terminal after another interactive TUI detaches.
- Preserve explicit completion, cancellation, host-failure, and signal exit
  statuses.
- Do not treat temporary absence of input as terminal loss.
- Do not solve the leak by adding another frequent polling loop.
- Cover the behavior in a subprocess so a stuck Crossterm reader cannot hang
  the test runner itself.
