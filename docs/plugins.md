# Plugins

Runyte plugins are explicitly enabled programs speaking the stable `runyte-1`
application protocol over stdin/stdout. Stable support starts with Runyte 0.3.0.
Python, JavaScript, Rust, C and other languages use the same public contract.

Start with the [authoring guide](plugins/authoring.md),
[application contract](plugins/applications.md),
[compatibility policy](plugins/compatibility.md) and
[conformance checks](plugins/conformance.md). The private bundled frontend
transport and headless testing facade are not plugin APIs.

External agents use the separately paired [workspace context profile](plugins/context.md).
It provides scoped reads, buffer edits and individually approved terminal text
proposals over a private Unix socket; ordinary plugin capabilities do not grant
access to that transport.

## Install and enable the example

Copy [uppercase.py](plugins/uppercase.py) and the maintained
[application.py](plugins/application.py) beside it. Python 3.10+ and its standard
library are sufficient. Replace these absolute paths in your configuration:

```yaml
plugins:
  - id: case
    enabled: true
    api: runyte-1
    runyte: ">=0.3.0, <0.4.0"
    executable: /absolute/path/to/python3
    args: [/absolute/path/to/plugins/uppercase.py]
    capabilities: [text, selections]
    bindings:
      uppercase: F12
```

Run `::uppercase` or press F12 in Normal or Select mode. The example reads the
invoking pane's native operative spans, checks its captured selection identity,
reads text at the captured buffer revision, and submits one atomic edit.
Unicode scalar offsets let `éß` become `ÉSS`; a reversed selection preserves
its native span. `u` undoes the complete edit. A changed selection before the
read or a changed buffer before the edit refuses the action without replay.
Each read and the total edit remain subject to the public text limits.

Executable paths are absolute and arguments are passed without a shell, tilde
expansion or variable expansion. The process working directory is the workspace
root. Runyte does not discover bundles, execute metadata probes or install
packages. Omitted/disabled entries start no process. Configuration is read by
its owning host; restart an existing persistent host after changing it.
`:plugin-restart <id>` uses the already loaded configuration.

The plugin-authored `runyte` range is copied into configuration for pre-launch
admission, then checked against the process's registration. A user may narrow
that range but cannot widen plugin support. Explicit `api` and valid `runyte`
metadata are required; experimental configuration must be regenerated.

## Plugin command names

The configured ID scopes full names as `plugin.<id>.<local-name>`, for example
`:plugin.case.uppercase`. Local names and configured IDs contain 1–48 lowercase
ASCII letters, digits or hyphens. `stop` is reserved for the host's stop action.
A plugin registers its commands once, atomically, together with configured
bindings and typed argument metadata.

An optional command `alias` supplies a short spelling after `::`, independently
of the configured ID. `::uppercase` and `:plugin.case.uppercase` address the same
command. Aliases use the same bounded spelling rules. The palette, help, key
hints and dispatch read one registry. Full names remain available when aliases
collide: Runyte disables the alias for every claimant, reports the conflicting
full names, and restores the alias when only one claimant remains. Built-in
`:write` and a plugin's `::write` are distinct namespaces.

With negotiated `view-action-presentation`, command labels can contain spaces
(`Add database`) while identifiers and aliases keep their existing grammar.
Plugins can group Tab actions and hide an internal Enter callback from discovery
without disabling it. See [action labels and groups](plugins/applications.md#action-labels-and-groups).

With negotiated `view-default-bindings`, a view command can declare a
`default_binding` such as `"-"`. An explicit entry in the host configuration
overrides it. Older hosts still require configured bindings. Defaults pass the
same scope and collision validation as configured bindings.

With negotiated `view-help`, registration supplies bounded workflow topics. A
view model selects one, and `Space ?` in that view opens it with the view's live
actions and keys. See [contextual help](plugins/applications.md#contextual-help).

Bindings use physical spellings, at most eight keys, in Normal and Select modes.
They cannot replace existing commands or grammar controls. Invalid commands,
arguments, aliases or binding collisions reject the whole registration.

## Protocol and behavior

Transport is UTF-8 newline-delimited JSON, one object per line, with a 1 MiB
encoded limit including the newline. Flush each outbound message; reserve stdout
for protocol messages. Child stderr is discarded, so debug plugin logic separately.
The [stable schema](plugins/runyte-1.schema.json) and
[fixtures](plugins/stable-fixtures.json) describe both message directions.

The host sends hello with its protocol, actual package release, supported
capabilities/features and limits. Registration supplies the authored release
range, commands and required/optional capabilities/features. Only a successful
registered acknowledgement makes commands available. Required permissions need
both host support and explicit configuration grants. Features select optional
wire behavior and do not grant authority.

Requests have correlated IDs and may finish out of order. Handles and revisions
are opaque, owned by one connection generation. Mutations address captured,
explicit targets and revisions; they never follow the newly active pane.
Malformed messages can disconnect; unknown methods return `unsupported`.
Applications must not replay mutations after uncertain outcomes.

The [application contract](plugins/applications.md) defines commands, native
views/input, text and document operations, providers, jobs, activity, helpers,
subscriptions, cancellation, quotas and cleanup. Schema-valid messages alone do
not establish ownership, freshness, authority or successful side effects.

A text plugin's registration looks like:

```json
{"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.4.0","name":"Uppercase","commands":[{"name":"uppercase","alias":"uppercase","description":"Uppercase every selection","context":"buffer"}],"required_capabilities":["text","selections"],"optional_capabilities":[],"required_features":[],"optional_features":[]}
```

## Lifecycle and trust

`:plugins` shows configured entries, effective compatibility, grants, activity,
cleanup and bounded diagnostics. `:plugin-stop <id>` stops an owner;
`:plugin-restart <id>` waits for cleanup and establishes a fresh connection.
On Windows, `processes` is advertised and negotiated like other host
capabilities. Native helpers use bounded binary pipes and a private job that
the host closes with the helper's descendants before acknowledging cleanup.
There is no automatic restart or replay. Persistent-session detach retains the
same workers and live resources. Stop removes commands and input surfaces,
leaves application views readable but unavailable, and preserves dirty provider
documents. Editor-owned terminal sessions survive their originating plugin.

Plugins run with the user's environment and permissions. Capability grants
control editor operations; process isolation is not a security sandbox.
Quotas bound host resources but do not limit arbitrary child CPU, memory, files
or network use. Runyte installs no runtime or package manager on a plugin's behalf.
EOF/forced termination cannot promise a final reply or a completed remote write.
