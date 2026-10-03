---
title: "Configured direct key descriptions can panic while opening help"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 3fc286a
---

## Resolution

Commit `3fc286a` (`fix(help): keep configured direct action text literal`).

The direct-key table now identifies configured action and plugin descriptions
and escapes their key-marker syntax before resolving authored help markers.
It also records their prose range consistently with the existing scoped and
Insert/Replace tables. Unknown-looking markers therefore remain literal
instead of panicking, and valid-looking markers no longer rewrite an action's
argument. The registry's actual key labels keep their key styling, and
backticks in descriptions retain ordinary prose styling.

`direct_configured_action_descriptions_remain_literal_prose` in `src/help.rs`
reproduced the prior panic with a valid configured action and passes after the
fix. It checks unknown and valid-looking marker text, backtick styling, and
the F12 label without executing the configured command. The parent reviewer
checked description provenance and output range bookkeeping.

## Report

Opening contextual help can panic when a configured direct key action contains
text that looks like a help key marker. For example,
`keys: {bind: {normal: {F12: {command: pipe, argument: 'echo {key:nope}'}}}}`
is accepted, but opening
help treats the shell argument as an authored marker and panics when resolving
the unknown command `nope`.

The direct-key table in `help::render_document_with_descriptions` appends
configured descriptions without escaping markers. Scoped and Insert/Replace
tables already distinguish these descriptions from built-in authored prose.
Valid-looking markers can also silently change the displayed action argument.

All configured description text must remain literal in help, with its own
backticks governing prose styling. Keep registry-owned key labels styled as
keys and preserve the authored built-in markers. Cover both unknown markers
and valid-looking markers in direct configured actions without executing them.
