The Unix clipboard helper runner waits for its stdin writer through
`collect`, which returns `Ok(None)` when its deadline expires. The success
branch only checks `Err`, so a helper that exits successfully while a descendant
retains an undrained stdin pipe can be reported as a successful copy even though
the input writer never delivered the full value.

A successful copy requires confirmation that all input bytes were written.
Reproduce with a checked-in stand-in helper that leaves a descendant holding
stdin without reading it, then exits zero; send more input than the pipe
capacity. A bounded timeout must report failure and clean up the owned process
group. A normal helper that consumes all input and forks a legitimate selection
owner must retain its existing successful behavior.
