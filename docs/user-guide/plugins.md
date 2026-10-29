## Plugins

Plugins are external programs that Runyte starts and talks to. The
[plugin guide](../plugins.md) is the full reference: a runnable example,
installation and configuration, command bindings, errors, subscriptions,
limits, and the stable API.

- There is no package manager and no automatic discovery. A plugin runs only
  when explicitly enabled in the configuration.
- Enabled programs run with your permissions. They are not sandboxed.

### Using plugins

| Key or command | Action |
| --- | --- |
| `::` | Open the plugin-only command palette, with completion and descriptions |
| `:plugins` | Open the plugin manager |
| `:plugin-stop <id>` | Stop a plugin, or cancel its pending restart |
| `:plugin-restart <id>` | Reconnect after cleanup finishes |
| `Tab` in an application view | Actions from its plugin |

**Plugin commands:**

- Plugins can declare short names, such as ru-time's `::time`, `::time-add`,
  and `::time-delete`. Full `:plugin.<id>.<command>` names always work.
- Conflicting short names are disabled for all claimants and reported in
  `:notifications`. Stopping a claimant restores a name once it is unique.
- Backspace over the second colon returns to the ordinary palette.
- Keys for plugin commands are set with `plugins[].bindings`.

**The manager** (`:plugins`) lists configured, disabled, and failed plugins.
Select one to see its capabilities, jobs, activity, helpers, and diagnostics;
Enter offers lifecycle actions.

- Restart does not replay old actions.
- Stopping keeps retained views readable and dirty provider documents
  editable.
- Persistent hosts keep plugin processes across detach and reattach.

### What plugins can do

**Edit text.** Explicitly enabled process plugins can register commands, read
the invoking buffer's text and selections, and return replacements as one
undoable edit. They run asynchronously; if the text changed meanwhile, the
stale result is rejected.

**Application views.** Plugins can provide retained native views, typed
commands, finite background jobs, explicit buffer and selection operations,
and bounded local filesystem operations with native confirmation.

- **Tab menus** can depend on the selected rows: a connected database can offer
  Disconnect while a disconnected one offers Connect. Multiple rows offer only
  actions common to all. Moving the selection after opening the menu requires
  reopening it, and changed models are checked again before an action runs.
  Other plugins keep view-wide menus.
- Actions can have readable labels and be grouped into named sections.
  Headings cannot be selected; typing filters across sections. Enter can open
  the selected item even when its callback is hidden from the menu.
- View titles identify the content, and plugins can place labelled facts,
  such as filters or a database path, above it.
- **Read-only documents** open on demand and scroll, search, and copy as one
  buffer, even when received in chunks. The supported limit is 8 MiB and
  250,000 lines; loading can fail while the host's shared memory allowance is
  in use. A shortened preview stays a preview until the full content is
  published.

**Prompts and forms.** Native prompts, filterable choices, and forms with
masked secret fields.

- A text or secret field rejects a whole paste containing control characters
  (including a trailing newline) or exceeding its byte or character limit.
  The value and cursor stay unchanged, and feedback names the cause.
- Required fields and minimum lengths are checked on submit.
- Fields can opt into asynchronous validation after Enter. The form stays open
  while you edit, stale results are rejected, and input never blocks. Secret
  values reach validation only when that field opts in.
- Text fields can opt into local path completion: type a directory or name
  prefix, choose with Up/Down, and press Tab. Directory suggestions continue
  into the directory. Shift-Tab returns to the previous field and Escape
  cancels. Enter completes the selected suggestion while the path does not
  name an existing entry, and submits once it does. Relative paths start at
  the workspace root; spaces and quotes are literal; `~` and environment
  variables are not expanded. Older hosts keep ordinary text fields.

**Documents.** Applications can create named unsaved documents and save them
asynchronously. Later edits stay dirty, and pending or uncertain writes
cannot silently complete a close or `--wait` request.

**Remote documents** (providers) open version-bound UTF-8 documents with
normal editing, highlighting, and remote saves.

- Providers with conditional writes save normally. Weaker providers need
  native confirmation describing the overwrite race and replacement
  guarantee. Cancelling keeps the text and undo.
- Newer edits stay dirty during uploads; save-and-close waits for confirmed
  success.
- An explicit rebind reconciles restarted providers and uncertain writes.
- `:diff-remote` compares fresh remote text with your edits without changing
  the saved baseline; both sides must fit the 4 MiB comparison limit.
- `:reload` reads fresh remote text. Dirty or uncertain documents offer reload
  (keeping your edits against a new baseline) or cancel. Reload is one
  undoable edit and keeps earlier history. Unknown writes need provider
  settlement before a baseline is accepted. Each side may hold at most 8 MiB.
  See [provider recovery](../plugins/applications.md#native-reload-and-conflict-recovery).

**Subscriptions.** Applications can subscribe to buffer, pane, owned
view/job, and attachment metadata, with consistent baselines and bounded,
ordered delivery. New buffers are reported without polling; slow consumers
get an explicit resynchronization marker.

**Helpers.** Applications can launch
[managed helpers](../plugins/applications.md#managed-helpers) with bounded binary
input and retained output, kept separate from the plugin protocol.

- Closing a helper, or stopping its plugin, cleans up and reaps the owned
  process tree. Output from a naturally exited helper stays readable until
  released.
- Idle enabled helpers alone do not prevent persistent host retirement.
- The runnable helper controller shows Send, Flood, EOF, and Close actions
  without a network service.

**Notifications and handoffs.** Applications can publish owner-labelled
notifications without changing focus or action feedback. Explicit native
handoffs can open a terminal session, an HTTP/HTTPS URL in the browser, or a
workspace file with its system handler. A terminal handed over survives
plugin stop and follows normal terminal retention. See the
[handoff example](../plugins/applications.md#notifications-and-native-handoffs).

**Activity leases.** Continuing playback or service work can hold a renewable
[activity lease](../plugins/applications.md#continuing-activity) of at most ten
minutes per grant.

- Session health and `:service-health` show its owner, title, and state.
- Active leases, and their two-second cancellation grace, protect against
  normal quit and idle retirement. Detach keeps them running.
- An owner that does not acknowledge expiry or cancellation is stopped, with
  its managed helpers cleaned up.
- `:q!` does not bypass this. Stop the owner in `:plugins` before quitting, or
  `:detach` in mux mode to leave it running. A forced persistent-session
  stop remains available.

**Settings and state.** Applications can read their configured settings and
save bounded, non-secret workspace preferences through the
[settings and state API](../plugins/applications.md#settings-and-workspace-state).
Updates check the revision the application saw and keep existing data on
conflict. The runnable `preferences.py` example shows a configured default
and a saved destination that survives restarting the editor. Secrets belong in
a credential manager or authentication helper.

### Example applications

The [application guide](../plugins/applications.md) has runnable examples and
lists remaining work:

| Example | What it shows |
| --- | --- |
| [SFTP browser and editor](../plugins/applications.md#sftp-browser-and-editor) | Remote files over SSH |
| [FTP/FTPS browser and editor](../plugins/applications.md#ftp-and-ftps-browser-and-editor) | The same workflow over standard-library FTP/FTPS |
| [Local media controller](../plugins/applications.md#local-media-controller) | Audio and video playback through mpv |
| [Todo showcase](../plugins/todo/README.md) | Matching Python, Rust, and C task-list applications |
| Task list, local file manager, document, memory provider, background jobs | Other runnable examples in the guide |

**SFTP.** Verifies SSH host keys, uses explicit identity files or an existing
SSH agent, and opens remote UTF-8 documents up to 8 MiB. Run `::sftp .` with
its documented profile; Enter opens a file, and `:write` confirms the remaining
remote overwrite race.

**FTP/FTPS.** Defaults to certificate-verified FTPS with encrypted data
connections. Plain FTP is an explicit profile choice and is labelled in the
UI. Saves need native confirmation of non-atomic replacement, and credentials
come from an explicit private file.

**Both remote browsers:**

| Action | Flow |
| --- | --- |
| `download` | Asks for a new local path and stages up to 8 MiB of file bytes. When the browser shows `Download ready`, `confirm-download` opens the native review. Downloads never publish automatically; `cancel-download` cancels unaccepted work. |
| `upload` | Freezes the bytes of a workspace disk file of at most 8 MiB; `confirm-upload` opens native confirmation for the remote destination. Unsaved editor text is not the disk file. `cancel-upload` cancels pending work; an unknown promotion outcome stays visible and blocks another upload until an explicit plugin restart. See [binary uploads](../plugins/applications.md#binary-uploads-from-workspace-disk-files). |
| `mkdir`, `rename`, `delete` | Act on one remote file or empty directory. `confirm-operation` opens native confirmation; `cancel-operation` cancels. Deletion is permanent, checks are best effort, and an unknown outcome must be inspected before retrying. See the [remote operation guide](../plugins/applications.md#confirmed-remote-directory-operations). |

Remote identities stay separate from local files, and open remote documents
keep their identity and unsaved text.

**Media controller.** Plays workspace audio and video through an installed
mpv, with a native playlist: play, pause, relative seek, next, previous, and
stop. Video appears in mpv's own window. Playback continues across detach and
protects the persistent session while active; pausing releases that
protection, and stop closes the player. The
[service adapter guide](../plugins/media-services.md) covers Spotify
prerequisites and YouTube browser playback separately.

<!-- navigation -->
[Contents](README.md) · [Previous: Commands](commands.md) · [Next: Agent access to workspace context](agent-access-to-workspace-context.md)
