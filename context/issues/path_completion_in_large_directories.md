# Path completion in large directories

Path completion offers no matches in a directory with many entries. The
behavior was observed while typing a path in insert mode: the popup that
opens on `/` lists entries, and the first character typed after it empties
the popup. Typing the next `/` makes entries appear again for a moment, and
the following character empties it once more. The command palette's rows for
a path argument behave the same way.

Small directories complete correctly, so the failure depends on the number of
entries rather than on the path being typed.

## Observed cause

Both path popups bound the filesystem work one keystroke may do, and both
applied that bound to the directory read rather than to what they keep:

- `App::path_completion` collected the first 512 entries `fs::read_dir`
  returned and offered them to the typed fragment afterwards.
- `App::matching_path_hints` did the same with the first 512 entries before
  comparing them against the typed prefix.

A directory read returns names in whatever order the filesystem holds them,
which for a large directory is neither sorted nor related to what is being
typed. In a directory of 20,000 files, the retained 512 are about 2.5% of it,
so a typed name is almost never among them and the popup shows nothing.

## Expected behavior

A name that exists in the directory is offered when it is typed, whatever the
size of the directory and wherever the filesystem returns it. A bound on how
many rows are shown is expected; a bound that decides which matches exist is
not.

## Constraints

The read happens on the input thread, between the keystroke and the redraw
that answers it, so completing a path in a very large directory must stay
within a keystroke. The command palette recomputes its rows for every frame
it draws, so its cost is paid again on each redraw.

## Reproduction

Create a directory with 20,000 files:

```sh
mkdir -p /tmp/wide && cd /tmp/wide
for i in $(seq -w 0 19999); do : > "file_$i.txt"; done
```

In insert mode, type `/tmp/wide/` — entries appear — then type `file_0`. The
popup empties, though 10,000 entries match that prefix. The same happens with
`:open /tmp/wide/file_0`.
