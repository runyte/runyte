---
title: "Runyte has no built-in about page"
status: resolved
reported: 2026-08-13
resolved: 2026-08-13
legacy_commit: 4414693
---

## Resolution

Commit `4414693` (`Add a read-only about page`) added the missing semantic
`show-about` command and exposed it as `:about` through the shared command
inventory. There was previously no command or generated view for a product
introduction, so the version and first steps were available only through
separate documentation and command surfaces.

The new `about::render` builds a compact front page from the checked-in
`logo/ascii/logo.txt`, the package version compiled into the running binary,
and a short selection-first navigation guide. `App::open_about` opens that
page as the reusable `[about]` virtual buffer,
which gives it ordinary buffer navigation, search, splitting, closing, and
read-only mutation protection without introducing a separate overlay or UI
path. The logo asset is included in published source packages so the same
source remains available outside a repository checkout. A follow-up corrected
the initial renderer to center the imported logo as one block with a uniform
prefix; centering its source rows independently had destroyed their relative
indentation and therefore the shape. The getting-started block was then aligned
on the left and updated to name help, files, the explorer, open buffers,
cross-buffer history, the command palette, and quit. Its selection guidance
now reads `Navigate. Select. Act.`, avoiding the Helix-specific implication
that ordinary Runyte movement creates or extends a selection. A targetless
standalone `runyte` launch now opens the same page as its first visible frame;
directory and file targets continue through their existing explorer and
file-opening paths. Until presentation-level centering exists, the generated
page uses a fixed layout whose widest line begins after ten spaces.

Coverage lives in `src/about.rs` as
`about_contains_the_source_logo_version_and_first_steps` and
`about_places_its_widest_line_after_ten_spaces`, and in `src/app.rs`
as `about_command_opens_one_read_only_front_page`.

Known limitation: the page has fixed padding baked into its buffer text, so it
does not reflow or recenter when its pane changes width. General live centering
for generated and interactive read-only content is tracked in
`context/issues/resolved/auto_centered_virtual_content.md`.

## Report

A `:about` command was requested to describe what Runyte is and show the
current version in a read-only buffer like `:h`.

The page should contain only high-level information about Runyte. Its top
center should show the logo and text from `logo/ascii/logo.txt`, followed by
basic information about what Runyte is and how to get around. Neovim's front
page was given as inspiration.
