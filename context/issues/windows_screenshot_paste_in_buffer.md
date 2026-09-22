# Screenshot paste with `Ctrl-v` does not work in a buffer on Windows

On Windows, pressing `Ctrl-v` with a screenshot on the clipboard in an editable
Runyte buffer has no visible effect. No image link or text appears. The
screenshot tool, terminal, buffer mode, and clipboard formats have not yet been
recorded.

To reproduce, copy a screenshot to the system clipboard, focus an editable
Runyte buffer, and press `Ctrl-v`. For an image-only clipboard, the expected
behavior is to store the image under the workspace's `.runyte/cache/images/`
directory and insert a numbered Markdown link such as `[Image 1](...)` into
the buffer. The documented clipboard rule gives text precedence when the
clipboard offers both text and image formats.
