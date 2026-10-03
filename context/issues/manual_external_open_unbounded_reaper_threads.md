On Unix, manually opening a file or URL starts its external process and then
creates an infallible, dedicated reaper thread. Each application that stays
alive retains another thread, without the bound already used by native Windows
and plugin system-opening paths. If thread creation fails, `thread::spawn`
panics after the child has already started, leaving no assigned reaper owner.

Use the existing bounded external-opener ownership for these launches as well.
Admission and fallible reaper creation must succeed before a child can start.
A full opener pool must produce an actionable native error; an admitted child
must retain its independent stdio and process group and remain owned until it
exits. Thread-creation failure can be injected through the existing opener
fixture instead of exhausting actual machine resources. Explicit program
arguments and the final literal file/URL argument must keep their existing
meaning.
