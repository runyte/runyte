---
title: "Raw input normalization could diverge or retain stale repeat state"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: 6f4a40d
---

## Resolution

Commit 6f4a40d (`Normalize raw text input boundaries`) made the standalone and
attached event loops enforce the same one-MiB text-input limit before editor
dispatch. Oversized standalone input now produces a visible host error, while
an attached client sends a bounded notification instead of causing protocol
framing to reject and disconnect the client.

`KeyRepeatDetector::observe` was also retaining legacy press history across
non-key input. It now resets that history for text, resize, and other non-key
events, preventing later presses from being misclassified as repeats.

Coverage lives in `src/main.rs` in
`non_key_input_resets_legacy_repeat_history` and
`text_input_limit_is_shared_before_standalone_or_attached_dispatch`, with the
shared byte boundary defined and tested in `src/input.rs`.

## Report

Raw terminal input decoding and its normalization into frontend-independent
editor input required a focused hardening review. The scope included
`src/tui/input.rs`, `src/input.rs`, `src/input_grammar.rs`, the event-loop input
boundary in `src/main.rs`, local protocol input DTOs, and their tests.

The review covered modifier and shifted-key handling, enhanced keyboard
events, repeat detection, escape ambiguity, bracketed paste, large paste
bounds, mouse coordinates and gestures, resize events, focus events,
unsupported events, standalone versus attached equivalence, macOS and Linux
differences, and input received during mode, pane, or workspace transitions.
Platform normalization was required to remain separate from command dispatch
policy.
