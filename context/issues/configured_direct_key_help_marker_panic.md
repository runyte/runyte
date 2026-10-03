Opening contextual help can panic when a configured direct key action contains
text that looks like a help key marker. For example,
`keys: {bind: {normal: {F12: {command: pipe, argument: 'echo {key:nope}'}}}}`
is accepted, but opening
help treats the shell argument as an authored marker and panics when resolving
the unknown command `nope`.

The direct-key table in `help::render_document_with_descriptions` appends
configured descriptions without escaping markers. Scoped and Insert/Replace
tables already distinguish these descriptions from built-in authored prose.
Valid-looking markers can also silently change the displayed action argument.

All configured description text must remain literal in help, with its own
backticks governing prose styling. Keep registry-owned key labels styled as
keys and preserve the authored built-in markers. Cover both unknown markers
and valid-looking markers in direct configured actions without executing them.
