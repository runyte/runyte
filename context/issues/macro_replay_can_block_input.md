Macro replay runs synchronously inside the input dispatch that started it. The
editor does not redraw, process service events, or accept cancellation until
the complete replay returns.

A recursion-depth check rejects a replay once sixteen macro calls are active,
but it only returns from that innermost call. The enclosing macro continues.
A macro with more than one recursive replay can therefore branch into a very
large amount of finite work, and mutually recursive macros have the same
behavior. A non-recursive macro can also be replayed with a count as high as
999,999. In either case the terminal can appear hung for a long time even
though the recursion depth itself is bounded.

Macro replay should have one bounded execution state shared by the top-level
macro, every nested macro, counted command repetitions, and literal text.
Direct and mutual recursion should be refused with the register chain that
caused it, and exhausting the total work budget should stop the entire replay
rather than only one nested call. Playback should advance in bounded batches
between host events so standalone and persistent-session frontends remain
responsive.
`Escape` or `Ctrl-c` typed by the user while playback is active should cancel
the remaining work. An abort keeps edits and other actions that already
completed; replay cannot promise rollback because recorded input can invoke
non-transactional editor workflows.

Reproduction:

1. Record macro `@a` so it replays `@a` more than once, or record `@a` and
   `@b` so that each replays the other.
2. Replay `@a`.
3. Observe that the depth-limit error does not stop the enclosing replay and
   input is unavailable until all branches unwind.

The same lack of responsiveness can be reproduced by recording a macro with
several inputs and replaying it with a very large count.
