# macOS explorer clipboard paste loses line breaks

On macOS, selecting several files in the explorer, copying them with
`Space c y`, and pasting with `Cmd-v` produces a single line: the separators
between the copied entries disappear.

Expected behavior is that each copied entry retains its line break when
pasted into an editable document. The terminal's paste action must preserve
multiline text, including when it transports newline separators as carriage
returns. Existing LF and CRLF text must remain valid, and paste must remain
literal text rather than executing editor keys. Single-line prompts retain
their control-character rejection.

Reproduce by opening a directory containing multiple files, selecting several
entry lines, pressing `Space c y`, then using `Cmd-v` to paste into a document
in Insert mode. The report does not identify the outer terminal application
or specify whether the original destination was a document or a terminal.
