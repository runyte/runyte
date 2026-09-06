# Session navigation implementation review

Examined: working-tree implementation based on `9750cd0`, 2026-09-06.
Scope: the session-navigation plan, local Navigator, session strip and directory
chooser, parent-terminal control requests, external-editor waits, and remote
open-destination inventories. Three subagents reviewed architecture, navigation,
and the integrated implementation; the third reviewer also cross-reviewed the
other two implementations. This record contains their actionable comments and
resolution, rather than unrestricted tool output.

## Review comments and responses

- **Attachment completion and history (architecture reviewer).** The old
  switcher dropped the source attachment before knowing whether the destination
  accepted it, and its recovery endpoint was not successful-attachment history.
  Parent handoffs now retain a bounded receipt in the source host until the TUI
  acknowledges the destination's welcome and first frame. Previous-session
  history advances only after successful attachment. Duplicate/expired receipts
  report errors instead of repeating a switch.
- **Delayed parent requests (independent reviewer).** A control connection
  accepted under one TUI could delay its request until a replacement TUI attached.
  Parent requests now validate the interactive attachment identity captured at
  control connection time, in addition to the terminal capability, live terminal,
  active terminal context for attachment handoffs, and kernel peer ownership of
  the PTY's Unix session. Parent waits may originate in another visible pane.
- **Input and failed-open boundaries (independent reviewer).** Overlay detection
  alone omitted command/search input and pending grammar. Parent requests now
  check the complete input context. Parent waits preflight their originating
  pane before opening or refreshing requested files, so a hidden-origin refusal
  does not modify the working set.
- **Request ownership across navigation and splits (architecture reviewer).**
  Pane-only covered-terminal state is insufficient: navigation clears that state,
  and closing a pane does not necessarily close a shared buffer. Explicit
  request-buffer ownership and completion events now preserve scoped quit
  semantics across navigation and splits. Parent waits suppress ordinary
  detached-client TUI takeover. Explicit detach cancels them; session switching
  leaves them pending. Unsaved shared buffers retain cancellation protection.
- **Navigator focus and history (navigation and independent reviewers).** The
  terminal manager's Show action moves a PTY, while Navigator Enter must focus
  its existing pane. Separate focus and bring-here paths retain one PTY view.
  A mixed destination history records terminal leave transitions independently
  of buffer fallback and jump history.
- **Stale Navigator selections (independent reviewer).** Removing a selected
  resource could make Enter choose the new first row. Removal now invalidates
  acceptance until another selection/query gesture. Surviving selections retain
  their resource identity; action indices are never retargeted.
- **Identity labels and highlights (independent reviewer).** Basename-only rows
  hid structural kinds and dirty state and could hide a matched path abbreviation.
  Navigator rows now carry structural relative paths and `[+]`, `[STALE]`, `[RO]`.
  Match emphasis is included in semantic snapshots even without a preview.
  Terminal Show is labelled Bring into active pane in the Navigator.
- **Terminal cleanup race (regression review).** A child exit refreshed the
  terminal manager and erased its pending action menu. Refresh now preserves
  the menu, filter and selected terminal identity. Bulk cleanup determines exited
  identities from current state when executed and preserves live children.
- **Directory chooser updates (independent reviewer).** Background worktree and
  observation replies reset selection to the first directory, and pasted paths
  bypassed chooser query state. Updates now preserve selected paths, and pasted
  text follows the same completion state as typed paths.
- **Remote inventory validation (independent reviewer).** Sender-side bounds
  did not protect the receiving client from oversized replies or zero resource
  IDs. Both bounds and unique nonzero identities are validated at receipt;
  activation rechecks the host incarnation and resource identity after attachment.
  Delayed replies are guarded by request generation and selected root.
- **Observation and strip geometry (navigation and primary reviewers).** Full
  catalog refresh includes Git work and must not run per frame. Background
  discovery uses a bounded asynchronous 15-second observation, omits Git, and
  skips attention-only reads for hidden/zen presentation. Unchanged observations
  do not publish frames. Narrow-width overflow prioritizes the current identity;
  exact-fit entries are not unnecessarily omitted. Existing viewing rules clear
  unread/bell state, without acknowledging hidden terminals on session visits.
- **Strip updates during terminal output (architecture reviewer).** Compact
  terminal damage previously ignored session-strip changes when geometry and
  terminal rows were unchanged. Strip changes now require a full semantic frame,
  covered by
  `protocol::frame::tests::changed_session_strip_requires_full_frame_even_when_terminal_rows_match`
  in `src/protocol/frame.rs`.
- **Performance measurement boundaries (navigation reviewer).** Automatic strip
  discovery can take one observation cycle before attention polling is enabled;
  a 17-second warm-up admitted initial attention updates into the idle window.
  The benchmark now settles for 32 seconds, covering both 15-second cycles.
  Warm attachment waits for a document-content token absent from its filename,
  so a pane title cannot end the measurement prematurely. CPU counts each
  editor process once and excludes terminal children.

## Validation

Behavior tests are in `src/app/tests/session_navigation.rs`,
`src/app/tests/workspace.rs`, `tests/local_protocol.rs`, the catalog tests in
`src/workspace/catalog.rs`, and strip snapshot tests in `src/ui.rs`.
On Linux with Rust 1.97.1, `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` pass. The full suite
reports 2,959 passed and 31 ignored. Tests ran with real local sockets and PTYs,
including outer-TUI handoff and return to the original shell, parent waits, and
standalone refusal without nested editor startup.

Canonical `cargo llvm-cov --locked --workspace` also passes, reporting 91.46%
total line coverage (99,788 of 109,110), above the enforced 89% floor. Details
are in `context/reference/test-coverage.md`. Native macOS validation remains
unverified in this Linux environment.

Release performance measurements use three samples each for one host, three
hosts, a hidden strip, and a noisy terminal in another host. All 12 idle windows
have zero screen writes. Median cold startup is 32.59–33.48 ms, warm attachment
5.22–6.55 ms, and editor CPU 0.00–0.25%. The
[performance register](../reference/startup-performance.md) records the method,
individual samples, and limits of these first-output timings.
