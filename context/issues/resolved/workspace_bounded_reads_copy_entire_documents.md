---
title: "Bounded session reads copy whole documents before truncating"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: c81e0f3
---

## Resolution

Commit `c81e0f3` (`perf(session): copy only bounded buffer read prefixes`).

`WorkspaceHost::read_buffer` now converts only the rope prefix ending at the
1 MiB UTF-8 boundary. Buffer session previews similarly materialize only the
first 240 characters per row. Previously both paths created whole-document or
whole-line strings before discarding almost all of them on the host thread.
Control-character replacement, tabs, viewport position and truncation flags
retain their previous behavior. A trailing-terminator scan is skipped whenever
it cannot affect the visible prefix; when needed it uses a reverse rope iterator
instead of repeated indexed character lookups.

The three tests `bounded_buffer_reads_preserve_utf8_boundaries_and_truncation`,
`bounded_buffer_previews_keep_long_unicode_lines_and_control_spelling`, and
`bounded_buffer_previews_preserve_long_carriage_return_suffixes` in
`src/workspace/host.rs` passed. They cover empty and exact-limit reads,
multibyte characters straddling the byte limit, long Unicode previews, controls,
retained viewport position, and long carriage-return suffixes. The parent
reviewer checked the slicing boundaries and requested the reverse-iterator
follow-up to avoid introducing repeated rope searches.

Known limitation: preserving the existing removal of an arbitrary trailing
carriage-return run can require scanning that suffix when it affects the
preview prefix. That scan is linear and does not allocate the discarded text.
Terminal preview construction is separate from this buffer-only change.

## Report

Bounded persistent-session reads perform work proportional to the whole
document before returning their bounded output. `WorkspaceHost::read_buffer`
first calls `Buffer::to_string`, copying every byte, and only then retains the
first 1 MiB. Buffer rows in `WorkspaceHost::session_preview` similarly call
`line_string` before retaining the first 240 characters of each of at most eight
rows. A large single-line document therefore causes a large temporary
allocation even for a compact preview.

These synchronous operations run on the workspace host thread. Repeated
control reads or session previews can delay editor input and rendering, with
temporary memory and copy cost growing with the source document rather than
the response limit.

Read only the required rope prefix or line slice before converting it to an
owned string. Preserve the exact 1 MiB UTF-8 boundary and `truncated` flag for
buffer reads, and the current 240-character preview limit, terminal-control
replacement, tabs, line terminator treatment, and viewport position.

Reproduction: open a document containing one 128 MiB ASCII line. Request its
buffer contents through the persistent-session control API or request a
session preview. Although the result contains at most 1 MiB or 240 characters,
respectively, the current implementation first allocates and copies the whole
128 MiB line. Compare the same requests against a small document with an
identical returned prefix; only the discarded suffix changes the copy cost.
