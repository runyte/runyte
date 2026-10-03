---
title: "Pasted image links misinterpret reserved pathname characters"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 5900be7
---

## Resolution

Commit `5900be7` (`fix(images): preserve reserved path characters in pasted links`).

destination now escapes literal percent signs before introducing escapes for hash, backslash, and angle brackets. image_reference_target normalizes path separators only on Windows, preserving Unix backslashes as filename characters. Ordinary readable cache paths keep their spelling and existing space/parenthesis wrapping.

Coverage: destination assertions in src/pasted_image.rs and pasted_image_links_preserve_reserved_characters_in_state_paths in src/app/tests/editing_and_buffers.rs. The latter pastes real image bytes into temporary state roots containing hash, literal percent escapes, spaces/parentheses, and Unix backslashes, then follows each link through gf to the stored binary target. All 11 targeted tests pass.

Known limitation: general navigation deliberately tries literal filenames before percent decoding. If both an encoded spelling and its decoded pathname exist, that existing precedence still selects the literal spelling; this change does not redesign ambiguous path navigation.

## Report

# Pasted-image references can interpret pathname characters as Markdown syntax

`pasted_image::destination` wraps spaces and parentheses and escapes angle
brackets, but leaves `#` unchanged. With a configured state root such as
`cache#scratch`, an image reference contains a target like
`cache#scratch/cache/images/<hash>.png`. Following that Markdown link with
`gf` splits the destination at `#` and treats the rest as a heading fragment,
instead of opening the stored image. External Markdown readers also interpret
the character as a URL fragment. `App::image_reference_target` also replaces every backslash with `/` on
Unix, although a backslash is a literal filename character there. A state root
named `cache\scratch` is consequently spelled as the nonexistent nested path
`cache/scratch`. Percent-encoded path spellings must retain their literal
meaning when introduced to repair the link encoding.

Image references should preserve the exact stored image identity for supported
pathnames. A reproduction uses temporary workspace/configuration storage with
a state-root directory containing `#`, pastes an image, and follows the generated
link. The stored file exists, but navigation currently looks up an unrelated
path or heading. Ordinary `.runyte/cache/images/` references should keep their
existing readable spelling.
