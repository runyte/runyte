# Backspace removes indentation one character at a time

GitHub report: https://github.com/runyte/runyte/issues/10

In Insert mode, Tab inserts spaces to a configured indentation stop, but
Backspace removes only one character of ordinary leading indentation. Helix
removes indentation to the preceding level instead.

The report includes a side-by-side recording, Runyte on the left and Helix on
the right, editing an indented Rust statement:
https://github.com/user-attachments/assets/2da1be16-6f31-4905-9e61-c9b6b0135a7c

Expected behavior is aligned deletion within leading spaces and tabs. With a
four-column width, Backspace at columns eight, six and four should reach four,
four and zero respectively. Tab should advance to the next multiple. After text,
Backspace should retain character deletion. Existing Markdown list handling,
selection deletion, pair deletion and newline joining remain applicable.

Width and indentation style must remain configurable globally, with language
exceptions such as Python width four and YAML width two, and file-pattern
exceptions. The settings page should offer contextual override creation, show
saved overrides only, and allow direct editing and removal. The input should
show inherited values and their sources. The original video report did not
specify configuration precedence or an override UI.
