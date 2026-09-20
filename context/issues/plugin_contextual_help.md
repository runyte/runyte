`Space ?` in a `ru-dbviewer` view opens the generic text-help overview rather
than help describing the database workflow. Registered plugin bindings can appear
in the generated key table, but there is no plugin-authored contextual help page.
In `src/help.rs`, `HelpTopic::for_context` maps `BindingScope::Plugin(_)` to
`HelpTopic::Text`.

To reproduce, enable `ru-dbviewer`, connect to a database, open its tables or
rows view, and press `Space ?`. The overview describes ordinary text editing
instead of table browsing, filtering, paging, record inspection, and return
navigation. Repeat from a record or value view: the overview still does not
explain that view's workflow.

`Space ?` should open contextual help for the active plugin view. For dbviewer,
Tables, Rows, Record, and Value need their own explanations. Rows help should
make actions such as `Tab → Next page`, `Previous page`, and `Page size`
discoverable. Other plugins, including `ru-time`, should be able to supply their
own workflow explanations through the same public mechanism.

This needs coordinated Runyte and plugin changes. Runyte must provide a bounded,
negotiated extension to the public `runyte-1` contract for plugin-authored help
and route contextual help to the active view's content. Plugins must supply the
corresponding prose and view association. Older hosts and plugins that do not
negotiate the extension must retain a usable fallback; plugin-specific help must
not depend on private bundled-client DTOs or hard-coded plugin identities.

Help should retain Runyte's ordinary read-only, searchable, scrollable, splittable
document behavior. Action labels, available actions, and configured key spellings
must come from the live command/view and keymap registries, so execution, menus,
help, and hints remain consistent. Plugin prose should explain workflows without
introducing a second hand-maintained action or key table. The separate general
manual opened by `:help` should retain its existing role.

The exact wire shape, content limits, help-document identity and refresh behavior,
and handling of plugin stop or restart remain design decisions. Whether plugins
also need a general help entry point outside their own views is undecided.
