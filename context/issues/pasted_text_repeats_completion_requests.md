Pasting text refreshes completion and signature help once per pasted character
after the complete insertion has already moved the caret to its final position.
Every refresh therefore observes the same final buffer and caret, and trigger
characters in the middle of the pasted text issue requests for a context that
is no longer at the caret. Path completion can also repeatedly resolve and
rank the same final directory listing on the editor input thread.

With a language server advertising `.` as a completion trigger and `(` as a
signature-help trigger, paste `.(.(.(.(.(.(.(.(.(.(.(.(.(.(.(.(.(.(.(.(.` in
Insert mode. A single insertion sends 41 requests: 21 completions and 20
signature-help requests, all at the same final position. With explicit language
completion active, pasting 20 consecutive `.` characters restarts it 20 times.

A text insertion must refresh the final completion context once, with at most
one request for each applicable language service. Ordinary single-character
typing, explicit completion ownership, filtering by the complete inserted
text, whitespace termination, and path/word completion precedence must remain
intact. Trigger characters that are no longer at the caret must not schedule
obsolete requests.
