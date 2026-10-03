Debug builds open the optional `RUNYTE_INPUT_TRACE` destination with a blocking,
truncating `OpenOptions` call. A FIFO without a reader blocks startup before
input processing; other special objects are accepted as trace destinations.

The trace must open only regular files and validate the opened descriptor
before truncation or writes. Opening a FIFO or a symbolic link to one must
return an error promptly without changing the special object. Existing regular
files must still be truncated for a new trace, and missing regular trace files
must still be created. Release builds do not enable this diagnostic trace.
