With whitespace markers enabled, an unwrapped row whose final visible cell
cannot fit the next wide character or tab can show `¬` at that cell even though
the real line ending is offscreen. For example, an eight-cell text viewport
showing `aaaaaaa界\n` renders `aaaaaaa¬`. The snapshot producer checks spare
screen width and whether this is a final wrapped segment, but does not check
that it actually consumed the remaining document characters.

The same end-of-line decoration path can emit two caret cells. With the caret
at the end of `a\n` and whitespace markers enabled, it marks `¬` as the caret,
then appends another caret-styled blank after it.

Only decorate a line ending after its preceding text is visible, and let the
existing line-ending marker own the single caret cell. Preserve bounded scans
of long lines, ordinary clipping, wrapping, and the fallback blank caret when
whitespace markers are disabled or no line terminator exists.
