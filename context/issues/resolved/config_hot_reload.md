---
title: "Configuration edits made outside the settings interface are not reloaded"
status: resolved
reported: 2026-09-20
resolved: 2026-09-21
commit: 20bdfe9
---

## Resolution

Commit `20bdfe9` (`Reload configuration into a running editor`) added
`:config-reload`, a command-only identity in the Configuration category that
re-reads the path the running editor loaded, including an explicit
`--config PATH`. The editor half lives in `src/app/config_reload.rs`; the
configured-plugin half lives in `src/workspace/host/plugin_manager.rs`, which
owns the processes and the work in flight a restart would interrupt. In
persistent mode the host owns the editor state, so the command executes there
whether it is typed in a standalone editor or an attached client.

The report left three decisions open. The interaction is an explicit command
and no file watching: watching would have to account for atomic replacement and
would add idle cost to every session for something that happens by hand, while
a command is equally discoverable through the palette. The scope is the one
workspace the command runs in; another running session reloads when it is run
there. The treatment of service changes while work is active is described
below.

`Config::reload` reports an absent file rather than returning built-in
defaults, and `Config::read` is the one parse, merge and validation path both
it and startup use. `App::reload_configuration` refuses before mutating
anything when the file cannot be parsed, fails registry validation, or has been
deleted after being read, so a half-written file cannot break a live editor.
A path that never held a file is distinguished from one that was deleted by
`App::config_file_read`, which makes creating a configuration mid-session and
reloading it work without treating a deletion as harmless.

Settings read before the editor existed are reported rather than adopted.
`editor.mouse`, `lsp.enable`, `workspace.mode` and `workspace.state` keep the
values this process launched with, and the file's values still reach
`persisted_config`, so the settings page shows them as saved while the
effective column keeps naming what the editor is doing. That is the existing
effective-versus-saved split rather than a new concept, and the first three
match the `PreviewPolicy::RestartRequired` descriptors in `src/settings.rs`.
Everything else applies at once: the theme, the compiled keymaps, the editing
grammar, `notifications.history_limit`, and the explorer view, which is
refreshed only when one of the three settings `ListingView::from_config` reads
actually changed, because re-reading a listing costs it its remembered view and
jump history. The compiled maps carry the bindings of every plugin running at
the time, so the reloaded section has them laid back over it through
`plugin_keymaps`; a section that collides with one is refused on its own,
leaving the bindings in use and the keymap later rebuilds validate against
untouched.

A theme naming something that cannot be built keeps the theme already on
screen and reports it, instead of falling back to the built-in default the way
a fresh start does. Startup and reload compile the `keys` section through one
`compile_configured_keymaps`, so a binding accepted at one cannot be refused at
the other, and dispatch, help and key hints continue to read the single keymap
that `sync_keymap` selects.

`LspCommand::Reconfigure` replaces the manager's definitions. A language whose
command, arguments and initialization options are unchanged keeps its process,
handshake, open documents and diagnostics; only a changed definition is stopped
and started again on the next request, a removed one simply stops, and a
recorded launch failure is cleared for any definition that changed so a
corrected command is tried. That clearing happens after the stops as well as
before them: a graceful stop keeps draining every other language while it waits
for its own shutdown reply, so a server whose turn has not come yet can exit in
the meantime and be recorded as failed again, after which its own iteration
finds nothing to stop and would leave it failed for good. It is sent on every successful reload rather than
when the editor believes the definitions changed: the manager is the side that
can see which languages are running, and an unconditional send means a refused
message costs nothing but running the command again. A refusal is reported.

Configured plugins are reconciled by their `id`, so moving an entry is not a
change to the plugin it describes, and an entry whose configuration is
unchanged keeps its running process untouched. The values a reload computes
live in a `Reloaded` value rather than being written onto the published
`Entry`, which is what allows a change to be held back: a plugin holding a
running job, an activity lease, a helper process, outstanding cleanup, or an
unsaved or uncertain remote document keeps running exactly as it was, the
change waits on its record, and both the manager row and a notification say so.
The count is everything `stop_plugin` would cancel or orphan, which includes a
command invocation whose response has not arrived, accepted local filesystem
work, view-model preparation, an issued staging handle, and an open input
surface: a job and an activity lease are the two a plugin declares on purpose,
but nothing obliges it to declare one before doing something a restart would
ruin.
The wait ends by itself — `settle_deferred_plugin_entry` delivers the change as
soon as the protected work is gone — and `:plugin-restart` or `:plugin-stop`
only bring it forward. While a change waits, the entry keeps publishing the
running generation's `valid` verdict, because `:plugin-stop` is refused for an
entry marked invalid and stopping is exactly what a plugin whose new
configuration cannot be admitted needs. A parked change deliberately carries no
record position: it outlives the reconciliation that produced it, and both
pruning and a later reload renumber the manager.

A plugin the file no longer lists is stopped and its record stays visible until
its cleanup settles, so nothing claims a cleanup finished that has not. An
entry added to the file starts from the reload, and one that was disabled or
refused at startup can be corrected and started the same way, which is why
`start_plugins` now creates the plugin event queue whatever the configuration
enables. A reload that would exceed the configured-plugin bound is refused with
the running plugins left alone, rather than replacing the published manager
with the synthetic startup error row.

Tests: `src/config.rs` covers the loader seam in
`reloading_distinguishes_an_absent_file_from_an_unusable_one` and
`reloading_reads_an_atomically_replaced_file_as_its_new_contents`.
`src/app/tests/config_reload.rs` covers the editor half — adoption without a
restart, an invalid replacement leaving the working configuration untouched, an
unresolvable theme, startup-bound settings, the `keys` section reaching
dispatch and key hints together, rejected binding entries, the notification
limit, explorer gating, an unchanged `plugins` section asking the host for
nothing, the definitions handed to the language-server manager, a refused
reconfiguration, a deleted file, and a file created after startup.
`src/workspace/host/tests/plugin_configuration_reload.rs` covers the plugin
half — unchanged, changed, added, removed, reordered, disabled and invalidated
entries; deferral on a running job and on an unsaved provider document; a
deferred change settling by itself; stopping a deferred plugin; a removal taken
back; a parked change surviving renumbering; and the configured-plugin bound.
The plugin file also covers the two things a reload does to the keymap, in
`a_reload_keeps_the_keys_of_the_plugins_that_are_running` and
`a_keys_section_colliding_with_a_live_plugin_binding_is_refused_on_its_own`,
and the undeclared work in
`an_outstanding_request_defers_a_change_without_any_declared_work`.
`tests/lsp_client.rs` covers the manager against its mock in
`reconfiguring_an_unchanged_definition_leaves_its_server_running`,
`reconfiguring_restarts_only_the_language_whose_definition_changed`,
`a_removed_definition_stops_its_server_and_reports_it_is_unconfigured`,
`reconfiguring_clears_a_recorded_failure_so_the_new_command_is_tried` and
`a_server_that_exits_during_another_shutdown_still_takes_its_new_command`.

Known limitation: a reload reaches only the workspace whose editor ran the
command. Several running persistent hosts each need their own `:config-reload`;
nothing broadcasts a configuration change between them. An editor that rewrites
the file in place rather than replacing it atomically can be read mid-write,
which fails as invalid YAML and leaves the working configuration untouched; the
command simply has to be run again.

## Report

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

The interaction was left undecided between an explicit reload command, automatic
file watching, or both. The treatment of service changes while work is active, and
the scope of reload across multiple running workspace hosts, were also left as
design decisions. Automatic watching would have to account for atomic file
replacement and avoid adding unnecessary idle polling.
