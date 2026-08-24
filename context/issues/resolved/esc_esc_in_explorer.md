---
title: "Explorer path completion requires two Esc presses to leave Insert mode"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: 32b9889
---

## Resolution

Commit `32b9889` (`Fix Explorer escape with path completion`) corrected
`App::handle_completion_key`, which consumed every `Escape` while a completion
popup was open. Editing after an Explorer directory marker opened path
completion automatically, so the first press only closed that popup and never
reached the registry's `enter-normal-mode` binding. The completion handler now
dismisses the popup but lets that same `Escape` continue through normal key
dispatch when the active buffer is an Explorer.

Ordinary file-buffer completion deliberately keeps its existing dismissal
behavior; the Explorer differs because its structural trailing slash can open
completion without an explicit request. `FsPlan::build` already rejected an
identified directory whose destination starts below its own path. The
end-to-end coverage now verifies that `:w` reports that refusal before opening
a filesystem-plan confirmation.

The behavior is covered by
`one_escape_leaves_insert_mode_when_path_completion_is_open_in_an_explorer` and
`writing_a_directory_inside_itself_is_rejected_before_confirmation` in
`tests/directory_buffer.rs`.

## Report

Editing a directory name after its trailing slash in the Explorer required two
`Esc` presses to return to Normal mode. For example, given:

```text
file1.txt
file2.txt
dir_a/
dir_b/
```

editing the listing to:

```text
file1.txt
file2.txt
dir_a/some_string
dir_b/
```

made the first `Esc` insufficient; one press was expected to leave Insert mode.
Writing that edited directory name with `:w` was also expected to reject
`dir_a/some_string` rather than offer a filesystem change for confirmation.
