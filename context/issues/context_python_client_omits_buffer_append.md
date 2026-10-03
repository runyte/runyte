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
