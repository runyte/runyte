# Windows explorer title shows a verbatim path prefix

On Windows, opening the explorer can show a pane title like:

```text
[explorer] \\?\C:\Users\...
```

The `\\?\` prefix is Windows' extended path syntax. It may be needed for
filesystem operations, but the pane title should present an ordinary,
recognizable path such as `C:\Users\...`. `Buffer::pane_title` currently
formats the stored directory path directly, which can expose the prefix.

Reproduce by opening an explorer for a Windows directory whose stored path is
in verbatim form and reading the pane title. The report concerns the displayed
spelling; any change must preserve the path's filesystem identity and the
behavior of long and UNC paths.
