---
title: "`gf` includes a sentence's full stop in a terminal path"
status: resolved
reported: 2026-09-27
resolved: 2026-09-28
commit: a2013d0
---

## Resolution

Commit a2013d0 (`Fix gf path inference at sentence endings`) fixed
`App::goto_file_under_cursor` and terminal review's `gf` dispatch, which sent
the entire inferred path token to `App::open_navigation_target`. A sentence's
trailing `.` remained part of that token, so `src/store.rs.` could not resolve
when only `src/store.rs` existed. `navigation_target::under_cursor` also ended
file tokens before commas and closing wrappers, preventing an existing name
ending in one of those characters from receiving literal path precedence.

Both document buffers and terminal review now mark a target inferred under a
bare caret. `App::inferred_navigation_candidates` checks the path as written
first, then removes trailing sentence punctuation one character at a time
until a path resolves. It never tries the empty path, which would otherwise
resolve as a directory. The token parser retains punctuation that ends a path
before whitespace or a quote so a real punctuation-ending filename can win;
internal delimiters still separate tokens. Explicit selections and Markdown
link destinations retain their exact spelling. The same inference applies in
ordinary buffers as in terminal review, resolving the scope left open by the
report. This is Runyte's path policy; it does not change the `gf` binding or
the behavior of web links.

The behavior is covered by
`src/app/tests/navigation_and_files.rs::goto_file_trims_sentence_punctuation_only_for_missing_inferred_paths`,
`src/app/tests/navigation_and_files.rs::goto_file_in_terminal_review_trims_sentence_punctuation_but_keeps_literal_names`,
and `src/navigation_target/tests.rs::terminal_path_punctuation_reaches_literal_first_resolution`.

Known limitation: unselected paths with internal token delimiters such as
commas, semicolons, or closing brackets still require an explicit selection.

## Report

In terminal review, `gf` with a bare cursor on a path at the end of a sentence
takes the full stop as part of the path, so opening fails. This was observed
with Runyte 0.3.3 on Linux.

### Reproduction

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

Coding agents such as Claude Code and Codex commonly end a reply with a
sentence naming the changed file, so the path is often followed by `.`, `,`,
`:` or a closing parenthesis. A filename can itself end in `.` on Unix, so
the report raised the need to prefer a literal existing path over a shorter
punctuation-stripped candidate. The report left that precedence rule and
whether the same inference should apply to ordinary buffers undecided.
