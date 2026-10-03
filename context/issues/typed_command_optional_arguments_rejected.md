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
