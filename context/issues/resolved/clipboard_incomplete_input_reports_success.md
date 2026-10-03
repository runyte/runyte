---
title: "Clipboard helpers report successful copies with incomplete input"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 75cb009
---

## Resolution

Commit `75cb009` (`fix(clipboard): require complete input before reporting copy success`).

run_helper_with_environment now requires the stdin writer to confirm completion when the helper exits successfully. A channel deadline returning no result is translated into an explicit timeout, which follows the existing owned-process-group cleanup path. Helpers that consume all input and retain a legitimate forked selection owner continue to succeed.

Coverage: successful_exit_requires_all_clipboard_input_to_be_written in src/clipboard/helpers.rs sends 1 MiB to a checked-in stand-in that exits zero while a descendant retains undrained stdin. It checks the timeout and absence of the descendant marker after cleanup. The regression reproduced false success before the change; all 19 clipboard tests pass, including the existing forked-selection-owner case.

The fixture saves stdin on descriptor 3 before starting the background owner.
Ubuntu's dash otherwise replaces the background job's stdin with `/dev/null`
before applying `<&0`, causing `BrokenPipe` instead of exercising the intended
stalled writer. Redirecting from the saved descriptor preserves the input pipe
on both dash and bash without weakening the timeout or cleanup assertions.

## Report

The Unix clipboard helper runner waits for its stdin writer through
`collect`, which returns `Ok(None)` when its deadline expires. The success
branch only checks `Err`, so a helper that exits successfully while a descendant
retains an undrained stdin pipe can be reported as a successful copy even though
the input writer never delivered the full value.

A successful copy requires confirmation that all input bytes were written.
Reproduce with a checked-in stand-in helper that leaves a descendant holding
stdin without reading it, then exits zero; send more input than the pipe
capacity. A bounded timeout must report failure and clean up the owned process
group. A normal helper that consumes all input and forks a legitimate selection
owner must retain its existing successful behavior.
