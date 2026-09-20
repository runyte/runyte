Configuration edits made outside the native settings interface are not reloaded
by a running Runyte editor. Plugins and language servers continue to use the
configuration captured at startup. `:plugin-restart <id>` and `:lsp-restart
<language>` restart their services using that loaded configuration; neither
re-reads the configuration file. In persistent mode, restarting the frontend
alone also leaves the host's configuration unchanged.

For example, a `ru-time` entry still declaring `api: runyte-experimental-2`
cannot load on the stable host. Correcting the entry to `api: runyte-1` and
adding `runyte: ">=0.3.0, <0.4.0"` currently requires restarting Runyte,
including the persistent session host when applicable, before the correction
takes effect.

To reproduce, start Runyte with an enabled plugin, edit that plugin's entry in
the loaded YAML configuration, save the file, then run `:plugin-restart <id>`.
The plugin restarts with its previous configuration rather than the saved entry.

Runyte should support reloading configuration without restarting the whole editor
or persistent session host. The native settings interface already applies some
settings live; external file edits should have a supported path to those same
effects. Reloading must use the originally loaded configuration path, including
an explicit `--config` path.

Parsing and validation must preserve the working configuration when a replacement
is invalid. Key dispatch, help, and hints must continue to agree after binding
changes. Plugin additions, removals, executable changes, compatibility declarations,
capabilities, bindings, and settings require coordinated lifecycle handling, as do
language-server changes. Reload must not silently interrupt plugin jobs, running
timers, pending database transactions, unsaved provider documents, or other active
work. Unchanged services should retain their state. Settings tied to startup,
such as `workspace.mode`, need an explicit restart-required result rather than a
claim that their new value is already active.

The interaction is undecided: an explicit reload command, automatic file watching,
or both. The treatment of service changes while work is active, and the scope of
reload across multiple running workspace hosts, also need design decisions.
Automatic watching must account for atomic file replacement and avoid adding
unnecessary idle polling.
