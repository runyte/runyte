---
title: "Program cache files can block startup or grow without a bound"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 4c5729c
---

## Resolution

Commit `4c5729c` (`fix(files): bound and validate program cache files`).

ProgramCache loads only descriptor-validated regular files and reads at most 64 KiB plus the overflow sentinel before parsing. Persistence opens without truncation, uses nonblocking Unix descriptors, validates the opened object, and only then truncates and writes within the same byte limit. Missing or unreadable hints continue to degrade to an empty cache; ordinary symlink targets remain supported.

Coverage: oversized_program_cache_files_are_discarded_before_parsing and special_program_cache_files_cannot_block_loading_or_persistence in src/external_open/tests/cache_files.rs. The latter owns the bounded special_cache_fixture subprocess and checks both cache names, regular shorter rewrites, and symlink persistence. Both regressions failed before the fix; 26 external_open tests pass, with the helper fixture ignored outside its owning test.

## Report

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
