---
title: "Failed file-manager pagination leaks a directory snapshot handle"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 7b2dd47
---

## Resolution

Commit `7b2dd47` (`fix(examples): release directory handles after failed pagination`).

The file-manager listing helper now retains the handle acquired by its first page and releases it when a later page returns a PluginError. It preserves the currently displayed directory handle and reports the original listing failure even when release also fails. This prevents an abandoned snapshot from exhausting the host limit of two directory handles.

Coverage: test_file_manager_releases_new_directory_when_later_page_fails in docs/plugins/check_applications.py verifies new versus displayed handles and successful versus failed cleanup after a stale second page. It failed before the fix; all 12 application conformance tests pass.

## Report

The local file-manager example retains a newly allocated host directory handle
when a later listing page fails. `listing` accumulates pages without releasing
the handle on error, and `refresh` starts its cleanup only after `listing`
returns successfully.

If the directory changes between pages, the host returns `stale`. Its first
page's snapshot remains allocated even though the example never adopts or
displays it. With one directory already visible, this consumes the second and
last directory slot, causing attempts to browse another directory to fail with
`Directory snapshot limit reached; release an old directory`.

On a failed paginated listing, release any newly acquired directory handle
while preserving the handle backing the currently displayed directory. Preserve
the original listing error if cleanup also fails. Reproduce by receiving a
first page with a new directory handle and a non-null `next`, then returning
`stale` for the subsequent page.
