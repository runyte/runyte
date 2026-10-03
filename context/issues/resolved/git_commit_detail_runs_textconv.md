---
title: "Commit detail reads execute configured Git text converters"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: e20e236
---

## Resolution

Commit `e20e236` (`fix(git): disable text converters in commit detail reads`)
adds `--no-textconv` to `GitCliProvider::commit_detail` alongside its existing
`--no-ext-diff`. Git's text-conversion mechanism is independent of external
diff commands, so disabling only the latter still ran configured converters
and displayed their transformed text. Commit details now use stored file
contents consistently with the other native diff readers.

`commit_detail_never_executes_configured_text_converters` in
`tests/git_provider.rs` installs the checked-in stand-in as a converter in a
temporary real Git repository. It proves no converter side effect occurs and
asserts both original and changed source lines appear in the patch. The test
failed before the fix and passed after it; all three related commit-detail
tests passed. The parent reviewer checked the command arguments and fixture.

## Report

Opening a commit detail can run a configured text conversion command.

`GitCliProvider::commit_detail` runs `git show --patch --no-ext-diff` without
`--no-textconv`. Git therefore applies a configured `diff.<driver>.textconv`
command selected by the file's `diff` attribute. Other Runyte diff readers
explicitly disable both mechanisms. A converter can have side effects or
delay opening the commit, and the displayed patch represents converted text
rather than the stored file contents.

Configure a text converter for a tracked file through `.gitattributes` and
repository configuration, make a commit changing that file, and open that
commit from the Git history. The converter executes. Reading a commit patch
should use the stored text without executing conversion commands, consistent
with other native diff views.
