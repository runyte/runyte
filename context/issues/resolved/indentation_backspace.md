---
title: "Backspace removes indentation one character at a time"
status: resolved
reported: 2026-09-30
resolved: 2026-09-30
commit: a95da12
---

## Resolution

Commit `a95da12` (`Align indentation backspace and add language and file
overrides`) changes Insert-mode Backspace in `src/app/editing.rs`. Its ordinary
delete path previously selected one character even when the entire prefix was
indentation. It now finds the previous visual tab stop through leading spaces
and tabs, and sends that span through the existing transactional deletion path.
Selection deletion, pair deletion, Markdown list unwinding and newline joining
retain their existing precedence. Ordinary text and Replace mode retain character
deletion.

A shared per-buffer resolver applies each configured field independently:
global defaults, then language values, then matching workspace-relative file
patterns in declaration order. Editing, rendering, hit testing and LSP formatting
use that same width and style. Generated comparison buffers inherit their source
document's settings without acquiring a writable path. Unlike Helix's language
indent defaults or Neovim's separate indentation options, Runyte keeps one width
and adds no automatic language defaults or indentation detection.

The configuration page offers contextual language and file-pattern override
creation on width/style rows, displays saved overrides, and supports direct
editing and removal. Prompts show inherited values and sources. Atomic scoped
writes preserve unrelated YAML, comments and pattern order; removing one field
writes null so the field inherits again. Failed writes retain a retryable UI.

Coverage includes `backspace_aligns_leading_whitespace_and_preserves_ordinary_deletion`,
`backspace_merges_overlapping_carets_and_undo_restores_indentation`,
`language_and_path_widths_reach_tab_backspace_and_reload`,
`settings_keys_create_edit_and_remove_an_explicit_language_override`,
`file_pattern_flow_validates_cancels_and_saves_without_losing_target`,
`scoped_style_preview_rolls_back_on_cancel_and_failed_save`,
`pattern_prompt_uses_the_origin_documents_inherited_value_and_source`,
`simultaneous_panes_render_and_hit_test_their_own_tab_widths`,
`numeric_override_save_failure_keeps_input_and_can_retry`,
`document_comparison_snapshots_inherit_source_widths_without_local_paths`, and
`failed_override_removal_retains_the_row_and_explains_the_retry_action` in
`src/app/tests/indentation.rs`. Configuration precedence, glob validation,
comment-preserving writes and scalar-looking filename keys are covered by
`tests/indentation.rs`; early exit outside indentation is covered by
`ordinary_backspace_stops_at_the_first_non_indent_character` in
`src/indentation/tests.rs`.

Known limitation: EditorConfig, automatic indentation detection, and separate
soft-tab and display widths are not implemented. File rules apply to local paths
inside the workspace; language rules also apply to pathless/provider documents.

## Report

GitHub report: https://github.com/runyte/runyte/issues/10

In Insert mode, Tab inserts spaces to a configured indentation stop, but
Backspace removes only one character of ordinary leading indentation. Helix
removes indentation to the preceding level instead.

The report includes a side-by-side recording, Runyte on the left and Helix on
the right, editing an indented Rust statement:
https://github.com/user-attachments/assets/2da1be16-6f31-4905-9e61-c9b6b0135a7c

Expected behavior is aligned deletion within leading spaces and tabs. With a
four-column width, Backspace at columns eight, six and four should reach four,
four and zero respectively. Tab should advance to the next multiple. After text,
Backspace should retain character deletion. Existing Markdown list handling,
selection deletion, pair deletion and newline joining remain applicable.

Width and indentation style must remain configurable globally, with language
exceptions such as Python width four and YAML width two, and file-pattern
exceptions. The settings page should offer contextual override creation, show
saved overrides only, and allow direct editing and removal. The input should
show inherited values and their sources. The original video report did not
specify configuration precedence or an override UI.
