// SPDX-License-Identifier: MPL-2.0

use super::App;
use crate::input_grammar::InputGrammar;
use crate::{
    buffer::{Buffer, GeneratedViewIdentity},
    plugin::{application as api, view},
    selection::{Range, Selection},
    syntax::{Scope, Span},
};

impl App {
    pub(crate) fn plugin_selection_revision(&self, pane: usize) -> u64 {
        self.panes[&pane].selection_revision
    }
    pub(crate) fn plugin_set_selection(
        &mut self,
        pane: usize,
        ranges: Vec<crate::plugin::editor::Range>,
        primary: usize,
    ) {
        self.panes
            .get_mut(&pane)
            .unwrap()
            .replace_selection(Selection::new(
                ranges
                    .into_iter()
                    .map(|range| Range::new(range.anchor, range.head))
                    .collect(),
                primary,
            ));
    }

    pub(super) fn open_plugin_actions(&mut self) -> bool {
        let crate::keymap::BindingScope::Plugin(owner) = self.key_binding_scope() else {
            return false;
        };
        let commands = self
            .plugins
            .commands
            .values()
            .filter(|c| c.plugin == owner && c.context == api::CommandContext::View)
            .collect::<Vec<_>>();
        if commands.is_empty() {
            return false;
        }
        let buffer = self.active().buffer;
        let revision = self.buffers[buffer].revision();
        self.list = Some(super::ListPicker::fuzzy(
            "Application actions",
            commands
                .iter()
                .enumerate()
                .map(|(index, c)| {
                    super::PickerItem::new(c.local.clone(), c.description.clone(), index)
                })
                .collect(),
        ));
        self.list_actions = commands
            .iter()
            .map(|c| super::ListAction::PluginCommand {
                command: c.id,
                buffer,
                revision,
            })
            .collect();
        self.grammar.reset();
        true
    }

    pub(crate) fn create_plugin_view(
        &mut self,
        owner: usize,
        handle: &str,
        model: &view::Model,
    ) -> usize {
        let buffer = self.buffers.len();
        self.buffers.push(Buffer::virtual_text_identified(
            GeneratedViewIdentity::Plugin {
                owner,
                view: handle.into(),
            },
            model.title.clone(),
            "",
        ));
        self.syntax.push(None);
        self.publish_plugin_view(buffer, None, model);
        self.retire_detached_ephemeral_buffers();
        buffer
    }

    pub(crate) fn publish_plugin_view(
        &mut self,
        buffer: usize,
        old: Option<&view::Model>,
        model: &view::Model,
    ) {
        let text = model.text();
        let new_rows = model
            .rows
            .iter()
            .enumerate()
            .map(|(row, value)| (value.id.as_str(), row))
            .collect::<std::collections::BTreeMap<_, _>>();
        let row_map = |row: usize| {
            old.and_then(|old| old.rows.get(row))
                .and_then(|row| new_rows.get(row.id.as_str()))
                .copied()
                .unwrap_or_else(|| row.min(model.rows.len().saturating_sub(1)))
        };
        let before = &self.buffers[buffer];
        let captures = self
            .panes
            .iter()
            .filter(|(_, pane)| pane.buffer == buffer)
            .map(|(&id, pane)| {
                let positions = pane
                    .selection
                    .ranges()
                    .iter()
                    .map(|range| {
                        let point = |offset| {
                            let row = before.offset_to_row(offset);
                            (
                                row_map(row),
                                offset.saturating_sub(before.line_to_offset(row)),
                            )
                        };
                        (point(range.anchor), point(range.head))
                    })
                    .collect::<Vec<_>>();
                (
                    id,
                    positions,
                    pane.selection.primary_index(),
                    row_map(pane.scroll_row),
                )
            })
            .collect::<Vec<_>>();
        self.buffers[buffer].replace_plugin_projection(&text);
        if let crate::buffer::BufferKind::Virtual { name, .. } = &mut self.buffers[buffer].kind {
            *name = model.title.clone();
        }
        let after = &self.buffers[buffer];
        for (id, positions, primary, scroll) in captures {
            let ranges = positions
                .into_iter()
                .map(|(anchor, head)| {
                    let offset = |(row, column): (usize, usize)| {
                        after.line_to_offset(row) + column.min(after.line_len(row))
                    };
                    Range::new(offset(anchor), offset(head))
                })
                .collect();
            let pane = self.panes.get_mut(&id).unwrap();
            pane.replace_selection(Selection::new(ranges, primary));
            pane.scroll_row = scroll;
        }
        let mut offset = 0;
        let spans = model
            .rows
            .iter()
            .filter_map(|row| {
                let from = offset;
                offset += row.text.chars().count() + 1;
                let name = match row.role {
                    view::Role::Ordinary => return None,
                    view::Role::Muted => "comment",
                    view::Role::Heading => "markup.heading",
                    view::Role::Warning => "diagnostic.warning",
                    view::Role::Error => "diagnostic.error",
                };
                Scope::named(name).map(|scope| Span {
                    from,
                    to: offset - 1,
                    scope,
                })
            })
            .collect();
        self.generated_highlights.insert(buffer, spans);
        self.normalize_buffer(buffer);
    }

    pub(crate) fn present_plugin_view(&mut self, buffer: usize) {
        self.switch_buffer(buffer);
    }

    pub fn note_plugin_frontend(&mut self, attached: bool) {
        if self.plugins.frontend_attached != attached {
            self.plugins.frontend_attached = attached;
            self.plugins.attachment_generation += 1;
        }
    }

    pub(crate) fn plugin_foreground(
        &self,
        context: &api::CapturedContext,
    ) -> Result<(), api::Error> {
        if !self.plugins.frontend_attached {
            return Err(api::Error::new(
                api::ErrorCode::NoFrontend,
                "No attached frontend",
            ));
        }
        if !context.foreground_allowed
            || self.plugins.attachment_generation != context.attachment
            || self.plugins.foreground_generation != context.foreground
            || self.active_pane != context.pane
            || self.active().buffer != context.buffer
            || self.active().terminal != context.terminal
        {
            return Err(api::Error::new(
                api::ErrorCode::ContextChanged,
                "Invoking context changed",
            ));
        }
        Ok(())
    }
}
