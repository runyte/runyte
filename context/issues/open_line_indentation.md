# Open-line commands discard indentation

[Issue #8](https://github.com/runyte/runyte/issues/8) reports that opening a line
with `o` or `O` starts at column zero even inside an indented block. The new
line should preserve the surrounding indentation, as Enter does.

Reproduction: place the caret on an indented line, press `o` or `O`, and type.
The text begins at column zero instead of after the inherited indentation.
The report does not specify syntax-driven extra levels or list continuation.
