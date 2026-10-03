Language-server location results can crash the editor when a result uses the
largest representable protocol row. With at least two definition or reference
locations, the editor builds a picker and adds one to each zero-based `u32` row
before formatting it. A row of `4294967295` overflows: debug builds panic, while
release builds display row zero.

The response must remain harmless presentation data even when a server returns
an out-of-range position. The picker should display the one-based row without
overflow, and selecting that result should retain the existing document-bound
clamping used by navigation. Text mutation coordinates must retain their stricter
validation.

Reproduce by delivering a references response containing two local-file
locations, with one start position at line `4294967295`, character `0`, then
opening the resulting picker.
