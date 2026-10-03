Configured key remappings can be accepted even when the input grammar cannot
execute them. For example, `keys: {leader: '1'}` moves the leader menu onto a
decimal count key, and `keys: {rebind: {'Space e': '2 e'}}` advertises an explorer
binding whose first key is consumed as a count. A target such as `F12 Esc e` or
`F12 Backspace e` is also accepted, although the second key cancels or removes
the pending prefix.

The compiler checks structural keymap collisions but does not check these
grammar reservations for named prefixes and `keys.rebind`. `keys.bind` already
rejects comparable assignments. The accepted remappings remove their working
default bindings and cause help and hints to advertise unreachable commands.

Reject unreachable mappings non-fatally, retain the affected defaults, and
preserve independent valid rules. Check effective modes and scopes so Insert
mode's ordinary Tab and the directory tree's numbered pane keys remain valid.
Regression coverage should exercise both compiled lookup and actual grammar
dispatch after rejection.
