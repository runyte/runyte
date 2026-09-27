# `gf` includes a sentence's full stop in a terminal path

In terminal review, `gf` with a bare cursor on a path at the end of a sentence
takes the full stop as part of the path, so opening fails.

## Reproduction

In an integrated terminal inside a workspace that contains `src/store.rs`,
print a line that ends a sentence with the path:

```sh
echo 'Added Store::complete and its test in src/store.rs.'
```

Enter review with `Ctrl-\` twice (terminal Normal, then review). Place the
cursor inside `src/store.rs`, without a selection, and press `gf`.

Observed:

```text
g f (Open the file or web link under the cursor · failed: path not found: src/store.rs.)
```

Expected: `src/store.rs` opens, with the trailing `.` treated as sentence
punctuation.

Selecting exactly `src/store.rs` first (for example `v` and `l` across the
path) and then pressing `gf` opens the file, so only the inference of the path
under a bare cursor is affected.

## Context

Coding agents such as Claude Code and Codex commonly end a reply with a
sentence naming the changed file, so the path is often followed by `.`, `,`,
`:` or a closing parenthesis. A file name can itself end in `.` on Unix, so a
fix probably needs to prefer the existing path when the punctuation-stripped
candidate exists and the literal one does not. The report does not decide
that rule, or whether the same inference applies to ordinary buffers.

Observed with Runyte 0.3.3 on Linux.
