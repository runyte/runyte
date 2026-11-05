# Per-terminal scrollback and stable review

Status: implemented. Tracking issue:
[#62](https://github.com/runyte/runyte/issues/62).

## Behavior

Replace the shared terminal history/review byte budget with
`terminal.scrollback_lines`, default 2,000. The limit counts primary-screen
history rows per terminal, excluding its visible screen. Zero disables history; the supported maximum is 100,000 rows.
Native review retains one immutable snapshot per terminal, independently of the
history limit, until input resumes or the terminal is closed. Other terminals
and subsequent output cannot evict a review.

Changing the setting updates existing and future terminals. A reduction trims
old live history immediately, including hidden primary history while the
alternate screen is active, without changing a captured review. Increasing the
setting cannot recover discarded rows. Live trimming invalidates Finder
results even without subsequent output. Finder activation reuses existing
review and refuses target identities absent from it. Emulator resets preserve the configured
limit. Prepared plugin terminal installation uses the current limit.

External context captures keep the existing per-reader and 64 MiB host limits,
independent of terminal history and native review. No public wire shape changes.
Total native terminal memory scales with terminal count, width, configured
history, and retained review snapshots.

## Implementation

1. Add typed configuration, validation, settings discovery/persistence, and
   initial/live propagation. Match the existing integer setting workflow;
   avoid destructive trimming during cancellable setting previews.
2. Store the line limit in each primary grid, preserve it on emulator reset,
   trim with correct lost-history/revision accounting, and propagate it through
   terminal creation and prepared installation.
3. Remove shared native memory accounting and all review eviction calls.
   Preserve independent context capture admission and expiry limits.
4. Add behavior tests at grid, terminal, application settings/input, and context
   boundaries; update the user guide and current terminal compatibility record.
5. Validate formatting, Clippy, terminal and desktop suites, coverage and guide
   links. Independent implementation review covers the integration paths above;
   validation results and environment limitations belong in the fixing PR.

## Validation focus

- Defaults, zero and custom values, invalid values, persistence and runtime
  changes, including new/prepared terminals.
- Multiple busy terminals retain independent newest rows; alternate-screen
  history and reset obey the configured value.
- Movement and line selection survive output and history truncation; copying
  returns the selected frozen text; repeated review reuses the snapshot and
  returning to input releases it.
- Context captures retain admission limits without charging native terminals.
