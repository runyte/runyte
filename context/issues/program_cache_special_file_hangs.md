`ProgramCache::load` opens `recent-programs` and `default-program` with
`fs::read_to_string`, and persistence uses `fs::write`. A named pipe at either
cache pathname can therefore block startup or a program-choice action while
waiting for a peer. The startup cache is supposed to degrade to an empty hint
list when unreadable. The 16-program limit applies only after the whole file
has been read, so it does not bound the initial allocation.

Use an injected temporary cache root and replace each cache file with a FIFO
without a peer. Loading must return without waiting; remembering a program or
persisting a default must report a failure without waiting or modifying the
special object. Regular cache files, missing files, and normal remembered
choices must keep working. Reads should have a finite byte budget before text
allocation, and descriptor validation must precede reads or truncation.
