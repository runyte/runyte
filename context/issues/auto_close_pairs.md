# Optional automatic closing of brackets and quotes

[Issue #9](https://github.com/runyte/runyte/issues/9) requests automatic closing
of brackets `() [] {}` and quotes `'' ""`. Typing an opener currently inserts
only that character. When enabled, typing an opener should insert its matching
closer and leave the caret between them.

The setting must be available in `Space o o` and disabled by default. The
original report leaves context heuristics, closer skipping and Backspace
behavior unspecified. Literal paste and terminal input must retain their text.
