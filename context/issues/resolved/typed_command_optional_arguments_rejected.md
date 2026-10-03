---
title: "Typed commands reject valid optional arguments"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 7dc376f
---

## Resolution

Commit `7dc376f` (`fix(commands): accept valid typed optional arguments`).

`CommandInvocation::from_parts` omitted the optional parameter forms produced
by the named parser for eight terminal commands and the path Finder. It now
accepts those forms alongside the parameterless key-binding form and still
validates execution context. Git stash commands accept an absent name to open
the existing prompt, while explicitly blank names remain invalid.

Coverage: `typed_construction_preserves_named_optional_argument_invocations`
and `typed_construction_accepts_the_entire_named_command_inventory` in
`src/command.rs` reconstruct parsed invocations, retain keyboard calls, and
reject incompatible parameter/context values. All 19 command tests passed.

## Report

The validated `CommandInvocation::from_parts` constructor rejects argument
shapes accepted by `parse_named_command` for terminal commands and the path
Finder. A typed caller cannot reconstruct `terminal htop`,
`terminal-file-directory htop`, `terminal-directory-root htop`,
`terminal-selected-directory htop`, `terminal-session-directory 1`,
`terminal-show 1`, `terminal-rename build`, `terminal-send 1`, or
`file-picker-path src` from their command identity, parameters, and execution
context: reconstruction returns `InvalidParameters`.

The same discrepancy affects `git-stash-tracked`, `git-stash-all`, and
`git-stash-untracked` without arguments. The named commands accept an absent
name and open the native name prompt, whereas typed construction requires a
nonempty name. An absent name should preserve the prompt behavior; an explicitly
empty or whitespace-only name should still be rejected.

These terminal commands parse to `OptionalText`, and the path Finder parses to
`OptionalPath`, but the constructor accepts only `None` for their identities.
Typed callers should accept the same valid parameters as named commands while
retaining the parameterless invocations used by key bindings. Invalid parameter
types, character operands, and unsupported counts must still be rejected.

Reproduce by parsing one of these commands, passing its `id()`, cloned
`parameters()`, and `execution()` to `CommandInvocation::from_parts`, and
comparing the reconstructed invocation with the original.
