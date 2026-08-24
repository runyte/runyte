---
title: "Unicode signature-help parameter offsets could panic rendering"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: 2258911
---

## Resolution

Commit 2258911 (`Validate signature help parameter spans`) normalized signature
parameter ranges at the LSP boundary. `signature_lines` previously retained
protocol offsets directly and mixed a byte start with a character-count end
for simple labels. Both forms were later interpreted as Rust byte indices.

Offset labels are now converted from UTF-16 code units to byte indices only
when both endpoints fall on complete Unicode scalar boundaries and form an
ordered, in-bounds range. Simple labels use their byte start and byte length.
Invalid server ranges discard only the emphasis while preserving the signature
text. The TUI also independently checks ordering, bounds, and UTF-8 boundaries
before slicing so malformed state cannot panic a frame.

Coverage lives in `src/lsp/mod.rs` and `src/ui.rs`. Run
`cargo test signature --lib`; the focused tests are
`signature_offsets_convert_from_utf16_to_valid_byte_ranges`,
`simple_unicode_signature_labels_use_byte_ranges`, and
`malformed_signature_parameter_ranges_render_without_emphasis`.

## Report

LSP signature-help parameter offsets could panic TUI rendering for valid
Unicode labels or malformed server data.

Offset labels from the protocol were stored without normalization. The
simple-label path found a byte offset but added a character count, while the
TUI later used both values as Rust byte slice indices. Rendering checked only
that the end was not greater than the string length; it did not require
`start <= end` or either value to be a UTF-8 boundary.

A signature containing non-ASCII text could therefore produce an invalid byte
range, and a server could also return reversed or out-of-bound offsets. Slicing
the label then panicked during every affected frame.

Protocol offsets needed to be normalized once into validated byte or character
spans, with `start <= end` and UTF-8 boundaries guaranteed. Invalid spans
should render without emphasis rather than panic. Coverage needed to include
valid Unicode labels, UTF-16 offset semantics, reversed offsets, and offsets
inside a multi-byte character.

Relevant code was `src/lsp/mod.rs` in `signature_lines` and `src/ui.rs` in
`draw_signature`.
