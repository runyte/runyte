---
title: "Space ? in a plugin view opens generic text help instead of the view's workflow"
status: resolved
reported: 2026-09-20
resolved: 2026-09-24
commit: 648ff3e
---

## Resolution

Commit `648ff3e` (Add negotiated plugin contextual help) added the optional
`runyte-1` feature `view-help`. Previously `HelpTopic::for_context` in
`src/help.rs` mapped `BindingScope::Plugin(_)` to `HelpTopic::Text`, and the
wire contract had no way for a plugin to supply help prose.

Registration may now carry `help_topics`: at most 16 `{id,title,paragraphs}`
topics, each with a title of up to 64 bytes and 1–16 paragraphs of up to
2,048 bytes, with no control characters and at most 64 KiB in total. A view
model or header selects one with `help`. Topics belong to one registration, so
a view model can switch topics as a single view moves between pages, as
ru-dbviewer's browser does. Topics are validated in `src/plugin/help.rs`.
They are retained in `plugin::help::Registered`, together with the
application name, only when topics exist, and are charged to the owner's
retained payload. `ActionPolicy` in `src/workspace/host/plugin_models.rs`
rejects `help` without the feature (`unsupported`) and an unregistered topic
(`invalid_argument`) on every publication path. The registration envelope
check in `application::decode` now counts the new field.

`App::plugin_help_page` in `src/app/plugin_views.rs` builds the page from the
live registries. The topic comes from the model the active pane shows. The
action list comes from `plugin_menu_commands`, which was extracted from
`open_plugin_actions`, so help and the Tab menu share labels, groups, hidden
callbacks and row-dependent availability. The renderer replaces only the
overview prose. It keeps the general-help pointer and mouse notes, adds an
**Application actions** section, and then prints the generated key tables. A
model without a topic, an unnegotiated plugin and an older host keep the text
overview. A running plugin still gets the generated action list. A stopped
plugin leaves the registry, so its readable view falls back to the plain
overview. Help remains the shared read-only help buffer, rendered as a
snapshot when opened.

Plugin-authored text is escaped by `key_spelling::escape_markers` before
key-marker resolution. Before this, a plugin command label or description
containing `{key:…}` reached `resolve_with_map`, whose failure help `expect`s,
so `Space ?` panicked in the host. `HelpDocumentWriter::reset_to_prose` clears
the document-wide key, heading and command colouring from plugin text,
including plugin descriptions in the key table; only the plugin's own
backticks mark code.

The plugin side is ru-dbviewer: `src/app/help.rs` covers Databases, Tables,
Rows, Record, Value, Schema, Filters, Review and Transactions. Its Rows page
lists Next page, Previous page and Page size from the live menu rather than
from the prose.

Tests: `src/plugin/tests/help_topics.rs` (bounds and streaming decode limits);
`src/workspace/host/tests/plugin_help.rs`
(`space_question_follows_the_models_registered_topic_and_live_actions`,
`help_topics_are_negotiated_validated_atomic_and_charged`,
`model_help_without_the_feature_is_unsupported`); `src/help.rs`
(`a_plugin_topic_replaces_the_overview_and_keeps_generated_sections`,
`a_plugin_view_without_a_topic_keeps_the_text_overview_and_lists_actions`);
`src/key_spelling.rs` (`escaped_text_resolves_to_itself`);
`docs/plugins/check_schema.py` (`test_view_help_shape`) and
`docs/plugins/check_compatibility.py`
(`test_help_topics_are_sent_only_to_hosts_that_offer_view_help`).

Known limitation: There is no plugin help entry point outside a plugin's own
views. Help does not follow later publications; reopen it after the view
changes. Prose is plain text apart from backtick code spans and cannot refer to
actions or keys symbolically. ru-time does not yet supply topics.

## Report

`Space ?` in a `ru-dbviewer` view opened the generic text-help overview rather
than help describing the database workflow. Registered plugin bindings could
appear in the generated key table, but there was no plugin-authored contextual
help page. In `src/help.rs`, `HelpTopic::for_context` mapped
`BindingScope::Plugin(_)` to `HelpTopic::Text`.

To reproduce: enable `ru-dbviewer`, connect to a database, open its tables or
rows view, and press `Space ?`. The overview described ordinary text editing
instead of table browsing, filtering, paging, record inspection and return
navigation. Repeating from a record or value view gave the same overview, with
no explanation of that view's workflow.

Expected: `Space ?` opens contextual help for the active plugin view. For
dbviewer, Tables, Rows, Record and Value each need their own explanation. Rows
help should make actions such as `Tab → Next page`, `Previous page` and
`Page size` discoverable. Other plugins, including `ru-time`, should be able to
supply their own workflow explanations through the same public mechanism.

Constraints: this needs coordinated Runyte and plugin changes. Runyte must
provide a bounded, negotiated extension to the public `runyte-1` contract for
plugin-authored help and route contextual help to the active view's content.
Plugins supply the prose and the view association. Older hosts and plugins
that do not negotiate the extension must keep a usable fallback. Plugin help
must not depend on private bundled-client DTOs or hard-coded plugin
identities.

Help keeps Runyte's ordinary read-only, searchable, scrollable and splittable
document behavior. Action labels, available actions and configured key
spellings come from the live command/view and keymap registries, so execution,
menus, help and hints stay consistent. Plugin prose explains workflows without
introducing a second hand-maintained action or key table. The general manual
opened by `:help` keeps its existing role.

Left open by the report: the exact wire shape, content limits, help-document
identity and refresh behavior, and handling of plugin stop or restart. Whether
plugins also need a general help entry point outside their own views was
undecided.
