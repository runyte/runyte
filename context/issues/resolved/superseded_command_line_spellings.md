---
title: "Superseded command-line spellings remained public and accepted"
status: resolved
reported: 2026-08-18
resolved: 2026-08-18
legacy_commit: 09bf13d
---

## Resolution

Commit 09bf13d (`Unify workspace command-line terminology`) removed the
compatibility branches from `LaunchArguments::parse_from`. The parser had
continued to map verb-first workspace options and the older host-named family
onto the same launch modes as the canonical `--workspace-*` options, despite
there being no deployed compatibility requirement. It also singled out `-l`
with a replacement message instead of treating it like any other unknown
option.

Only `--workspace-list`/`--wls`, `--workspace-stop`/`--wst`,
`--workspace-restart`, `--workspace-name`, and `--workspace-rename` now reach
those management modes. All retired long options and `-l` take the ordinary
`unknown option` path. The obsolete compatibility paragraphs were removed
from `runyte --help` and `README.md`; the short options remain documented as
the command-line equivalents of `:wls` and `:wst`.

Coverage lives in
`launch::tests::workspace_modes_accept_only_current_and_short_spellings` and
`launch::tests::superseded_workspace_spellings_are_unknown_options` in
`src/launch.rs`, plus
`editor_help_hides_internal_options_and_uses_workspace_modes` in
`tests/release_packaging.rs`.

## Report

`runyte --help` documented command-line spellings that existed only for
backward compatibility, although Runyte had a single user and no deployed
scripts written against the old names.

The canonical spellings were:

- `--workspace-list`, with `--wls`
- `--workspace-stop`, with `--wst`
- `--workspace-restart`
- `--workspace-name`
- `--workspace-rename`

The superseded verb-first spellings were `--list-workspaces`,
`--shutdown-workspace`, `--restart-workspace`, `--name-workspace`, and
`--rename-workspace`. The still older host-named family was `--list-hosts`,
`--shutdown-host`, `--restart-host`, `--name-host`, and `--rename-host`.

The `WORKSPACES:` block in `--help` ended with a paragraph explaining which
spellings were superseded and that they still worked. With the compatibility
spellings removed, only the sentence naming `--wls` and `--wst` as the
abbreviations of `:wls` and `:wst` remained relevant. `README.md` carried the
same compatibility paragraph and a note describing `-l` as the one removal.

An option that no longer parses should fail like any other unknown option.
`-l` instead had a bespoke message explaining that `--workspace-list` and
`--wls` replaced it.

The test
`workspace_modes_accept_current_short_and_superseded_spellings` in
`src/launch.rs` asserted that every superseded spelling still parsed; the
required behavior was for canonical and short spellings to parse while all
removed spellings were rejected.
