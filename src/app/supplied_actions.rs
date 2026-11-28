// SPDX-License-Identifier: MPL-2.0
//! Synchronous supplied prompt values. Interactive prompts still own completion.
use super::*;

impl App {
    pub(super) fn execute_supplied_prompt(&mut self, command: EditorCommand, value: &str) {
        use EditorCommand as C;
        if !crate::keymap::admission::admits_command(self.key_binding_scope(), command) {
            self.action_failed("action is unavailable in this view");
            return;
        }
        if command.is_mutating()
            && let Some(reason) = self.active_buffer().read_only_reason()
        {
            self.action_failed(reason);
            return;
        }
        // Match the interactive command's gesture boundary before accepting
        // its value. Tree handlers return before this ordinary restoration;
        // terminal search already discards its origin as an atomic action.
        if command != C::DirectoryTreeSearch
            && !crate::keymap::temporary::continues_temporary(command)
            && let Some((mode, name)) = self.take_temporary_mode()
        {
            self.restore_binding_mode(mode, name.as_deref());
        }
        match command {
            C::Search | C::SearchRegex => {
                if !self.require_editing_role(Mode::Select) {
                    return;
                }
                let query = SearchQuery {
                    pattern: value.into(),
                    mode: if command == C::SearchRegex {
                        SearchMode::Regex
                    } else {
                        SearchMode::Insensitive
                    },
                    region: self.scoping_region(),
                };
                if !self.commit_search(query) {
                    self.sequence_halted = true;
                } else {
                    self.line_select = None;
                }
            }
            C::TerminalSearch | C::TerminalSearchRegex => {
                if !self.search_terminal_review(
                    value,
                    if command == C::TerminalSearchRegex {
                        SearchMode::Regex
                    } else {
                        SearchMode::Insensitive
                    },
                ) {
                    self.sequence_halted = true;
                }
            }
            C::DirectoryTreeSearch => {
                let result = self.directory_tree.search(value);
                self.report_tree_search(result);
            }
            C::GlobalSearch | C::GlobalSearchRegex => {
                self.open_global_search(
                    value,
                    if command == C::GlobalSearchRegex {
                        SearchMode::Regex
                    } else {
                        SearchMode::Insensitive
                    },
                );
            }
            C::KeepMatchingSelections | C::RemoveMatchingSelections => {
                self.filter_selections(command == C::KeepMatchingSelections, value)
            }
            C::JoinSelections => self.join_selections(value),
            _ => unreachable!("supplied prompt inventory and dispatch agree"),
        }
    }
}
