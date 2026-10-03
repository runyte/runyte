---
title: "Published context Python client and schema omit buffer append"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: bc6f6a1
---

## Resolution

Commit `bc6f6a1` (`fix(context): include buffer append in the published Python client`).

The host and bridge already implement buffer.append, but the vendorable Python method registry and JSON schema omitted it. Both now accept the documented shape under buffer_edit. The Python boundary enforces nonempty UTF-8 byte limits and treats append as a mutation, so a lost response reports outcome_unknown without retry. An omitted or null expected_tail follows the native Option semantics; a supplied string must be nonempty and bounded.

Tests: `test_append_success_and_uncertain_delivery` and `test_append_shape_and_utf8_limits` in `docs/plugins/check_context.py` cover successful wire delivery, missing permission before send, disconnect uncertainty, optional/null tails, Unicode byte bounds and closed request fields. All 15 context client/schema tests pass.

Known limitation: standard JSON Schema lengths count characters; exact UTF-8 byte limits are enforced by the Python client and native host, as for other text-bearing context requests.

## Report

The published Python context client and context JSON schema reject
`buffer.append`, although the current context protocol, documentation, native
host, and MCP bridge support it. Calling
`validate_request('buffer.append', {'buffer': 'b:1', 'text': 'line\n'})` raises
`invalid_argument: Method is not in the context profile`, and the equivalent
request fails the published schema.

The vendorable Python client and schema should accept the documented append
shape under `buffer_edit`: a buffer handle, nonempty UTF-8 text bounded to
512 KiB, and an optional nonempty UTF-8 `expected_tail` bounded to 4 KiB.
Missing permissions must reject the call before writing socket bytes. Because
an append is a mutation, loss of a matching response must report
`outcome_unknown` without retrying the request.
