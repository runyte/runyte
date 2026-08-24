---
title: "Vim pending hints label incomplete commands as exact"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: 8ef17cb
---

## Resolution

Commit 8ef17cb (`Fix Vim pending hint labels`) fixed `draw_key_hints`, which
was unconditionally setting `exact` on every grammar-owned Vim hint. The Vim
grammar now exposes `vim_hint_is_exact` so presentation can distinguish an
immediately executable continuation from a continuation that still awaits a
character or enters a nested grammar namespace. Operator `f`/`F`/`t`/`T`,
`g`, `i`, and `a` rows therefore use the same `›` marker as registry-owned
namespaces, while runnable motions retain `(exact)`.

Coverage is in
`input_grammar::tests::vim_registers_are_deferred_and_grammar_hints_are_not_runyte_rows`
in `src/input_grammar.rs` and
`vim_help_and_pending_operators_use_grammar_owned_rows` in
`tests/key_hints.rs`.

## Report

In Vim mode, pressing `g` listed the possible keybindings with many rows
ending in "(exact)". That was inconsistent with the native Runyte popup, where
a namespace needing more keys gets a "›" appended to its description and
"(exact)" appears only when a sequence is both immediately runnable and a
prefix of a longer one.

In Vim mode every hint row was unconditionally marked "(exact)", including
rows that were not yet runnable. Under an operator such as `d`, the `g` hint
("operator g motions") and the `i`/`a` hints ("inside/around syntax object")
are not complete commands on their own — `dg` needs a third key such as `dgg`,
and `di`/`da` need a text-object key such as `diw` — yet all were tagged
"(exact)" as though they would run immediately.

The Vim hint popup should use the same two markers as the native popup: "›"
for rows needing more keys, and "(exact)" only for rows that are both runnable
now and ambiguous with a longer sequence.
