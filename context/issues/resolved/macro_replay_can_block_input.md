---
title: "Macro replay can block input for an unbounded duration"
status: resolved
reported: 2026-08-25
resolved: 2026-08-25
commit: a6ca494
---

## Resolution

Commit `a6ca494` ("Bound and yield macro replay") replaced synchronous replay
in `App::replay_macro` with one root-owned execution state. The old function
recursively dispatched every recorded input and only rejected the innermost
call at depth sixteen, so enclosing branches and large counts could continue
doing finite but effectively unbounded work while the host could not regain
control.

The new replay state lazily advances recorded inputs, nested macro frames, and
counted command repetitions. Direct and mutual cycles abort the complete root
with their register chain, distinct calls retain a defensive sixteen-level
nesting limit, and one 10,000-unit budget covers raw events, literal-text
characters, semantic repetitions, and nested work. Each host slice is bounded
by 128 charged work units. Stateful grammar-level ranges that cannot safely be
split are refused above 128 repetitions per recorded input. Oversized macro
snapshots and text events are measured before cloning or applying them.

The standalone and persistent-session event loops now schedule replay between
host events. While replay owns input, `Escape` and `Ctrl-c` cancel it; other
keyboard, text, pointer, key-hint, and attached semantic-command paths cannot
interleave. Lifecycle commands stop trailing replay immediately, exact batch
boundaries release input ownership, and errors from the final replayed action
remain visible. Cancellation and abort deliberately retain completed effects
because recorded input may invoke non-transactional workflows.

Regression coverage is provided by
`recursive_macro_replay_aborts_the_whole_root_before_trailing_inputs`,
`mutual_macro_recursion_reports_the_active_register_chain`,
`one_total_work_budget_bounds_large_counted_replay`,
`a_recorded_maximal_command_count_is_expanded_cooperatively`,
`grammar_level_counts_cannot_bypass_the_macro_work_budget`,
`semantic_range_work_counts_toward_the_current_replay_slice`,
`an_oversized_recorded_text_event_is_refused_before_it_edits`,
`an_oversized_raw_recording_is_refused_before_snapshotting`,
`macro_replay_preserves_action_errors_across_progress_and_completion`,
`replay_finishing_on_a_batch_boundary_releases_input_immediately`,
`a_lifecycle_command_stops_trailing_macro_input`, and
`escape_and_ctrl_c_cancel_cooperative_macro_replay` in
`src/app/tests/editing_and_buffers.rs`;
`direct_pointer_input_cannot_interleave_with_macro_replay` in
`src/app/tests/presentation_and_settings.rs`;
`macro_owned_input_clears_hints_before_frontend_dispatch` in `src/main.rs`;
`host_macro_replay_returns_between_input_and_cooperative_playback` and
`attached_semantic_commands_cannot_interleave_with_macro_replay` in
`src/workspace/host.rs`; and
`a_macro_cannot_focus_a_hidden_pane_by_maximizing_before_the_next_frame` in
`tests/maximized_panes.rs`.

Known limitation: one ordinary editor action and one literal-text event remain
atomic. Literal text is still bounded by the 10,000-unit root budget, and
grammar-level ranges have the separate 128-repetition per-action limit.

## Report

Macro replay runs synchronously inside the input dispatch that started it. The
editor does not redraw, process service events, or accept cancellation until
the complete replay returns.

A recursion-depth check rejects a replay once sixteen macro calls are active,
but it only returns from that innermost call. The enclosing macro continues.
A macro with more than one recursive replay can therefore branch into a very
large amount of finite work, and mutually recursive macros have the same
behavior. A non-recursive macro can also be replayed with a count as high as
999,999. In either case the terminal can appear hung for a long time even
though the recursion depth itself is bounded.

Macro replay should have one bounded execution state shared by the top-level
macro, every nested macro, counted command repetitions, and literal text.
Direct and mutual recursion should be refused with the register chain that
caused it, and exhausting the total work budget should stop the entire replay
rather than only one nested call. Playback should advance in bounded batches
between host events so standalone and persistent-session frontends remain
responsive.

`Escape` or `Ctrl-c` typed by the user while playback is active should cancel
the remaining work. An abort keeps edits and other actions that already
completed; replay cannot promise rollback because recorded input can invoke
non-transactional editor workflows.

Reproduction:

1. Record macro `@a` so it replays `@a` more than once, or record `@a` and
   `@b` so that each replays the other.
2. Replay `@a`.
3. Observe that the depth-limit error does not stop the enclosing replay and
   input is unavailable until all branches unwind.

The same lack of responsiveness can be reproduced by recording a macro with
several inputs and replaying it with a very large count.
