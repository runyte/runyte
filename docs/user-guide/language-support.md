## Language support

### Syntax highlighting

Tree-sitter grammars are compiled into the binary: no network access, no
grammar directory, and no runtime library loading. Adding a language means
adding a dependency and a row to `src/syntax/grammars.rs`.

#### Language detection

Runyte checks, in order:

1. an exact filename,
2. a registered filename prefix,
3. a case-insensitive extension,
4. a bounded first-line shebang.

| Language name | Detected files include |
| --- | --- |
| `bash` | `.bashrc`, `.bash_profile`; `sh`, `bash`, `ebuild`, `eclass` extensions; `sh`, `bash`, `dash` shebangs |
| `cmake` | `CMakeLists.txt` |
| `make` | `Makefile`, `makefile`, `GNUmakefile` |
| `lua` | `lua` shebangs |
| `dockerfile` | `Dockerfile`, `Containerfile`, their lowercase names, dot-suffixed variants such as `Dockerfile.dev`, and `.dockerfile` / `.containerfile` |
| `kotlin` | `.kt`, `.kts` |
| `jsonc` | `.jsonc`, `tsconfig.json`, `jsconfig.json`, `devcontainer.json`, and `.devcontainer.json` |
| `xml` | `.xml`, `.svg`, `.xsd`, `.xsl`, `.xslt`, `.wsdl`, `.xaml`, `.csproj`, `.fsproj`, `.vbproj`, `.props`, `.targets`, `.resx`, and `.plist` |
| `hcl` | `.hcl`, `.tf`, and `.tfvars`, including `terragrunt.hcl` and `production.auto.tfvars` |
| `ruby` | `.rb`, `.rake`, `.gemspec`, `.ru`, `Gemfile`, `Rakefile`, `Guardfile`, `Vagrantfile`, `Brewfile`, `Podfile`, `Fastfile`, `Appfile`, `.irbrc`, and `.pryrc`; `ruby` and `jruby` shebangs |
| `php` | `.php`, `.phtml`, `.php3`, `.php4`, `.php5`, `.php7`, `.php8`, and `.phps`; `php` shebangs |
| `elixir` | `.ex` and `.exs`, including `mix.exs`, `config.exs`, and `.formatter.exs` |

Terraform's `.tf.json` files keep JSON highlighting.

#### Notes by language

**INI** parses both `;` and `#` comments. `toggle-comments` inserts `;`.

**JSONC** is JSON with `//` and `/* */` comments, and uses the JSON grammar.
`toggle-comments` inserts `//`; in plain JSON it changes nothing.

- Language servers see `jsonc` as the language, so configure one under
  `lsp.jsonc`; a server under `lsp.json` does not attach to JSONC files.
- Only the exact file names in the table are JSONC. Detection sees the file
  name, not its directory, so `.vscode/settings.json`, `tsconfig.base.json`,
  and other `.json` files read as JSONC by their tools stay JSON.
- Trailing commas are not part of the grammar. A file with one still
  highlights, but its tree has a parse error and some of its folds are
  missing.

**Dockerfile** — the language name is `dockerfile`, including in Markdown
fences.

- Instructions, comments, strings, build options, and ports are highlighted.
- Shell commands and `RUN` heredoc bodies use Bash highlighting, within the
  injection size limit. Shell highlighting assumes Bash, even when a file
  selects another interpreter with `SHELL`.

**Kotlin** supports Kotlin 2 multi-dollar strings and guarded `when`
branches. The pinned grammar does not yet model context receivers: it parses
`context(Logger)` as a separate call before the function, so a function text
object starts at `fun` and deliberately leaves out the context-receiver
prefix.

**XML** uses its own parser, not the HTML one, including XML declarations,
entities, and CDATA.

**HCL** highlights blocks, expressions, interpolation, template directives,
and heredocs. HCL fences can use `hcl` or `tf`.

**Ruby** tells local bindings from method calls, including loop, exception,
and pattern bindings. Class/module bodies and methods isolate their locals.
Limitation: in `object = Object.new; def object.greet; end`, the receiver
`object` can take function colouring, because the local-scope query covers the
whole method.

**PHP** highlights code inside `<?php` or `<?=` tags and injects HTML into the
surrounding template, including JavaScript and CSS in script/style elements.

- Heredoc and nowdoc labels naming a bundled language, such as `SQL` or
  `JSON`, highlight their body in that language; unknown labels keep string
  highlighting. Inside an injected heredoc, the embedded language decides the
  colours, so interpolations such as `$id` may look like strings.
- PHPDoc comments keep ordinary comment highlighting.
- PHP snippets in Markdown fences need an opening PHP tag.

**Elixir** highlights offline, without an Elixir runtime or language server.
An `elixir` Markdown fence uses the same grammar within the injection limit.
EEx and HEEx templates are not registered as Elixir, and no Elixir language
server is bundled or configured.

**Missing structural queries.** Dockerfile, XML, HCL, Ruby, PHP, and Elixir
have no dedicated text-object, outline, indentation, or fold queries. In
Elixir, ordinary syntax-tree selection works, but function, class, and
parameter text objects, outlines, syntax indentation, and folds do not.

#### Large files

- Files above 128 KB are highlighted without language injection, so embedded
  languages such as fenced code in Markdown are highlighted only in smaller
  files.
- Markdown's inline grammar is also an injection. Above the limit, headings,
  lists, quotes, and other block structure keep their colours, while emphasis,
  strong text, links, and inline code use the ordinary foreground. The
  injection query still runs over the whole document on every edit, which is
  why this limit remains.
- There is no line or byte limit on highlighting itself. A slow parse delays
  the new tree, not the keystroke that asked for it.

#### Background parsing

Parsing and reparsing run on a background worker.

- A document is readable and editable as soon as its contents load, in
  ordinary text colours until its first syntax tree arrives. Highlighting then
  appears without replacing text or moving the cursor or viewport.
- Opening files, undo/redo, reload, and language changes all use this path.
- If typing outruns the parser, one pending request per buffer is replaced
  rather than queued.
- After the first tree, the previous tree stays in use during updates and its
  visible highlight spans are shifted through pending edits, so colours stay
  visible.
- Completed trees are applied between frames, and only when their parse
  generation, language, and text revision match the live document.
- A parse failure leaves the document editable as plain text; see
  `:service-health`.

**Structural features wait for a current tree.** Outline, folds, matching
brackets, text objects, and structural selection cannot use stale offsets.

- `Space x` commands and `mm` are dimmed while syntax is pending. Invoking one
  reports `Syntax is still parsing`; it is not queued.
- Smart newline falls back to ordinary indentation and list handling.
- Language detection, comments, editing, search, saving, and language servers
  do not need a syntax tree.

### Document preview

In the desktop window, `:preview` captures selected text, or the
whole current buffer when the selection is a bare caret. It includes unsaved
edits and leaves the source text and selection intact. Multiple nonempty ranges
are joined in document order with a newline. A selected SVG diagram is rendered
graphically even inside a Markdown or source document.

Desktop packages include preview in the editor executable, including inside
local macOS app bundles. The Linux x86-64 and ARM64 desktop release archives require glibc
2.39 or newer and the native runtime dependencies; the terminal archives and
curl installer remain separate. See the [desktop packaging guide](../../contrib/packaging/README.md).
For a checkout build, use the [preview instructions](../../crates/runyte-preview/README.md).
Markdown uses proportional GitHub-like typography; static HTML and SVG render
graphically. JSON is pretty-printed when valid and retains its original text
with a diagnostic otherwise. YAML retains its comments, structure and ordering.
Source code uses available syntax highlighting; other text uses monospace.

Generated pages — Markdown, JSON, YAML, source and plain text — take the active
theme's colours: its background and text, its link and heading colours, and
its syntax colours for highlighted code. Switching theme or the setting
repaints an open preview. Set `editor.preview_theme_colors` to `false` to keep
the original light page. Static HTML and valid SVG keep the appearance they
were written with; HTML that styles itself through `prefers-color-scheme`
follows whether the theme is dark or light.

Wheel/trackpad scrolling preserves fractional pixels; Shift-wheel pans horizontally.
The preview also uses the built-in PDF navigation keys: `j`/`k` and arrows,
`h`/`l`, page/half-page controls, `gg`/`ge`, and `+`/`-` zoom (`z1` resets).
Left-drag selects text; `y` or Ctrl-Shift-c copies it. Escape dismisses an overlay
first, then returns to source; `q` also returns to source. A dismissed preview
stays closed when switching workspaces or reattaching to its persistent session.
Invoke `:preview` again to refresh. Links expose their destination without
navigating. Scripts and network fetching are disabled. Local raster images must
be beneath the source document's directory. Captures are limited to 128 KiB;
selecting a smaller section of a larger file is supported. The terminal frontend
reports that this command needs `--window`. See the preview instructions for
resource limits, lifecycle and tested static CSS/SVG support.

### Rendered Markdown

`?` in a Markdown document opens it as a page to read; `?` again returns to
the source. `:render` (also `:markdown`) does the same.

| Source | Rendered page |
| --- | --- |
| `**strong**` | **strong** (bold) |
| `*emphasis*` | *emphasis* (italic) |
| `~~struck~~` | crossed out |
| `# Heading` | Heading without hashes; the first level underlined with a rule |
| `- item` | `•`, then `◦` and `▪` by depth |
| `1. item` | Keeps its number |
| `- [ ]` / `- [x]` | `☐` / `☑` |
| `> quote` | Marked in the margin with `▌` |
| `---` | A rule |
| A table | Re-aligned around `│` |
| A fenced code block | Indented, without fences |
| `[text](url)` | The text followed by its destination, underlined |
| YAML front matter | Kept without its `---` fences, in the dimmed colour |

Anything Runyte does not recognize is passed through as written.

**Moving between the two:**

- Both directions move the cursor to the corresponding text and bring it into
  view.
- On markup removed by rendering, the page uses the nearest surviving text.
  Toggling straight back restores the exact source position; moving around the
  page first takes you to that new place in the source.
- `g f` on a link or image label opens its destination, as in the source.
  Relative paths resolve beside the source document and at the project root.
- `Space e` opens the source document's directory and selects its file.

**The page is a buffer.** It is a generated read-only buffer beside the
document, not a mode the document is in, so both stay open and the source
keeps every editing key.

- Nothing is concealed. The markers are absent because they were never
  written into the page, so no offset moves and rows and columns mean what they
  say.
- Search, selections, splits, and yank work on the page, and find the text
  actually on screen.
- It is rendered from the buffer, not the file, so it shows unsaved work.
  Rendering the same document again reuses and regenerates its one page.
  Returning to the page through `Ctrl-o`/`Alt-o`, the buffer list, or the
  previous destination regenerates it too when the document has changed,
  keeping the caret on the same text.

**Each pane remembers which you left.** Opening a document by name shows it
the way that pane last showed it: if you left it on its page, you come back to
the page; if you left it in the source, you come back to the source.

- This applies to `:open`, `:split`/`:vsplit` with a path, choosing the
  document by name in the Finder (`Space f`), the directory tree, explorer
  entries, Git changed-file rows, `gf` on a file, and links to
  `file.md#heading`, which land on the page's heading. Native launch targets
  without a position follow this memory too. The page's own
  `[rendered]` row in the Finder always opens the page.
- A request for a place to edit still opens the source: a Finder content
  match, a diagnostic, a definition or reference, a native launch target with
  a position, and any host or `--wait` open request.
- The buffer list keeps one row for the document and one for its page, and
  each row opens exactly that.
- Panes remember separately, so one can keep the source while another reads
  the page. A split starts with what the pane it came from remembered.
- The page lives as long as its document. Closing the document closes its
  page, and the retention limit for hidden generated buffers never removes
  it. Closing the page itself means the document opens as source again.

**The scratch buffer** renders too while `editor.scratch_markdown` is on — the
same option that makes `Space p r` refill it as Markdown. Its page is named
`[rendered scratch]`. With the option off, the scratchpad is plain text and
`?` is unbound there.

**Tables and soft wrap.** Rendered pages, including rendered scratchpads,
wrap by default independently of source buffers. Set
`editor.always_wrap_rendered_markdown: false` to follow the general
`editor.soft_wrap` setting controlled by `Space p s`. Opening or leaving a page
does not change the general setting. When the override is active, toggling
soft wrap still changes the general setting and the status-line arrow; the
feedback explains why the page stays wrapped. Comparison panes and the
large-line safety limit still prevent wrapping.

Rendered tables use the same effective wrapping setting as the rest of the page.

- Columns share the pane's text width, keeping short columns compact and
  wrapping longer cells independently.
- A row is as tall as its tallest cell. Vertical separators continue through
  its lines, and horizontal rules separate entries.
- Resizing changes only the visual layout. With soft wrap off, tables keep
  their natural widths.
- Columns keep a small minimum width. If the pane is too narrow for them all,
  the table scrolls horizontally; moving the caret into a hidden column
  brings it into view. Other text still wraps normally.
- Up/down follows visual rows and skips added rules. In a shorter cell's blank
  continuation, it moves to the nearest cell with text on that row.
- Clicks on padding place the caret at nearby text; added rules are not click
  targets.
- Search and yank use the logical text: wrap breaks, padding, and added rules
  are not copied. A selection across cells follows that order, so it need not
  be a rectangle on screen.

**Attributes and colours.** Bold and underline are drawn by every terminal,
italic by most. Each role also has a theme colour, so a terminal that ignores
an attribute still shows the difference. The roles are the `markup.*` entries
in [Syntax colours](themes.md#syntax-colours), so a theme can recolour them.

### Language servers

#### Setting up a server

`rust-analyzer` is configured out of the box. For another language:

1. Install the server so it is on `PATH`.
2. Add it under `lsp` in the configuration file.
3. Run `:config-reload`.
4. Make sure the workspace allows language servers: answer the prompt shown
   when it first opened, or run `:lsp-trust` (see
   [Permission to run servers](language-support.md#permission-to-run-servers)). A reload does not
   ask again.

```yaml
lsp:
  enable: true
  markdown:
    command: marksman
    args: ["server"]
```

| Field | Meaning |
| --- | --- |
| `command` | Executable name or absolute path |
| `args` | Optional list of arguments. The server is started directly, not through a shell. |
| `initialization_options` | Optional; passed verbatim as `initializationOptions` |
| `roots` | Optional list of file names that mark a project root, such as `[pyproject.toml]`. Omitted, the language's built-in markers apply; `[]` roots every server at the workspace. See [Server roots](language-support.md#server-roots). |

**Language keys** are Runyte's language names, so a buffer's language is the
same question for highlighting and for LSP: `rust`, `python`, `swift`, `c`,
`cpp`, `javascript`, `typescript`, `tsx`, `html`, `css`, `go`, `bash`, `java`,
`kotlin`, `json`, `jsonc`, `sql`, `lua`, `c-sharp`, `zig`, `cmake`, `proto`,
`make`, `ini`, `toml`, `yaml`, and `markdown`. Other keys under `lsp` are
rejected, apart from `enable` and the `servers` wrapper.

- `lsp.servers.<language>` is an accepted compatibility spelling of
  `lsp.<language>`, which is preferred.
- Copy-ready examples for the servers in Runyte's compatibility tests are in
  [docs/lsp/](../lsp/README.md).
- Server definitions are YAML-only. `Space o o` can only turn LSP on or off as
  a whole.
- `:help lsp` repeats this setup inside the editor.

**Reloading and restarting:**

| Command | Effect |
| --- | --- |
| `:config-reload` | Adopt new definitions; restart only languages whose definition changed |
| `:lsp-restart [language]` | Restart a stopped server from the configuration already loaded, or every server of the named language. Each document's [root](language-support.md#server-roots) is found again. It does not re-read YAML, so it is the wrong command after editing the file. |
| `:lsp-status` (`Space l ?`) | Report servers that started or failed. The report replaces the command's echo while that echo is still current; `:not` keeps the full report. |
| `:service-health` | Show whether the active document has a configured and attached server |

`lsp.enable` is decided at startup. To turn LSP on or off, reopen an ide-mode
Runyte, or restart a persistent session with
`runyte --session-restart [WORKSPACE]` (passing the same `--config PATH` if
the host used a non-default one).

**Failures cost only the language features.** A server that is missing,
crashes, or answers slowly never makes the editor wait. Diagnostics from a
stopped server are dropped rather than left as stale claims. A launch error
appears in `:lsp-status` after the first start attempt and in the
notification center.

#### Server roots

A workspace can hold several projects for one language: a Rust crate in one
directory and another beside it, or a Python package inside a repository
whose root holds something else. Each document's server is started at the
root of the project the file belongs to, so the server reads that project's
`Cargo.toml` or `pyproject.toml`. One server runs per language and root.

The root is found the way Helix finds it. Runyte walks up from the document's
directory to the workspace root and takes the **top-most** directory holding
one of the language's marker files, so a member of a Cargo workspace is served
from the Cargo workspace rather than from its own crate. A document with no
marker above it, or one outside the workspace, uses the workspace root.

| Language | Built-in markers |
| --- | --- |
| `rust` | `Cargo.toml`, `Cargo.lock` |
| `python` | `pyproject.toml`, `setup.py`, `poetry.lock`, `pyrightconfig.json` |
| `go` | `go.work`, `go.mod` |
| `javascript` | `package.json`, `jsconfig.json` |
| `typescript`, `tsx` | `package.json`, `tsconfig.json` |
| `swift` | `Package.swift` |
| `java` | `pom.xml`, `build.gradle`, `build.gradle.kts` |
| `kotlin` | `settings.gradle`, `settings.gradle.kts` |
| `lua` | `.luarc.json`, `.luacheckrc`, `.stylua.toml`, `selene.toml`, `.git` |
| `zig` | `build.zig` |
| `markdown` | `.marksman.toml` |
| `dockerfile` | `Dockerfile`, `Containerfile` |
| `php` | `composer.json`, `index.php` |
| `elixir` | `mix.exs`, `mix.lock` |

Other languages have none, and their servers run at the workspace root.
`roots` in a server definition replaces the list; each entry is a single file
or directory name.

- `:lsp-status` names a server rooted below the workspace with its directory,
  as in `rust (tools): rust-analyzer (ready)`.
- A root is looked up once per directory. A marker file created later takes
  effect after `:lsp-restart` or `:config-reload`.
- Permission covers the whole workspace, so every root inside it shares the
  decision below.

#### Permission to run servers

The first time a workspace opens with LSP enabled, Runyte asks whether
language servers may run there.

| Choice | Lasts |
| --- | --- |
| **Keep LSP disabled** (default) | — |
| **Allow LSP once** | Until the ide-mode editor or persistent host stops |
| **Always allow LSP** | Remembered for this exact workspace |
| Escape | Dismisses without changing permission; on a first visit LSP stays off |

Editing, search, and highlighting work either way.

**Why it asks.** Language servers can run project code — build scripts,
plugins — with your permissions. Approval covers every configured server,
custom ones included, and future code changes in the workspace. It is
permission to run those tools, not a security review or a sandbox.

**Changing your mind.** `:lsp-trust` reopens the choices, including
revocation.

- Disabling LSP there stops this host's servers and clears their editor state.
  It cannot undo code already run.
- A persistent host owns the permission, so detaching and reattaching cannot
  bypass it. Other running editors keep their own decision until changed or
  restarted.
- Setting `lsp.enable: false` suppresses the question and keeps approval from
  starting LSP.

**Where decisions are stored.** Private `lsp-trust/` storage under Runyte's
per-user platform cache, separate from `.runyte/`.

- Clearing that cache makes Runyte ask again.
- A workspace rooted at your home directory may use Runyte's standard cache
  under it; other project-local cache overrides are rejected.
- Records use the canonical workspace root: symlinked aliases share a
  decision, but nested workspaces, other worktrees, and clones elsewhere do
  not.

**When storage fails:**

- An unreadable decision keeps LSP disabled and asks again.
- If storage is unavailable, the prompt explains why and offers only **Keep
  LSP disabled for now** and **Allow LSP once**.
- A failed save keeps the prompt open with the error and those temporary
  choices.
- **Keep LSP disabled for now** stops this editor's servers without changing
  any remembered decision.
- **Allow LSP once** needs no new storage, but first removes an older
  remembered decision; if that fails, the choice stays pending. The next
  launch asks again.
- Reopening `:lsp-trust` retries the store once it is repaired.

#### What a server is asked

Runyte only sends requests for capabilities the server advertised in its
handshake. Hover, completion, signature help, goto (definition, declaration,
type definition, implementation), references, document and workspace symbols,
rename, code actions, and formatting are gated independently.

- Asking for one the server never advertised reports it as unavailable on the
  interaction line. It is not kept as a notification, since it is expected
  rather than a fault. Typing near a trigger character a server does not
  support costs no round trip.
- A `Method not found` from a server that *did* advertise the capability is a
  protocol violation, reported and kept as an ERROR.

**Trigger characters** come from the server too:

| Server | Completion after | Signature help |
| --- | --- | --- |
| clangd | `/`, `"` | Seven delimiters |
| Pyright | `[`, `"` | Among its characters, names `)` but answers nothing there, closing the popup |
| rust-analyzer | `'` | |
| gopls | | Two delimiters |
| sourcekit-lsp, Marksman | | None |
| A server advertising the capability but naming no characters | `.`, `:` | `(`, `,` |

- The table names only the characters discussed here, not each server's full
  list; a blank cell says nothing.
- Some servers name the closing `)`, so the inner `)` of `f(g(a), b)` asks
  again instead of dismissing the popup; clangd and
  typescript-language-server answer with the enclosing signature.
- A server may name retrigger characters, active only while a popup shows.
  Runyte advertises the `contextSupport` that allows them and tells the server
  which character asked and whether a popup was open.
- A server naming `)` neither way has the popup closed locally.
- `tests/lsp_real_servers.rs` records what each server in the compatibility
  matrix does.

#### Edits from a server

- Servers may not create, rename, or delete files. Such operations in a
  workspace edit are reported and skipped, and files a rename touches are
  opened as buffers rather than written behind your back.
- Text edits must name absolute local files inside the project and exact,
  forward-ordered character boundaries. A malformed, remote, out-of-range, or
  overlapping edit rejects the whole multi-file change.
- Open target buffers are checked against the revisions the request was made
  from, including targets other than the source buffer.
- Versioned diagnostics are ignored once their document has moved on.

#### Compatibility tests

An opt-in Docker matrix tests real servers: Python (Pyright), Swift
(SourceKit-LSP), C and C++ (clangd), JavaScript (typescript-language-server),
Go (gopls), Rust (rust-analyzer), and Markdown (Marksman).

```sh
tests/lsp/run.sh
```

- The image pins every server toolchain. Each test uses Runyte's production
  stdio transport for the full handshake, in a disposable project.
- After initial symbol and definition checks, each sends an unsaved
  incremental change with non-ASCII text and resolves a definition that exists
  only in the changed document.
- Fixtures then exercise completion, hover, signature help, references,
  rename, formatting, code actions, and diagnostics, as far as each pinned
  server meaningfully implements them. `tests/lsp_real_servers.rs` marks each
  feature as tested, advertised-only, or unsupported per server.
- Rust, Go, and JavaScript also resolve definitions across files.
- Diagnostic fixtures check the reported range and the publication that clears
  it after a fix.
- Ordinary `cargo test` skips the matrix: building its image downloads several
  large toolchains, and running it starts eight real servers.

### Language features

| Key | Action |
| --- | --- |
| `Space l h` | Show documentation for the symbol under the cursor |
| `Space l s` / `Space l S` | Document / workspace symbols |
| `Space l d` | Diagnostics |
| `Space l r` / `Space l a` | Rename symbol / apply a code action |
| `Space l c` | Ask for completions (`Ctrl-x` in Insert mode) |
| `Space l f` / `Space l R` / `Space l ?` | Format / restart language servers / report language-server state |
| `Space l g d/D/y/r/i` | Definition / declaration / type definition / references / implementation |
| `gd` / `gD` | Go to definition / declaration |
| `gy` / `gi` | Go to type definition / implementation |
| `gr` | Go to references |
| `:format` | Format the buffer (typed equivalent of `Space l f`) |
| `Tab` | Code actions for the selection |

**Hover documentation** stays anchored to its source.

- A short document is a peek: it closes and passes the next key on.
- With more than twelve lines, the title says how much was left out, and Enter
  opens the full text in a retained read-only `[documentation]` buffer.

**Goto and pickers.** One result moves the selection to it; several open a
picker. Every picker filters as you type, moves with the arrows or
`Ctrl-n`/`Ctrl-p`, opens with Enter, and closes with Escape.

**Undo.** A completion that needs an import applies both edits as one undo
step, and so does a rename across a file. Everything typed between entering
Insert mode and returning to Normal is one undo step too; undo and redo map
the caret through the inverse edit rather than leaving it at a stale offset.

#### Completion

Three sources share one popup:

| Source | Appears | Needs |
| --- | --- | --- |
| Language server | After one of the server's trigger characters, on `Ctrl-x`, or with `Space l c` | A server |
| Path | When the text before the caret is a valid directory followed by part of a name | Nothing |
| Word | When a typed prefix reaches `editor.word_completion_minimum` characters (3) | Nothing |

| Key | Effect |
| --- | --- |
| `Tab` | Accept |
| `Escape` | Dismiss |
| Enter | Always inserts a newline and dismisses the popup |

**Why Enter never accepts.** A popup can open on its own, after a trigger
character or any three-character word prefix, so Enter always keeps its usual meaning
rather than risking an unwanted candidate on a keystroke meant to end a line.

**Escape** dismisses the popup and returns to Normal mode in one press for
automatic word completion, and in an editable explorer, where a row's `/` can
open path completion unasked.

**Language-server completion:**

- An explicit `Ctrl-x` includes the identifier already before the caret, uses
  the server's `filterText` and `sortText` when present, and stays the active
  source until space, newline, acceptance, or dismissal.
- Typing punctuation does not hand the session to word or path completion. A
  trigger character refreshes the candidates for the new context.
- If the prefix has no matches, the popup hides without ending the session,
  so Backspace can bring matches back. Backspace and Delete keep the session;
  moving the caret or another editing command ends it.
- `Ctrl-x` replaces a word popup as soon as the request is sent.

**Path completion:**

- It opens on any keystroke that leaves a valid directory plus a name
  fragment before the caret, not only on `/`, so editing an existing path
  shows hints just like typing a new one.
- Relative paths resolve against both the active file's parent and the
  project root, so `dir/`, `files/`, `./files/`, and `../files/` work from
  whichever makes them valid.
- Directories keep a trailing `/`; accepting one offers its children.
- A large directory shows its first few hundred names, but typing more narrows
  against the whole directory, so every existing name is reachable.
- A path in progress takes over from word completion as soon as `/` is typed,
  unless an explicit LSP session is active.
- Command palette path arguments work the same way.

**Word completion** offers words from every open buffer in the workspace,
including the one being edited.

- Every open buffer contributes, including explorers, whose entries are file
  names.
- It appears in file buffers, the scratch buffer, and the commit message.
- A word is Unicode letters and numbers. A hyphen stays inside only when it
  joins characters on both sides, so `up-to-date` stays whole. Other
  punctuation is a boundary and never included.
- Words from the current buffer come first, ordered by how often they occur
  there, then words from every other buffer in the same order. The word being
  typed is never offered to complete itself.
- A background index keeps the list, so a candidate can be one keystroke
  stale, but typing never waits for it.
- It never overrides `Ctrl-x` or a path in progress.
- Turn it off, or change the trigger length, with `editor.word_completion`
  and `editor.word_completion_minimum` in `Space o o`.

<!-- navigation -->
[Contents](README.md) · [Previous: Git](git.md) · [Next: Workspaces and persistent sessions](workspaces-and-persistent-sessions.md)
