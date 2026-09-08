// SPDX-License-Identifier: MPL-2.0

use super::App;
use crate::input_grammar::InputGrammar;
use crate::{
    buffer::{Buffer, GeneratedViewIdentity},
    plugin::{application as api, view},
    selection::{Range, Selection},
    syntax::Span,
};

#[derive(Clone, Debug)]
pub(super) struct PluginViewPosition {
    pub selection: Selection,
    pub scroll_row: usize,
    pub scroll_wrap: usize,
    pub scroll_col: usize,
}
impl PluginViewPosition {
    pub(super) fn capture(pane: &super::Pane) -> Self {
        Self {
            selection: pane.selection.clone(),
            scroll_row: pane.scroll_row,
            scroll_wrap: pane.scroll_wrap,
            scroll_col: pane.scroll_col,
        }
    }
    pub(super) fn restore(&self, pane: &mut super::Pane) {
        pane.replace_selection(self.selection.clone());
        pane.scroll_row = self.scroll_row;
        pane.scroll_wrap = self.scroll_wrap;
        pane.scroll_col = self.scroll_col;
        pane.preserve_scroll = true;
    }
}

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
        let allowed = self
            .plugins
            .instances
            .get(&owner)
            .and_then(|instance| {
                instance
                    .application
                    .views
                    .values()
                    .find(|view| view.buffer == self.active().buffer)
            })
            .map(|view| &view.model.actions);
        let commands = self
            .plugins
            .commands
            .values()
            .filter(|c| {
                c.plugin == owner
                    && c.context == api::CommandContext::View
                    && allowed
                        .is_some_and(|actions| actions.is_empty() || actions.contains(&c.local))
            })
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

    #[cfg(test)]
    pub(crate) fn create_plugin_view(
        &mut self,
        owner: usize,
        handle: &str,
        model: &view::Model,
    ) -> usize {
        let mut prepared =
            view::PreparedModel::build(model.clone(), &std::sync::atomic::AtomicBool::new(false))
                .unwrap();
        let body = std::mem::take(
            &mut std::sync::Arc::get_mut(&mut prepared.projection)
                .unwrap()
                .text,
        );
        let text = crate::buffer::PluginProjectionSource::empty().prepare(body);
        let spans = prepared.projection.spans.clone();
        self.create_prepared_plugin_view(owner, handle, &prepared, text, spans)
    }

    pub(crate) fn create_prepared_plugin_view(
        &mut self,
        owner: usize,
        handle: &str,
        model: &view::PreparedModel,
        text: crate::buffer::PreparedPluginProjection,
        spans: Vec<Span>,
    ) -> usize {
        let buffer = self.buffers.len();
        let mut document = Buffer::virtual_text_identified(
            GeneratedViewIdentity::Plugin {
                owner,
                view: handle.into(),
            },
            model.model.title.clone(),
            "",
        );
        assert!(
            document.install_plugin_projection(text),
            "new projection must be prepared from empty text"
        );
        self.buffers.push(document);
        self.syntax.push(None);
        self.generated_highlights.insert(buffer, spans);
        self.retire_detached_ephemeral_buffers();
        buffer
    }

    pub(crate) fn publish_prepared_plugin_view(
        &mut self,
        buffer: usize,
        old: &view::Projection,
        model: &view::PreparedModel,
        text: crate::buffer::PreparedPluginProjection,
        spans: Vec<Span>,
        remap: &[usize],
    ) -> bool {
        let projected = &model.projection;
        let row_map = |line: usize| {
            let Some(Some(index)) = old.line_rows.get(line) else {
                return line.min(projected.line_rows.len().saturating_sub(1));
            };
            let destination = remap.get(*index).copied().unwrap_or_default();
            projected.rows.get(destination).map_or(0, |row| row.line)
        };
        let before = self.buffers[buffer].text().clone();
        let captures = self
            .panes
            .iter()
            .filter_map(|(&id, pane)| {
                let saved = if pane.buffer == buffer {
                    PluginViewPosition::capture(pane)
                } else {
                    pane.plugin_view_positions.get(&buffer)?.clone()
                };
                let positions = saved
                    .selection
                    .ranges()
                    .iter()
                    .map(|range| {
                        let point = |offset| {
                            let line = before.offset_to_row(offset);
                            (
                                row_map(line),
                                offset.saturating_sub(before.line_to_offset(line)),
                            )
                        };
                        (
                            point(range.anchor),
                            point(range.head),
                            range.anchor > range.head,
                        )
                    })
                    .collect::<Vec<_>>();
                Some((
                    id,
                    positions,
                    saved.selection.primary_index(),
                    row_map(saved.scroll_row),
                    saved.scroll_wrap,
                    saved.scroll_col,
                ))
            })
            .collect::<Vec<_>>();
        if !self.buffers[buffer].install_plugin_projection(text) {
            return false;
        }
        if let crate::buffer::BufferKind::Virtual { name, .. } = &mut self.buffers[buffer].kind {
            *name = model.model.title.clone();
        }
        let after = &self.buffers[buffer];
        for (id, positions, primary, scroll, scroll_wrap, scroll_col) in captures {
            let ranges = positions
                .into_iter()
                .map(|(anchor, head, reverse)| {
                    let offset = |(line, column): (usize, usize)| {
                        after.line_to_offset(line) + column.min(after.line_len(line))
                    };
                    let mut anchor = offset(anchor);
                    let mut head = offset(head);
                    if (anchor > head) != reverse {
                        std::mem::swap(&mut anchor, &mut head);
                    }
                    Range::new(anchor, head)
                })
                .collect();
            let pane = self.panes.get_mut(&id).unwrap();
            let position = PluginViewPosition {
                selection: Selection::new(ranges, primary),
                scroll_row: scroll.min(after.last_row()),
                scroll_wrap,
                scroll_col,
            };
            if pane.buffer == buffer {
                position.restore(pane);
            }
            if pane.plugin_view_positions.contains_key(&buffer) {
                pane.plugin_view_positions.insert(buffer, position);
            }
        }
        for pane in self.panes.values_mut() {
            pane.jumps.map_selections(buffer, |selection| {
                selection.transform(|range| {
                    let point = |offset| {
                        let line = before.offset_to_row(offset);
                        let column = offset.saturating_sub(before.line_to_offset(line));
                        let line = row_map(line);
                        after.line_to_offset(line) + column.min(after.line_len(line))
                    };
                    let mut anchor = point(range.anchor);
                    let mut head = point(range.head);
                    if (anchor > head) != (range.anchor > range.head) {
                        std::mem::swap(&mut anchor, &mut head);
                    }
                    Range::new(anchor, head)
                })
            });
        }
        self.generated_highlights.insert(buffer, spans);
        true
    }

    pub(crate) fn present_plugin_view(&mut self, buffer: usize) {
        if self.host_buffer_is_closed(buffer) {
            return;
        }
        let changed = self.active().buffer != buffer;
        let saved = self.active().plugin_view_positions.get(&buffer).cloned();
        self.switch_buffer(buffer);
        if changed {
            if let Some(saved) = saved {
                saved.restore(self.active_mut());
            } else if let Some(offset) = self
                .plugins
                .instances
                .values()
                .flat_map(|instance| instance.application.views.values())
                .find(|view| view.buffer == buffer)
                .and_then(|view| view.projection.rows.first())
                .map(|row| row.from)
            {
                self.active_mut()
                    .replace_selection(Selection::point(offset));
            }
        }
        let pane = self.active_mut();
        // At most 16 live views per owner and eight owners. Retired entries
        // are removed by the central buffer retirement path.
        if pane.plugin_view_positions.len() < 128
            || pane.plugin_view_positions.contains_key(&buffer)
        {
            pane.plugin_view_positions
                .insert(buffer, PluginViewPosition::capture(pane));
        }
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
