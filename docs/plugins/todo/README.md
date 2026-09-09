# Todo showcase: Python, Rust and C

Three independent implementations of the same native todo list demonstrate
Runyte's [experimental application API](../applications.md). Each program speaks
`runyte-experimental-2` over stdin/stdout and requests only `views`. No Runyte
library, Python bridge, terminal renderer, account or network service is needed.

| Variant | Source | Runtime/build dependencies |
| --- | --- | --- |
| Python | [python/todo.py](python/todo.py) | Python 3.10+, standard library only |
| Rust | [rust/src/main.rs](rust/src/main.rs) | Rust 1.88+, `serde_json` and `libc`; build with the included lockfile |
| C | [c/todo.c](c/todo.c) | C11 compiler, POSIX, pkg-config and json-c 0.15+ |

The examples target Linux and macOS. Each owns its own task list; enabling all
three does not share tasks between them. The original [tasks.py](../tasks.py)
remains the smaller example of using the Python client.

## Build and launch

Install the C development dependency for your platform:

```sh
# Debian / Ubuntu
sudo apt-get install build-essential pkg-config libjson-c-dev

# Fedora
sudo dnf install gcc pkgconf-pkg-config json-c-devel

# macOS (with Xcode command line tools)
brew install pkg-config json-c
```

From the Runyte checkout root, with Python and Rust installed:

```sh
python3 docs/plugins/todo/build.py
cargo build --locked
./target/debug/runyte --config "$PWD/target/todo-showcase/config.yaml"
```

The helper compiles both native programs and writes a demo configuration with
absolute paths for all three variants under `target/todo-showcase/`. It accepts
`--output DIRECTORY`, `--offline` for cached Rust dependencies, and the usual
`CC` and `PKG_CONFIG_PATH` environment variables. Generated files are build
artifacts. The config refers to this checkout's Python script; regenerate it if
the checkout moves. The first Rust build may download its locked dependencies.

To build the native programs individually:

```sh
cargo build --locked --release --manifest-path docs/plugins/todo/rust/Cargo.toml
mkdir -p target/todo-showcase
cc -std=c11 -O2 -Wall -Wextra -Wpedantic -Werror docs/plugins/todo/c/todo.c \
  $(pkg-config --cflags --libs json-c) -o target/todo-showcase/todo-c
```

For a Python-only demo, copy this entry into a Runyte configuration and replace
both paths. Rust and C entries use their compiled binary as `executable` and
omit `args`. No executable permission is needed on the Python script.

```yaml
plugins:
  - id: todo-python
    enabled: true
    api: runyte-experimental-2
    executable: /absolute/path/to/python3
    args: [/absolute/path/to/runyte/docs/plugins/todo/python/todo.py]
    capabilities: [views]
```

Configuration is loaded at workspace host startup. An already running persistent
host must restart to load a new configuration; restarting a plugin alone reloads
its program with the existing configuration. The build helper does not change
your personal configuration or an existing persistent session.

## Demonstrate the same workflow in each language

Open `:plugin.todo-python.open`, `:plugin.todo-rust.open`, or
`:plugin.todo-c.open`. The title identifies the implementation. Each starts with
one sample task, “Read the plugin guide”. The following commands use the Python
ID; substitute `todo-rust` or `todo-c` for either native variant.

| Action | Command / default key |
| --- | --- |
| Open or return to the retained list | `:plugin.todo-python.open` |
| Add a task while its list is active | `:plugin.todo-python.add "Write the release notes"` |
| Mark selected tasks done or unfinished | `Enter` or `:plugin.todo-python.toggle` |
| Remove selected tasks | `:plugin.todo-python.remove` |
| Switch between all and unfinished tasks | `:plugin.todo-python.filter` |
| Discover available list actions | `Tab` |

Quotes preserve spaces in the typed title; no shell evaluates it. Adding uses a
typed command argument, without a separate input form. Removal is immediate and
has no undo. Other commands act on the selected task IDs in the current view.
Normal movement, selections, search, copying, splits, help, Navigator and Finder
come from Runyte. Split a list into two panes, toggle a task, then switch filters
to show how both panes follow the same model while retaining their own selections.
Compare the three implementations in adjacent panes to demonstrate that language
choice does not change the native interaction.

## Behavior and boundaries

Each implementation retains at most 100 tasks, with titles of at most 256 UTF-8
bytes. Titles must contain something other than ASCII spaces and cannot contain
control characters or Unicode line/paragraph separators. IDs remain stable
through edits and filtering, and removed IDs are not reused. An empty filtered
list is valid; adding a task still works there.

Tasks live in the plugin's memory. Closing and reopening its view retains them,
and persistent-session detach keeps the process alive. Plugin stop/restart or
workspace host exit discards tasks. These demos do not save files or use the
workspace state store. `:plugin-stop todo-python` and `:plugin-restart todo-python`
exercise that lifecycle; substitute the other IDs as needed. A stopped plugin's
old view remains readable with Runyte's unavailable marker.

The implementations use a small event loop with one outstanding host request.
Another command receives `busy` while an update is pending; the reader continues
handling responses and view closure. Publication uses the captured view and exact
revision, and commits candidate task data only after the host acknowledges it.
Definite refusals discard that candidate. A closure received before publication
settles also discards the candidate. A timeout, ambiguous acknowledgement or
`outcome_unknown` never triggers a retry; restart the plugin before more work.
`pane.show` uses only the original pending invocation's foreground authority.

Input frames are bounded to 1 MiB including their newline, with small fixed read
chunks. Output uses nonblocking writes with a two-second delivery deadline;
opening/publishing has one eight-second deadline inside the host's command
lifetime. EOF stops promptly, even with an outstanding request. When idle, each
process blocks on input without a timer or polling loop. Diagnostics and task
data never go to stdout outside protocol frames.

JSON decoding and escaping come from Python's standard library, `serde_json`,
and [json-c](https://json-c.github.io/json-c/json-c-current-release/doc/html/json__tokener_8h.html)
respectively. The C implementation enables strict parsing and UTF-8 validation.
Runyte plugins run as trusted local programs; capability grants describe the
host API they use, as explained in the [plugin contract](../../plugins.md).

## Verify parity

After building all three variants and installing the development-only
`jsonschema` package:

```sh
python3 docs/plugins/check_todo.py --require-all
cargo fmt --manifest-path docs/plugins/todo/rust/Cargo.toml --check
cargo clippy --locked --manifest-path docs/plugins/todo/rust/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path docs/plugins/todo/rust/Cargo.toml
```

Set `RUNYTE_TODO_BIN_DIR` when building with a custom `--output`. The checker
launches actual programs in temporary working directories, validates every
outgoing message against the public schema, and runs the same cases for each
language: Unicode and limits, multi-row actions, filters, stale context, refusal
rollback, stable IDs, view closure, uncertain outcomes, fragmented input and EOF.
Programs are compiled before tests run. Without `--require-all`, missing compiled
variants are reported as skips; Python still runs. The complete
[conformance runner](../conformance.md) includes this suite, and CI builds and
requires all variants on Linux and macOS. These wire checks simulate the host;
they do not claim an automated TUI demonstration or Rust editor coverage.
