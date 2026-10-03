Configuration reads use blocking file opens without checking the opened
object's type. On Unix, a configuration path replaced with a FIFO that has no
writer makes `:config-reload` wait indefinitely on the editor thread. The same
problem affects startup through `--config`, saving a setting such as a theme,
and saving an indentation override. A symlink to a FIFO behaves identically.
An unbounded character device can instead keep the read running indefinitely.

Load a regular configuration, replace that file externally with a named pipe,
and run `:config-reload` or save a setting to reproduce the stall. The editor
should reject the special file promptly and leave its working configuration
and the filesystem object intact. Existing regular-file symlinks and absent
configuration files must retain their current behavior.

Use the existing descriptor-checked regular-file opener for each configuration
read path. Regression tests must run potentially blocking operations in a
bounded subprocess so a regression cannot hang the entire test runner.
