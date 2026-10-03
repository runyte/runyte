Repository discovery changes valid filesystem paths before using them.

`GitCliProvider::discover_with_marker_probe` decodes each `git rev-parse`
path response as UTF-8 and calls `trim_end`. A repository whose root ends in
a space, tab, or newline is consequently addressed under a different path
when discovery asks for its Git directories. On Unix, a root containing
non-UTF-8 bytes is refused even though Git and the editor can address it.

Initialize a temporary repository, rename its directory to a name ending in
a space, and call `discover` on that directory. Discovery fails when the
shortened directory is absent and can inspect the wrong repository if the
shortened directory exists. The same mismatch applies to trailing whitespace
in an explicitly separate Git directory.

Discovery should remove only Git's one output newline and decode filesystem
paths with the platform's lossless path representation. Empty required path
responses must still fail. Other textual Git output retains its existing
parsing rules.
