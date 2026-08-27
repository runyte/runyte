// SPDX-License-Identifier: MPL-2.0

//! Non-blocking language-service coordination and language-backed result lists.

// Application-module dependencies:
use super::{
    ActionEntry, App, Assoc, BTreeMap, Buffer, BufferAction, BufferActionMenu, BufferKind, Change,
    ChangeSync, Completion, CompletionSource, CompletionState, ContextAction, ContextActionMenu,
    DocumentEdit, DocumentState, DocumentSyntax, Encoding, GLOBAL_SEARCH_RESULT_LIMIT, HashMap,
    HashSet, HoverState, InputGrammar, KeyCode, KeyStroke, ListAction, ListPicker, ListPurpose,
    LspCommand, LspEvent, LspHandle, LspRange, Mode, Modifiers, Offset,
    PATH_COMPLETION_ITEM_LIMIT_PER_ROOT, Path, PathActionMenu, PathBuf, PathClipboardTarget,
    PathPopup, PendingRequest, PickerItem, PromptKind, Range, Register, RequestKind, Response,
    Result, SPECIAL_BUFFER_RETENTION_LIMIT, SearchMode, Selection, SelectionSemantics, ServerState,
    SignatureContext, SignatureState, TerminalAction, TerminalActionMenu, TerminalSession, Text,
    TextDocumentContentChangeEvent, TrackedRequest, Transaction, WORD_COMPLETION_ITEM_LIMIT,
    WorkspaceSearchTarget, buffer_language, buffer_picker_columns, buffer_preview,
    checked_lsp_range, display_path, edit_summary, from_lsp_position, from_lsp_range,
    is_word_completion_character, language_completion_prefix_start, matches_in_text, open_or_new,
    operative_span, parse_buffer, path_token_before, push_matching_words, response_name,
    row_is_not_before, to_lsp_position, word_bounds, word_token_before,
    workspace_edit_path_identity, workspace_matches,
};
#[cfg(unix)]
use super::{SessionAction, SessionActionMenu};

// -- Language servers ------------------------------------------------------
//
// Every method here is non-blocking. Requests are queued with a token and
// forgotten; responses arrive later as `LspEvent`s and are matched back
// against `lsp_requests`. Nothing in this section can stall a frame, which is
// the Phase 2 gate.

impl App {
    /// Connects the editor to a running language-server manager.
    ///
    /// Separate from `App::new` because spawning the manager needs a Tokio
    /// runtime, and the editor must remain constructible without one.
    pub fn attach_lsp(&mut self, handle: LspHandle) {
        self.ports.attach_lsp(handle);
        for buffer_id in 0..self.buffers.len() {
            self.lsp_touch(buffer_id);
        }
    }

    /// The language name a buffer maps to, which is the same question asked by
    /// syntax highlighting: one buffer, one language, one server.
    pub(super) fn language_of(&self, buffer_id: usize) -> Option<String> {
        let buffer = self.buffers.get(buffer_id)?;
        let language = buffer_language(buffer, &self.registry)?;
        Some(self.registry.language_name(language).to_owned())
    }

    /// Makes sure a buffer's server is starting, and opens the document once
    /// the handshake has settled.
    pub(super) fn lsp_touch(&mut self, buffer_id: usize) -> bool {
        if !self.ports.has_lsp() || self.closed_buffers.contains(&buffer_id) {
            return false;
        }
        let desired = self.language_of(buffer_id).zip(
            self.buffers
                .get(buffer_id)
                .and_then(|buffer| buffer.path.clone()),
        );
        let current_matches = self.lsp_documents.get(&buffer_id).is_some_and(|document| {
            desired.as_ref().is_some_and(|(language, path)| {
                document.language == *language && document.path == *path
            })
        });
        if self.lsp_documents.contains_key(&buffer_id) && !current_matches {
            self.retire_lsp_buffer(buffer_id);
        }
        let Some((language, path)) = desired else {
            return false;
        };
        if !self.lsp_servers.contains_key(&language) {
            self.lsp_send(LspCommand::Ensure { language });
            return false;
        }
        if current_matches {
            if self.lsp_documents[&buffer_id].desynced {
                let document = self.lsp_documents[&buffer_id].clone();
                if self.lsp_servers[&document.language].sync.change != ChangeSync::None {
                    let accepted = self.lsp_send(LspCommand::Change {
                        language: document.language,
                        path: document.path,
                        version: document.version,
                        changes: vec![TextDocumentContentChangeEvent {
                            range: None,
                            range_length: None,
                            text: self.buffers[buffer_id].to_string(),
                        }],
                    });
                    if accepted {
                        self.lsp_documents.get_mut(&buffer_id).unwrap().desynced = false;
                    }
                }
            }
            return self
                .lsp_documents
                .get(&buffer_id)
                .is_some_and(|document| !document.desynced);
        }
        let sync = self.lsp_servers[&language].sync;
        let opened = !sync.open_close
            || self.lsp_send(LspCommand::Open {
                language: language.clone(),
                path: path.clone(),
                version: 1,
                text: self.buffers[buffer_id].to_string(),
            });
        if opened {
            self.lsp_documents.insert(
                buffer_id,
                DocumentState {
                    language,
                    path,
                    version: 1,
                    desynced: false,
                },
            );
        }
        opened
    }

    /// Closes one server-owned document and invalidates everything derived
    /// from that buffer/language pairing. A later `lsp_touch` may immediately
    /// open the same buffer under a new path or inferred language.
    pub(super) fn retire_lsp_buffer(&mut self, buffer_id: usize) {
        if let Some(document) = self.lsp_documents.remove(&buffer_id) {
            self.diagnostics.clear_path(&document.path);
            self.lsp_send(LspCommand::Close {
                language: document.language,
                path: document.path,
            });
        } else if let Some(path) = self
            .buffers
            .get(buffer_id)
            .and_then(|buffer| buffer.path.as_deref())
        {
            self.diagnostics.clear_path(path);
        }
        let retired: Vec<u64> = self
            .lsp_requests
            .iter()
            .filter_map(|(token, request)| (request.buffer == buffer_id).then_some(*token))
            .collect();
        for token in retired {
            self.cancel_lsp_request(token);
        }
        if self
            .completion
            .as_ref()
            .is_some_and(|completion| completion.buffer == buffer_id)
        {
            self.completion = None;
        }
        if self.active().buffer == buffer_id {
            self.signature = None;
            self.hover = None;
        }
    }

    pub(super) fn lsp_send(&mut self, command: LspCommand) -> bool {
        self.flush_lsp_replies();
        match self.ports.send_lsp(command) {
            Some(true) => true,
            Some(false) => {
                self.error("language server manager is not accepting work");
                self.unavailable_revision = self.unavailable_revision.wrapping_add(1);
                false
            }
            None => false,
        }
    }

    pub(super) fn flush_lsp_replies(&mut self) {
        while let Some(command) = self.pending_lsp_replies.front().cloned() {
            match self.ports.send_lsp(command) {
                Some(true) => {
                    self.pending_lsp_replies.pop_front();
                }
                Some(false) => break,
                None => {
                    self.pending_lsp_replies.clear();
                    break;
                }
            }
        }
    }

    fn lsp_reply(&mut self, command: LspCommand) {
        self.flush_lsp_replies();
        if self.ports.send_lsp(command.clone()) == Some(true) {
            return;
        }
        if self.pending_lsp_replies.len() < crate::lsp::EVENT_CAPACITY {
            self.pending_lsp_replies.push_back(command);
        } else {
            self.error("language server reply queue is full");
        }
    }

    fn cancel_lsp_request(&mut self, token: u64) {
        let Some(request) = self.lsp_requests.get_mut(&token) else {
            return;
        };
        request.cancelled = true;
        if self.lsp_send(LspCommand::Cancel { token }) {
            self.lsp_requests.remove(&token);
        }
    }

    /// Reports a document edit to the server that owns it.
    ///
    /// `before` is the text the transaction's offsets were computed against,
    /// which is the only thing the server's coordinates can be derived from.
    pub(super) fn lsp_change(
        &mut self,
        buffer_id: usize,
        before: &Text,
        transaction: &Transaction,
    ) -> bool {
        let Some(document) = self.lsp_documents.get(&buffer_id) else {
            return true;
        };
        let language = document.language.clone();
        let Some(server) = self.lsp_servers.get(&language) else {
            return false;
        };
        let (encoding, change_sync) = (server.encoding, server.sync.change);
        let Some(path) = self.buffers[buffer_id].path.clone() else {
            return false;
        };
        let (version, desynced) = {
            let document = self.lsp_documents.get_mut(&buffer_id).unwrap();
            let Some(version) = document.version.checked_add(1) else {
                document.desynced = true;
                return false;
            };
            document.version = version;
            (version, document.desynced)
        };
        if change_sync == ChangeSync::None {
            return false;
        }
        let changes = if change_sync == ChangeSync::Incremental && !desynced {
            // Descending order, so each range still describes the document the
            // transaction was built against: the server applies content
            // changes in sequence, and an earlier one would shift a later one.
            transaction
                .changes()
                .iter()
                .rev()
                .map(|change| TextDocumentContentChangeEvent {
                    range: Some(LspRange::new(
                        to_lsp_position(before, change.from, encoding),
                        to_lsp_position(before, change.to, encoding),
                    )),
                    range_length: None,
                    text: change.text.clone(),
                })
                .collect()
        } else {
            vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: self.buffers[buffer_id].to_string(),
            }]
        };
        let accepted = self.lsp_send(LspCommand::Change {
            language,
            path,
            version,
            changes,
        });
        let document = self.lsp_documents.get_mut(&buffer_id).unwrap();
        document.desynced = !accepted;
        accepted
    }

    /// Resynchronizes a whole document.
    ///
    /// Undo and redo move text without producing a transaction the caller can
    /// hand onwards, so the server is given the new text outright rather than
    /// a delta it could not be derived from.
    pub(super) fn lsp_resync(&mut self, buffer_id: usize) {
        let Some(document) = self.lsp_documents.get(&buffer_id) else {
            return;
        };
        let language = document.language.clone();
        let change_sync = self
            .lsp_servers
            .get(&language)
            .map_or(ChangeSync::None, |server| server.sync.change);
        let Some(version) = document.version.checked_add(1) else {
            self.lsp_documents.get_mut(&buffer_id).unwrap().desynced = true;
            return;
        };
        self.lsp_documents.get_mut(&buffer_id).unwrap().version = version;
        if change_sync == ChangeSync::None {
            return;
        }
        let Some(path) = self.buffers[buffer_id].path.clone() else {
            return;
        };
        let text = self.buffers[buffer_id].to_string();
        let accepted = self.lsp_send(LspCommand::Change {
            language,
            path,
            version,
            changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text,
            }],
        });
        let document = self.lsp_documents.get_mut(&buffer_id).unwrap();
        document.desynced = !accepted;
    }

    pub(super) fn lsp_save(&mut self, buffer_id: usize) {
        if !self.lsp_touch(buffer_id) {
            return;
        }
        let Some(document) = self.lsp_documents.get(&buffer_id) else {
            return;
        };
        let language = document.language.clone();
        let Some(path) = self.buffers[buffer_id].path.clone() else {
            return;
        };
        let text = self.buffers[buffer_id].to_string();
        self.lsp_send(LspCommand::Save {
            language,
            path,
            text,
        });
    }

    fn encoding_for(&self, language: &str) -> Encoding {
        self.lsp_servers
            .get(language)
            .map_or(Encoding::default(), |server| server.encoding)
    }

    pub(super) fn lsp_document_guards(&self) -> HashMap<PathBuf, (usize, u64)> {
        self.lsp_documents
            .keys()
            .filter_map(|buffer_id| {
                let buffer = self.buffers.get(*buffer_id)?;
                let path = buffer.path.as_deref()?;
                let identity = workspace_edit_path_identity(path).ok()?;
                Some((identity, (*buffer_id, buffer.revision())))
            })
            .collect()
    }

    fn lsp_document_guards_are_current(&self, guards: &HashMap<PathBuf, (usize, u64)>) -> bool {
        guards.iter().all(|(identity, (buffer_id, revision))| {
            !self.closed_buffers.contains(buffer_id)
                && self.buffers.get(*buffer_id).is_some_and(|buffer| {
                    buffer.revision() == *revision
                        && buffer.path.as_deref().is_some_and(|path| {
                            workspace_edit_path_identity(path)
                                .is_ok_and(|current| current == *identity)
                        })
                })
        })
    }

    /// Queues a request about the active buffer.
    ///
    /// Returns quietly after a status message when there is no server, no
    /// language, or no path — degrading to no-LSP behavior is the normal case,
    /// not an error worth interrupting an edit for.
    fn lsp_request(&mut self, kind: RequestKind, pending: PendingRequest) {
        let buffer_id = self.active().buffer;
        let revision = self.buffers[buffer_id].revision();
        self.lsp_request_from(buffer_id, revision, kind, pending);
    }

    /// Queues a request whose provenance was captured before a picker or
    /// another asynchronous UI step. The active pane may have changed in the
    /// meantime, but the eventual response still belongs to this buffer and
    /// revision.
    fn lsp_request_from(
        &mut self,
        buffer_id: usize,
        revision: u64,
        kind: RequestKind,
        pending: PendingRequest,
    ) {
        let _ = self.try_lsp_request_from(buffer_id, revision, kind, pending);
    }

    /// Queues a request and reports whether it was accepted by the manager.
    /// Explicit completion uses the result to avoid installing a session that
    /// can never receive candidates.
    fn try_lsp_request_from(
        &mut self,
        buffer_id: usize,
        revision: u64,
        kind: RequestKind,
        pending: PendingRequest,
    ) -> bool {
        let label = kind.label();
        if !self.ports.has_lsp() {
            self.error(format!("{label} needs a language server"));
            return false;
        }
        let Some(language) = self.language_of(buffer_id) else {
            self.error(format!("{label} needs a known language"));
            return false;
        };
        let Some(path) = self.buffers[buffer_id].path.clone() else {
            self.error(format!("{label} needs a saved file"));
            return false;
        };
        let Some((supported, generation)) = self
            .lsp_servers
            .get(&language)
            .map(|server| (server.capabilities.supports(&kind), server.generation))
        else {
            self.lsp_send(LspCommand::Ensure {
                language: language.clone(),
            });
            self.error(format!("{language} language server is not ready yet"));
            return false;
        };
        // A capability the server never advertised is never asked for: the
        // answer would be `Method not found`, indistinguishable at that point
        // from a real protocol violation, and every trigger character typed
        // near an unsupported request would otherwise cost a round trip that
        // always refuses.
        if !supported {
            self.mark_unsupported(format!(
                "the {language} language server does not support {label}"
            ));
            return false;
        }
        if !self.lsp_touch(buffer_id) {
            self.error(format!("{label} needs a synchronized document"));
            return false;
        }
        let documents = matches!(
            &pending,
            PendingRequest::Edits { .. } | PendingRequest::CodeActions
        )
        .then(|| self.lsp_document_guards())
        .unwrap_or_default();
        if let Some(group) = pending.transient_group() {
            let superseded: Vec<u64> = self
                .lsp_requests
                .iter()
                .filter_map(|(token, request)| {
                    (request.buffer == buffer_id
                        && request.pending.transient_group() == Some(group))
                    .then_some(*token)
                })
                .collect();
            for token in superseded {
                self.cancel_lsp_request(token);
            }
        }
        let token = self.next_lsp_token;
        let Some(next_token) = token.checked_add(1) else {
            self.error("language-server request identity space is exhausted");
            return false;
        };
        let command = LspCommand::Request {
            token,
            language: language.clone(),
            path,
            kind: Box::new(kind),
        };
        if self.lsp_send(command) {
            self.next_lsp_token = next_token;
            self.lsp_requests.insert(
                token,
                TrackedRequest::new(buffer_id, revision, pending)
                    .with_documents(documents)
                    .with_server(language, generation),
            );
            true
        } else {
            false
        }
    }

    fn action_command_supported(
        &self,
        language: &str,
        generation: u64,
        command: &lsp_types::Command,
    ) -> bool {
        self.lsp_servers.get(language).is_some_and(|server| {
            server.generation == generation
                && server
                    .capabilities
                    .supports(&RequestKind::ExecuteCommand(Box::new(command.clone())))
        })
    }

    fn send_action_command(
        &mut self,
        buffer: usize,
        language: String,
        generation: u64,
        command: lsp_types::Command,
    ) {
        if !self.action_command_supported(&language, generation, &command) {
            self.mark_unsupported(format!(
                "the {language} language server did not advertise command {}",
                command.command
            ));
            return;
        }
        let Some(path) = self
            .buffers
            .get(buffer)
            .and_then(|buffer| buffer.path.clone())
        else {
            self.error("command needs a saved file");
            return;
        };
        let token = self.next_lsp_token;
        let Some(next_token) = token.checked_add(1) else {
            self.error("language-server request identity space is exhausted");
            return;
        };
        if self.lsp_send(LspCommand::Request {
            token,
            language: language.clone(),
            path,
            kind: Box::new(RequestKind::ExecuteCommand(Box::new(command))),
        }) {
            self.next_lsp_token = next_token;
            self.lsp_requests.insert(
                token,
                TrackedRequest::new(
                    buffer,
                    self.buffers[buffer].revision(),
                    PendingRequest::Edits {
                        label: "ran",
                        path: PathBuf::new(),
                    },
                )
                .with_documents(self.lsp_document_guards())
                .with_server(language, generation),
            );
        }
    }

    /// The primary caret, in the coordinates the active buffer's server uses.
    pub(super) fn lsp_cursor(&self) -> crate::lsp::LspPosition {
        let buffer_id = self.active().buffer;
        let encoding = self
            .language_of(buffer_id)
            .map_or(Encoding::default(), |language| self.encoding_for(&language));
        to_lsp_position(
            self.buffers[buffer_id].text(),
            self.active().head(),
            encoding,
        )
    }

    pub(super) fn lsp_goto(&mut self, kind: RequestKind) {
        let label = kind.label();
        self.lsp_request(kind, PendingRequest::Goto { label });
    }

    pub(super) fn lsp_hover(&mut self) {
        let position = self.lsp_cursor();
        self.lsp_request(RequestKind::Hover(position), PendingRequest::Hover);
    }

    pub(super) fn lsp_completion(&mut self) {
        let _ = self.request_lsp_completion(None);
    }

    fn request_lsp_completion(&mut self, explicit_session: Option<u64>) -> bool {
        let buffer_id = self.active().buffer;
        let head = self.active().head();
        let anchor = language_completion_prefix_start(&self.buffers[buffer_id], head);
        let position = self.lsp_cursor();
        let revision = self.buffers[buffer_id].revision();
        self.try_lsp_request_from(
            buffer_id,
            revision,
            RequestKind::Completion(position),
            PendingRequest::Completion {
                buffer: buffer_id,
                anchor,
                explicit_session,
            },
        )
    }

    pub(super) fn start_explicit_lsp_completion(&mut self) {
        let session = self.next_completion_session;
        if !self.request_lsp_completion(Some(session)) {
            return;
        }
        self.next_completion_session += 1;
        let buffer = self.active().buffer;
        let head = self.active().head();
        let anchor = language_completion_prefix_start(&self.buffers[buffer], head);
        self.completion = Some(CompletionState {
            items: Vec::new(),
            selected: 0,
            buffer,
            anchor,
            filter: self.buffers[buffer].slice(anchor, head),
            source: CompletionSource::Language,
            explicit_session: Some(session),
        });
    }

    pub(super) fn explicit_completion_session(&self) -> Option<u64> {
        self.completion.as_ref().and_then(|completion| {
            (completion.source == CompletionSource::Language)
                .then_some(completion.explicit_session)
                .flatten()
        })
    }

    pub(super) fn refresh_explicit_completion_filter(&mut self) {
        let Some(session) = self.explicit_completion_session() else {
            return;
        };
        let buffer = self.active().buffer;
        let head = self.active().head();
        let anchor = language_completion_prefix_start(&self.buffers[buffer], head);
        let previous_anchor = self.completion.as_ref().map(|state| state.anchor);
        if previous_anchor != Some(anchor) {
            self.restart_explicit_lsp_completion(session);
            return;
        }
        let filter = self.buffers[buffer].slice(anchor, head);
        if let Some(state) = self.completion.as_mut() {
            state.filter = filter;
            state.selected = 0;
        }
    }

    pub(super) fn restart_explicit_lsp_completion(&mut self, old_session: u64) {
        if self.explicit_completion_session() != Some(old_session) {
            return;
        }
        self.completion = None;
        self.start_explicit_lsp_completion();
    }

    /// Offers entries below a filesystem path that has just gained `/`.
    ///
    /// Relative paths are intentionally tried against both useful editor
    /// contexts: the active file's directory and the stable project root.
    /// The same spelling is shown once even when both directories contain it.
    pub(super) fn path_completion(&mut self) {
        let buffer = self.active().buffer;
        let head = self.active().head();
        let token = path_token_before(&self.buffers[buffer], head);
        let Some(slash_at) = token.rfind('/') else {
            return;
        };
        let (directory_part, fragment) = token.split_at(slash_at + 1);
        let requested = Path::new(directory_part);
        let mut directories = Vec::new();
        if requested.is_absolute() {
            directories.push(requested.to_path_buf());
        } else {
            if let Some(directory) = self.buffer_directory(buffer) {
                directories.push(directory.join(requested));
            }
            directories.push(self.project_root.join(requested));
        }

        let mut visited = HashSet::new();
        let mut candidates = BTreeMap::<String, Completion>::new();
        // The typed fragment narrows the listing here rather than only in
        // `visible_indices`, so that the bound below is spent on names the
        // person could still be typing. Both filter without regard to case,
        // and the popup's own filter is the one a person sees, so this one
        // folds names exactly as that one does.
        let wanted = fragment.to_lowercase();
        for directory in directories {
            let Ok(directory) = directory.canonicalize() else {
                continue;
            };
            if !directory.is_dir() || !visited.insert(directory.clone()) {
                continue;
            }
            let Some(entries) = self.path_listings.borrow_mut().read(&directory) else {
                continue;
            };
            let detail = display_path(&directory);
            let mut kept = BTreeMap::<String, Completion>::new();
            for entry in entries.iter() {
                if !entry.name.to_lowercase().starts_with(&wanted) {
                    continue;
                }
                let is_directory = entry.is_directory;
                // Once the bound is full, only a name that sorts before the
                // last row kept can change the answer.
                if kept.len() >= PATH_COMPLETION_ITEM_LIMIT_PER_ROOT
                    && kept.last_key_value().is_some_and(|(last, _)| {
                        row_is_not_before(&entry.name, is_directory, '/', last)
                    })
                {
                    continue;
                }
                let label = if is_directory {
                    format!("{}/", entry.name)
                } else {
                    entry.name.clone()
                };
                kept.insert(
                    label.clone(),
                    Completion {
                        label: label.clone(),
                        filter_text: None,
                        sort_text: None,
                        detail: detail.clone(),
                        kind: if is_directory { "directory" } else { "file" },
                        insert: label,
                        edit: None,
                        additional: Vec::new(),
                    },
                );
                // Dropping the largest label keeps the smallest ones, so a
                // directory too large to offer whole is cut at a place the
                // person can predict instead of wherever the filesystem
                // happened to return entries.
                if kept.len() > PATH_COMPLETION_ITEM_LIMIT_PER_ROOT {
                    kept.pop_last();
                }
            }
            for (label, candidate) in kept {
                candidates.entry(label).or_insert(candidate);
            }
        }

        if candidates.is_empty() {
            self.completion = None;
            return;
        }
        self.completion = Some(CompletionState {
            items: candidates.into_values().collect(),
            selected: 0,
            buffer,
            anchor: head - fragment.chars().count(),
            filter: fragment.to_owned(),
            source: CompletionSource::Path,
            explicit_session: None,
        });
    }

    /// Offers words already seen elsewhere in the workspace once the typed
    /// prefix reaches `editor.word_completion_minimum`.
    ///
    /// Never opens over an active Language or Path completion: those already
    /// showing takes precedence, and a later Language response is still free
    /// to replace a Word popup (`show_completion` only protects Path).
    pub(super) fn word_completion(&mut self, character: char) {
        if !is_word_completion_character(character) || self.completion.is_some() {
            return;
        }
        if !self.config.editor.word_completion {
            return;
        }
        let Some(handle) = self.ports.word_index() else {
            return;
        };
        let buffer_id = self.active().buffer;
        let buffer = &self.buffers[buffer_id];
        if buffer.is_read_only() || buffer.is_directory() {
            return;
        }
        let head = self.active().head();
        let token = word_token_before(buffer, head);
        let query_len = token.chars().count();
        if query_len < self.config.editor.word_completion_minimum {
            return;
        }
        let query = token.to_lowercase();
        let snapshot = handle.current();
        let mut seen = HashSet::new();
        seen.insert(query.clone());
        let mut items = Vec::new();
        if let Some(own) = snapshot.buffer_words(buffer_id) {
            push_matching_words(&mut items, &mut seen, own.entries(), &query);
        }
        let mut others: Vec<_> = snapshot.other_buffers(buffer_id).collect();
        others.sort_by_key(|(id, _)| *id);
        for (_, words) in others {
            push_matching_words(&mut items, &mut seen, words.entries(), &query);
        }
        if items.is_empty() {
            return;
        }
        items.truncate(WORD_COMPLETION_ITEM_LIMIT);
        self.completion = Some(CompletionState {
            items,
            selected: 0,
            buffer: buffer_id,
            anchor: head - query_len,
            filter: token,
            source: CompletionSource::Word,
            explicit_session: None,
        });
    }

    pub(super) fn lsp_signature(&mut self, context: SignatureContext) {
        if !self.ports.has_lsp() {
            return;
        }
        let position = self.lsp_cursor();
        self.lsp_request(
            RequestKind::SignatureHelp { position, context },
            PendingRequest::Signature,
        );
    }

    pub(super) fn lsp_document_symbols(&mut self) {
        let path = self.active_buffer().path.clone().unwrap_or_default();
        self.lsp_request(
            RequestKind::DocumentSymbols,
            PendingRequest::Symbols {
                title: "Document symbols",
                path,
            },
        );
    }

    pub(super) fn lsp_workspace_symbols(&mut self) {
        self.lsp_request(
            // An empty query asks for everything the server is willing to
            // offer; the picker's own filter narrows it without a round trip.
            RequestKind::WorkspaceSymbols(String::new()),
            PendingRequest::Symbols {
                title: "Workspace symbols",
                path: PathBuf::new(),
            },
        );
    }

    pub(super) fn lsp_code_actions(&mut self) {
        let buffer_id = self.active().buffer;
        let Some(language) = self.language_of(buffer_id) else {
            self.error("code actions need a known language");
            return;
        };
        let encoding = self.encoding_for(&language);
        let text = self.buffers[buffer_id].text();
        let range = self.active().selection.primary();
        let (from, to) = operative_span(&self.buffers[buffer_id], &range);
        let lsp_range = LspRange::new(
            to_lsp_position(text, from, encoding),
            to_lsp_position(text, to, encoding),
        );
        // Only diagnostics overlapping the selection are relevant context, and
        // sending the whole file's worth would make quick fixes for unrelated
        // errors show up here.
        let diagnostics = self.buffers[buffer_id]
            .path
            .as_deref()
            .map(|path| {
                self.diagnostics
                    .for_path(path)
                    .iter()
                    .filter(|diagnostic| {
                        diagnostic.range.start <= lsp_range.end
                            && diagnostic.range.end >= lsp_range.start
                    })
                    .map(|diagnostic| diagnostic.raw.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.lsp_request(
            RequestKind::CodeActions {
                range: lsp_range,
                diagnostics,
            },
            PendingRequest::CodeActions,
        );
    }

    pub(super) fn lsp_format(&mut self) {
        let path = self.active_buffer().path.clone().unwrap_or_default();
        self.lsp_request(
            RequestKind::Format {
                tab_size: self.config.editor.tab_width.max(1) as u32,
                insert_spaces: true,
            },
            PendingRequest::Edits {
                label: "formatted",
                path,
            },
        );
    }

    /// Opens the rename prompt, seeded with the word under the caret so the
    /// common case is one keystroke away from a name that already exists.
    pub(super) fn lsp_rename_prompt(&mut self) {
        if !self.has_language_server() {
            self.error("rename needs a language server");
            return;
        }
        let buffer = self.active_buffer();
        let (from, to) = word_bounds(buffer, self.active().head());
        let current = buffer.slice(from, to);
        self.open_prompt(PromptKind::Rename);
        self.command = current;
        self.command_cursor = self.command.chars().count();
    }

    pub(super) fn lsp_rename(&mut self, new_name: String) {
        let position = self.lsp_cursor();
        let path = self.active_buffer().path.clone().unwrap_or_default();
        self.lsp_request(
            RequestKind::Rename { position, new_name },
            PendingRequest::Edits {
                label: "renamed",
                path,
            },
        );
    }

    /// Opens the diagnostics picker. Needs no request: diagnostics are pushed
    /// by the server and already in the store.
    pub(super) fn open_diagnostics_picker(&mut self) {
        let entries: Vec<(PathBuf, crate::lsp::Diagnostic)> = self
            .diagnostics
            .all()
            .into_iter()
            .map(|(path, diagnostic)| (path.to_path_buf(), diagnostic.clone()))
            .collect();
        if entries.is_empty() {
            self.status("no diagnostics");
            return;
        }
        let mut items = Vec::with_capacity(entries.len());
        let mut actions = Vec::with_capacity(entries.len());
        for (index, (path, diagnostic)) in entries.into_iter().enumerate() {
            let encoding = self
                .lsp_documents
                .values()
                .find(|document| document.path == path)
                .map(|document| self.encoding_for(&document.language))
                .unwrap_or_default();
            items.push(PickerItem::new(
                format!("{}:{}", display_path(&path), diagnostic.row() + 1),
                diagnostic.label(),
                index,
            ));
            actions.push(ListAction::Jump(crate::lsp::Location {
                path,
                range: diagnostic.range,
                encoding,
            }));
        }
        self.list_actions = actions;
        self.list = Some(ListPicker::new("Diagnostics", items).with_primary_action("jump"));
    }

    pub(super) fn open_buffer_picker(&mut self) {
        self.rebuild_buffer_picker(String::new(), 0);
    }

    fn rebuild_buffer_picker(&mut self, filter: String, selected: usize) {
        let active = self
            .active_terminal()
            .is_none()
            .then(|| self.active().buffer);
        let mut items = Vec::new();
        let mut actions = Vec::new();
        for (index, buffer) in self.buffers.iter().enumerate() {
            if !self.buffer_is_discoverable(index) {
                continue;
            }
            let (label, detail) =
                buffer_picker_columns(buffer, &self.project_root, active == Some(index));
            let action_index = actions.len();
            items.push(
                PickerItem::new(label, detail, action_index).with_preview(buffer_preview(buffer)),
            );
            actions.push(ListAction::Buffer(index));
        }
        self.list_actions = actions;
        let mut picker = ListPicker::new("Buffers", items)
            .with_preview("Contents")
            .as_manager("open", "Tab", "actions");
        picker.filter = filter;
        picker.selected = selected.min(picker.visible_indices().len().saturating_sub(1));
        self.list = Some(picker);
        self.buffer_action_menu = None;
    }

    pub(super) fn buffer_is_discoverable(&self, index: usize) -> bool {
        if self.closed_buffers.contains(&index) {
            return false;
        }
        let Some(buffer) = self.buffers.get(index) else {
            return false;
        };
        let displayed = self
            .panes
            .values()
            .any(|pane| pane.terminal.is_none() && pane.buffer == index);
        if buffer.is_empty_clean_scratch() && !displayed {
            return false;
        }
        true
    }

    pub(super) fn open_global_search(&mut self, pattern: &str, mode: SearchMode) {
        let matcher = match mode.compile(pattern) {
            Ok(matcher) => matcher,
            Err(error) => {
                self.error(format!("invalid regular expression: {error}"));
                return;
            }
        };
        let (mut matches, mut limited) = match workspace_matches(
            &self.project_root,
            &matcher,
            self.config.editor.show_hidden_files,
        ) {
            Ok(matches) => matches,
            Err(error) => {
                self.error(error.to_string());
                return;
            }
        };
        // Open buffers are authoritative over their on-disk versions so a
        // workspace search never jumps to text the person has already edited
        // away but not saved yet.
        for (buffer_id, buffer) in self.buffers.iter().enumerate() {
            if self.closed_buffers.contains(&buffer_id) {
                continue;
            }
            let Some(path) = buffer.path.as_deref() else {
                continue;
            };
            if buffer.is_directory() || !path.starts_with(&self.project_root) {
                continue;
            }
            matches.retain(|found| found.path != path);
            matches.extend(matches_in_text(path, &buffer.to_string(), &matcher));
        }
        matches.sort_by(|left, right| {
            (&left.path, left.row, left.column).cmp(&(&right.path, right.row, right.column))
        });
        limited |= matches.len() > GLOBAL_SEARCH_RESULT_LIMIT;
        matches.truncate(GLOBAL_SEARCH_RESULT_LIMIT);
        let result_count = matches.len();

        let mut lines = vec![
            format!("Query: {pattern}"),
            format!(
                "Mode: {} · {} result{} · query-time snapshot{}",
                if mode == SearchMode::Regex {
                    "regular expression"
                } else if mode == SearchMode::Sensitive {
                    "case-sensitive literal"
                } else {
                    "case-insensitive literal"
                },
                matches.len(),
                if matches.len() == 1 { "" } else { "s" },
                if limited {
                    " · result limit reached"
                } else {
                    ""
                }
            ),
            "Rerun the workspace search to refresh these results.".to_owned(),
            String::new(),
        ];
        let mut rows = vec![None; lines.len()];
        for found in matches {
            let relative = found
                .path
                .strip_prefix(&self.project_root)
                .unwrap_or(&found.path);
            let relative = relative
                .display()
                .to_string()
                .replace('\r', "\\r")
                .replace('\n', "\\n");
            lines.push(format!(
                "{}:{}:{}  {}",
                relative,
                found.row + 1,
                found.column + 1,
                found.preview
            ));
            rows.push(Some(WorkspaceSearchTarget {
                path: found.path,
                row: found.row,
                column: found.column,
                length: found.length,
            }));
        }
        let text = lines.join("\n");
        let mode_label = mode.qualifier().trim().to_owned();
        let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_workspace_search()).then_some(index)
        });
        let buffer = if let Some(existing) = existing {
            let preserved = self
                .panes
                .iter()
                .filter(|(_, pane)| pane.buffer == existing)
                .map(|(pane_id, pane)| {
                    let row = self.buffers[existing].offset_to_row(pane.head());
                    (
                        *pane_id,
                        self.buffers[existing]
                            .workspace_search_target_at(row)
                            .cloned(),
                    )
                })
                .collect::<Vec<_>>();
            self.buffers[existing]
                .replace_workspace_search(pattern, mode_label, &text, rows, limited);
            for (pane_id, target) in preserved {
                let row = target
                    .as_ref()
                    .and_then(|target| {
                        (0..self.buffers[existing].len_lines()).find(|row| {
                            self.buffers[existing].workspace_search_target_at(*row) == Some(target)
                        })
                    })
                    .unwrap_or(0);
                let offset = self.buffers[existing].line_to_offset(row);
                if let Some(pane) = self.panes.get_mut(&pane_id) {
                    pane.replace_selection(Selection::point(offset));
                    pane.preserve_scroll = false;
                }
            }
            existing
        } else {
            self.buffers.push(Buffer::workspace_search(
                pattern, mode_label, &text, rows, limited,
            ));
            self.syntax.push(None);
            self.buffers.len() - 1
        };
        self.list = None;
        self.push_jump();
        let first_result = (0..self.buffers[buffer].len_lines())
            .find(|row| {
                self.buffers[buffer]
                    .workspace_search_target_at(*row)
                    .is_some()
            })
            .unwrap_or(0);
        let offset = self.buffers[buffer].line_to_offset(first_result);
        let pane = self.active_mut();
        pane.retarget(buffer);
        pane.replace_selection(Selection::point(offset));
        pane.preserve_scroll = false;
        self.mode = Mode::Normal;
        self.status(format!(
            "workspace search: {} result{}{}",
            result_count,
            if result_count == 1 { "" } else { "s" },
            if limited { " (limit reached)" } else { "" }
        ));
    }

    /// Applies an editor event from the language-server manager.
    pub fn apply_lsp_event(&mut self, event: LspEvent) {
        match event {
            LspEvent::Ready {
                language,
                generation,
                name,
                encoding,
                sync,
                capabilities,
            } => {
                crate::log_info!(
                    "lsp",
                    "language server ready";
                    "language" => language,
                    "server" => name,
                    "generation" => generation
                );
                self.lsp_servers.insert(
                    language.clone(),
                    ServerState {
                        name: name.clone(),
                        generation,
                        encoding,
                        sync,
                        capabilities,
                    },
                );
                // Buffers opened before the handshake are announced now, in
                // coordinates that are finally known to be right.
                let buffers: Vec<usize> = (0..self.buffers.len())
                    .filter(|buffer_id| {
                        !self.closed_buffers.contains(buffer_id)
                            && self.language_of(*buffer_id).as_deref() == Some(language.as_str())
                    })
                    .collect();
                for buffer_id in buffers {
                    self.lsp_touch(buffer_id);
                }
                // A normal handshake is routine lifecycle, not retained
                // feedback. Explicit :lsp-status output is an INFO
                // notification; merely becoming ready stays silent.
            }
            LspEvent::Diagnostics {
                language,
                path,
                version,
                diagnostics,
            } => {
                let resolved = self.resolve_working_path(path.clone());
                let contained =
                    crate::path_safety::ensure_within_root(&self.project_root, &resolved).is_ok();
                let identity = contained
                    .then(|| workspace_edit_path_identity(&resolved).ok())
                    .flatten();
                let live = identity.as_ref().and_then(|identity| {
                    self.lsp_documents.values().find(|document| {
                        document.language == language
                            && workspace_edit_path_identity(&document.path)
                                .is_ok_and(|candidate| candidate == *identity)
                    })
                });
                if let Some(live) = live.filter(|document| {
                    !document.desynced && version.is_none_or(|version| version == document.version)
                }) {
                    self.diagnostics
                        .set(&language, live.path.clone(), diagnostics);
                }
            }
            LspEvent::Status { message, error } => {
                if error {
                    self.error_from("LSP", "Language server error", message);
                } else {
                    self.info_from("LSP", "Language server status", message);
                }
            }
            LspEvent::Stopped { language, message } => {
                // The boundary that knows both which server went away and what
                // editor state went with it. Nothing below reports it again.
                crate::log_warn!(
                    "lsp",
                    "language server stopped: {message}";
                    "language" => language
                );
                // Diagnostics with no server behind them are claims about the
                // code that nothing will ever correct, so they go with it.
                self.lsp_servers.remove(&language);
                self.pending_lsp_replies.retain(|command| {
                    !matches!(command, LspCommand::EditApplied { language: pending, .. } if pending == &language)
                });
                self.lsp_documents
                    .retain(|_, document| document.language != language);
                self.diagnostics.clear_language(&language);
                if self.completion.as_ref().is_some_and(|completion| {
                    self.language_of(completion.buffer).as_deref() == Some(language.as_str())
                }) {
                    self.completion = None;
                }
                if self.language_of(self.active().buffer).as_deref() == Some(language.as_str()) {
                    self.signature = None;
                    self.hover = None;
                }
                if self.lsp_action_source.as_ref().is_some_and(|source| {
                    self.language_of(source.buffer).as_deref() == Some(language.as_str())
                }) {
                    let action_list_visible = self
                        .list_actions
                        .iter()
                        .any(|action| matches!(action, ListAction::CodeAction(_)));
                    self.lsp_action_source = None;
                    self.lsp_actions.clear();
                    if action_list_visible {
                        self.list = None;
                        self.list_actions.clear();
                    }
                }
                self.error_from("LSP", "Language server stopped", message);
            }
            LspEvent::Restarted { language } => {
                crate::log_info!("lsp", "language server restarted"; "language" => language);
                self.lsp_servers.remove(&language);
                self.pending_lsp_replies.retain(|command| {
                    !matches!(command, LspCommand::EditApplied { language: pending, .. } if pending == &language)
                });
                self.lsp_documents
                    .retain(|_, document| document.language != language);
                self.diagnostics.clear_language(&language);
                if self.completion.as_ref().is_some_and(|completion| {
                    self.language_of(completion.buffer).as_deref() == Some(language.as_str())
                }) {
                    self.completion = None;
                }
                if self.language_of(self.active().buffer).as_deref() == Some(language.as_str()) {
                    self.signature = None;
                    self.hover = None;
                }
                if self.lsp_action_source.as_ref().is_some_and(|source| {
                    self.language_of(source.buffer).as_deref() == Some(language.as_str())
                }) {
                    let action_list_visible = self
                        .list_actions
                        .iter()
                        .any(|action| matches!(action, ListAction::CodeAction(_)));
                    self.lsp_action_source = None;
                    self.lsp_actions.clear();
                    if action_list_visible {
                        self.list = None;
                        self.list_actions.clear();
                    }
                }
            }
            LspEvent::ApplyEdit {
                language,
                generation,
                encoding,
                id,
                edits,
                skipped,
            } => {
                if self
                    .lsp_servers
                    .get(&language)
                    .is_none_or(|server| server.generation != generation)
                {
                    return;
                }
                let guards = self.lsp_document_guards();
                let outcome = self.apply_document_edits(edits, None, Some(&guards), encoding, None);
                match outcome {
                    Ok(summary) => {
                        self.status(edit_summary("applied", summary, skipped));
                        self.lsp_reply(LspCommand::EditApplied {
                            language,
                            generation,
                            id,
                            applied: true,
                        });
                    }
                    Err(error) => {
                        self.error_from("LSP", "Workspace edit failed", error);
                        self.lsp_reply(LspCommand::EditApplied {
                            language,
                            generation,
                            id,
                            applied: false,
                        });
                    }
                }
                self.report_new_registry_errors();
            }
            LspEvent::Response { token, response } => {
                let Some(tracked) = self.lsp_requests.remove(&token) else {
                    // A response to a request the editor stopped caring about,
                    // such as a completion the person typed past.
                    return;
                };
                if tracked.cancelled {
                    return;
                }
                self.apply_lsp_response(tracked, response);
            }
        }
    }

    fn apply_lsp_response(&mut self, tracked: TrackedRequest, response: Response) {
        if let Response::Failed(reason) = response {
            if let PendingRequest::Completion {
                explicit_session: Some(session),
                ..
            } = tracked.pending
                && self.explicit_completion_session() == Some(session)
            {
                self.completion = None;
            }
            self.error(reason);
            return;
        }
        if tracked.pending.source_revision_must_match()
            && self
                .buffers
                .get(tracked.buffer)
                .is_none_or(|buffer| buffer.revision() != tracked.revision)
        {
            self.error("stale language-server response; the originating buffer changed");
            return;
        }
        if tracked.pending.transient_group().is_some() && self.active().buffer != tracked.buffer {
            return;
        }
        if matches!(
            tracked.pending,
            PendingRequest::Edits { .. } | PendingRequest::CodeActions
        ) && !self.lsp_document_guards_are_current(&tracked.documents)
        {
            self.error(
                "stale language-server response; another document changed, closed, or moved",
            );
            return;
        }
        let TrackedRequest {
            buffer,
            revision,
            documents,
            pending,
            cancelled: _,
            server,
        } = tracked;
        match (pending, response) {
            (PendingRequest::Goto { label }, Response::Locations(locations)) => {
                match locations.len() {
                    0 => self.status(format!("no {label} found")),
                    1 => {
                        if let Err(error) = self.jump_to(&locations[0]) {
                            self.error(error.to_string());
                        }
                    }
                    _ => self.open_location_picker(label, locations),
                }
            }
            (PendingRequest::Goto { label }, Response::Empty) => {
                self.status(format!("no {label} found"));
            }
            (PendingRequest::Hover, Response::Hover(text)) => {
                self.hover = Some(HoverState {
                    lines: text.lines().map(str::to_owned).collect(),
                });
            }
            (PendingRequest::Hover, Response::Empty) => {
                self.hover = None;
                self.status("no documentation here");
            }
            (
                PendingRequest::Completion {
                    buffer,
                    anchor,
                    explicit_session,
                },
                Response::Completions(items),
            ) => {
                self.show_completion(buffer, anchor, explicit_session, items);
            }
            (
                PendingRequest::Completion {
                    explicit_session: Some(session),
                    ..
                },
                Response::Empty,
            ) => {
                if self.explicit_completion_session() == Some(session)
                    && let Some(state) = self.completion.as_mut()
                {
                    state.items.clear();
                    state.selected = 0;
                }
            }
            (
                PendingRequest::Completion {
                    explicit_session: None,
                    ..
                },
                Response::Empty,
            ) => {
                if !self.path_completion_active() && self.explicit_completion_session().is_none() {
                    self.completion = None;
                }
            }
            (PendingRequest::Signature, Response::Signatures(signatures)) => {
                self.signature = (!signatures.is_empty()).then_some(SignatureState { signatures });
            }
            (PendingRequest::Signature, Response::Empty) => self.signature = None,
            (PendingRequest::Symbols { title, path }, Response::Symbols(symbols)) => {
                self.open_symbol_picker(title, &path, symbols);
            }
            (PendingRequest::Symbols { title, .. }, Response::Empty) => {
                self.status(format!("{title}: none"));
            }
            (PendingRequest::CodeActions, Response::Actions(actions)) => {
                self.open_action_picker(actions, buffer, revision, documents);
            }
            (PendingRequest::CodeActions, Response::Empty) => {
                self.status("no code actions here");
            }
            (
                PendingRequest::Edits { label, path },
                Response::Edits {
                    edits,
                    skipped,
                    encoding,
                },
            ) => {
                let fallback = (!path.as_os_str().is_empty()).then_some(path);
                match self.apply_document_edits(edits, fallback, Some(&documents), encoding, None) {
                    Ok(summary) => self.status(edit_summary(label, summary, skipped)),
                    Err(error) => self.error(error),
                }
                self.report_new_registry_errors();
            }
            (
                PendingRequest::Edits { label, path },
                Response::ActionEdits {
                    edits,
                    skipped,
                    encoding,
                    command,
                },
            ) => {
                let Some((language, generation)) = server else {
                    self.error("resolved code action lost its server provenance");
                    return;
                };
                if command.as_ref().is_some_and(|command| {
                    skipped > 0 || !self.action_command_supported(&language, generation, command)
                }) {
                    self.error(
                        "resolved code action command is unsupported or depends on file operations",
                    );
                    return;
                }
                let fallback = (!path.as_os_str().is_empty()).then_some(path);
                match self.apply_document_edits(
                    edits,
                    fallback,
                    Some(&documents),
                    encoding,
                    command.as_ref().map(|_| (language.as_str(), generation)),
                ) {
                    Ok(summary) => {
                        self.status(edit_summary(label, summary, skipped));
                        if let Some(command) = command {
                            if summary.2 {
                                self.send_action_command(buffer, language, generation, command);
                            } else {
                                self.error(
                                    "code action command not sent because its edits did not reach every language server",
                                );
                            }
                        }
                    }
                    Err(error) => self.error(error),
                }
                self.report_new_registry_errors();
            }
            (PendingRequest::Edits { label, .. }, Response::Empty) => {
                self.status(format!("nothing to change ({label})"));
            }
            (pending, response) => {
                // A server answered with a shape its own capabilities did not
                // promise. Reporting beats guessing.
                self.error(format!(
                    "unexpected {} response for {}",
                    response_name(&response),
                    pending.label()
                ));
            }
        }
    }

    fn show_completion(
        &mut self,
        buffer: usize,
        anchor: Offset,
        explicit_session: Option<u64>,
        items: Vec<Completion>,
    ) {
        if self.path_completion_active() {
            return;
        }
        match explicit_session {
            Some(session) if self.explicit_completion_session() != Some(session) => return,
            None if self.explicit_completion_session().is_some() => return,
            _ => {}
        }
        if !matches!(self.mode, Mode::Insert | Mode::Replace) || self.active().buffer != buffer {
            self.completion = None;
            return;
        }
        if items.is_empty() {
            if let Some(state) = self.completion.as_mut()
                && explicit_session.is_some()
            {
                state.items.clear();
                state.selected = 0;
            } else {
                self.completion = None;
            }
            return;
        }
        // Whatever was typed while the request was in flight becomes the
        // initial filter, so a fast typist does not see a stale list.
        let head = self.active().head();
        let filter = if head > anchor {
            self.buffers[buffer].slice(anchor, head)
        } else {
            String::new()
        };
        self.completion = Some(CompletionState {
            items,
            selected: 0,
            buffer,
            anchor,
            filter,
            source: CompletionSource::Language,
            explicit_session,
        });
        if explicit_session.is_none()
            && self
                .completion
                .as_ref()
                .is_some_and(|state| state.visible_indices().is_empty())
        {
            self.completion = None;
        }
    }

    pub(super) fn path_completion_active(&self) -> bool {
        self.completion
            .as_ref()
            .is_some_and(|state| state.source == CompletionSource::Path)
    }

    fn open_location_picker(&mut self, label: &'static str, locations: Vec<crate::lsp::Location>) {
        let mut items = Vec::with_capacity(locations.len());
        let mut actions = Vec::with_capacity(locations.len());
        for (index, location) in locations.into_iter().enumerate() {
            items.push(PickerItem::new(
                format!(
                    "{}:{}",
                    display_path(&location.path),
                    location.range.start.line + 1
                ),
                self.location_preview(&location),
                index,
            ));
            actions.push(ListAction::Jump(location));
        }
        self.list_actions = actions;
        let title = format!("{}{label}", label[..1].to_uppercase());
        self.list = Some(
            ListPicker::new(format!("{}{}", &title[..1], &title[1..]), items)
                .with_primary_action("jump"),
        );
    }

    /// The source line a location points at, when the file is already open.
    ///
    /// Deliberately does not read from disk: a picker row is not worth an
    /// unbounded number of blocking file reads on the render path.
    fn location_preview(&self, location: &crate::lsp::Location) -> String {
        self.buffers
            .iter()
            .enumerate()
            .find(|(index, buffer)| {
                !self.closed_buffers.contains(index)
                    && buffer.path.as_deref() == Some(location.path.as_path())
            })
            .map(|(_, buffer)| buffer)
            .map(|buffer| buffer.line_string(location.range.start.line as usize))
            .map(|line| line.trim().to_owned())
            .unwrap_or_default()
    }

    fn open_symbol_picker(
        &mut self,
        title: &'static str,
        fallback: &Path,
        symbols: Vec<crate::lsp::SymbolEntry>,
    ) {
        if symbols.is_empty() {
            self.status(format!("{title}: none"));
            return;
        }
        let mut items = Vec::with_capacity(symbols.len());
        let mut actions = Vec::with_capacity(symbols.len());
        for (index, symbol) in symbols.into_iter().enumerate() {
            let detail = if symbol.container.is_empty() {
                symbol.kind.to_owned()
            } else {
                format!("{} · {}", symbol.kind, symbol.container)
            };
            items.push(PickerItem::new(symbol.name, detail, index));
            let mut location = symbol.location;
            // Hierarchical document symbols carry no URI; they are always in
            // the document that was asked about.
            if location.path.as_os_str().is_empty() {
                location.path = fallback.to_path_buf();
            }
            actions.push(ListAction::Jump(location));
        }
        self.list_actions = actions;
        self.list = Some(ListPicker::new(title, items).with_primary_action("jump"));
    }

    pub(super) fn open_action_picker(
        &mut self,
        actions: Vec<ActionEntry>,
        buffer: usize,
        revision: u64,
        documents: HashMap<PathBuf, (usize, u64)>,
    ) {
        if actions.is_empty() {
            self.status("no code actions here");
            return;
        }
        let items = actions
            .iter()
            .enumerate()
            .map(|(index, action)| PickerItem::new(action.title.clone(), "code action", index))
            .collect();
        self.list_actions = (0..actions.len()).map(ListAction::CodeAction).collect();
        self.lsp_actions = actions;
        let language = self.language_of(buffer).unwrap_or_default();
        let generation = self
            .lsp_servers
            .get(&language)
            .map_or(0, |server| server.generation);
        self.lsp_action_source = Some(super::ActionSource {
            buffer,
            revision,
            documents,
            language,
            generation,
        });
        self.list = Some(ListPicker::new("Code actions", items).with_primary_action("apply"));
    }

    /// Runs a chosen code action.
    ///
    /// An action may carry its edit inline, need a `codeAction/resolve` round
    /// trip first, or be a command the server runs itself and then asks the
    /// editor to apply. All three paths end at `apply_document_edits`.
    pub(super) fn run_code_action(&mut self, index: usize) {
        let Some(source) = self.lsp_action_source.clone() else {
            self.error("stale code action; the originating buffer changed");
            return;
        };
        let source_buffer = source.buffer;
        let source_revision = source.revision;
        let documents = source.documents;
        let language = source.language;
        let generation = source.generation;
        if self
            .buffers
            .get(source_buffer)
            .is_none_or(|candidate| candidate.revision() != source_revision)
        {
            self.error("stale code action; the originating buffer changed");
            return;
        }
        if self.closed_buffers.contains(&source_buffer) {
            self.error("stale code action; the originating buffer changed");
            return;
        }
        let current_generation = self
            .lsp_servers
            .get(&language)
            .map(|server| server.generation);
        if current_generation != Some(generation) {
            self.error("stale code action; the language server restarted");
            return;
        }
        if self
            .lsp_documents
            .get(&source_buffer)
            .is_none_or(|document| document.language != language)
        {
            self.error("stale code action; the document changed language ownership");
            return;
        }
        if !self.lsp_document_guards_are_current(&documents) {
            self.error("stale code action; another language-server document changed");
            return;
        }
        let Some(entry) = self.lsp_actions.get(index).cloned() else {
            return;
        };
        let encoding = self.encoding_for(&language);
        match entry.action().clone() {
            crate::lsp::CodeActionOrCommand::Command(command) => {
                self.send_action_command(source_buffer, language, generation, command);
            }
            crate::lsp::CodeActionOrCommand::CodeAction(action) if action.disabled.is_some() => {
                self.error(format!(
                    "code action is disabled: {}",
                    action.disabled.unwrap().reason
                ));
            }
            crate::lsp::CodeActionOrCommand::CodeAction(action) => {
                let command = action.command.clone();
                if command.as_ref().is_some_and(|command| {
                    !self.action_command_supported(&language, generation, command)
                }) {
                    self.error("code action contains an unadvertised command");
                    return;
                }
                match action.edit.clone() {
                    Some(edit) => {
                        match crate::lsp::flatten_edit(edit) {
                            Ok((_, skipped)) if command.is_some() && skipped > 0 => {
                                self.error(
                                    "code action command depends on unsupported file operations",
                                );
                            }
                            Ok((edits, skipped)) => match self.apply_document_edits(
                                edits,
                                None,
                                Some(&documents),
                                encoding,
                                command.as_ref().map(|_| (language.as_str(), generation)),
                            ) {
                                Ok(summary) => {
                                    self.status(edit_summary("applied", summary, skipped));
                                    if let Some(command) = command {
                                        if summary.2 {
                                            self.send_action_command(
                                                source_buffer,
                                                language,
                                                generation,
                                                command,
                                            );
                                        } else {
                                            self.error(
                                                "code action command not sent because its edits did not reach every language server",
                                            );
                                        }
                                    }
                                }
                                Err(error) => self.error(error),
                            },
                            Err(error) => self.error(error),
                        }
                        self.report_new_registry_errors();
                    }
                    None => match command {
                        Some(command) => {
                            self.send_action_command(source_buffer, language, generation, command)
                        }
                        None => self.lsp_request_from(
                            source_buffer,
                            source_revision,
                            RequestKind::ResolveCodeAction(Box::new(action)),
                            PendingRequest::Edits {
                                label: "applied",
                                path: PathBuf::new(),
                            },
                        ),
                    },
                }
            }
        }
    }

    /// Applies per-file edits, one transaction per file so each is a single
    /// undo step.
    ///
    /// Files that are not open are opened rather than written directly: an
    /// edit the person has not seen and has not saved is one they can still
    /// undo or discard.
    fn apply_document_edits(
        &mut self,
        edits: Vec<DocumentEdit>,
        fallback: Option<PathBuf>,
        guards: Option<&HashMap<PathBuf, (usize, u64)>>,
        encoding: Encoding,
        command_server: Option<(&str, u64)>,
    ) -> Result<(usize, usize, bool), String> {
        struct PlannedEdit {
            target: PlannedTarget,
            transaction: Transaction,
            edit_count: usize,
        }

        enum PlannedTarget {
            Existing {
                buffer_id: usize,
                staged: Buffer,
            },
            New {
                staged: Buffer,
                syntax: Option<DocumentSyntax>,
            },
        }

        struct GroupedEdit {
            path: PathBuf,
            identity: PathBuf,
            version: Option<i32>,
            edits: Vec<lsp_types::TextEdit>,
        }

        let mut grouped = Vec::<GroupedEdit>::new();
        let mut grouped_paths = HashMap::<PathBuf, usize>::new();
        for document in edits {
            let path = if document.path.as_os_str().is_empty() {
                match &fallback {
                    Some(path) => path.clone(),
                    None => {
                        return Err("the language server did not say which file to change".into());
                    }
                }
            } else {
                document.path
            };
            if document.edits.is_empty() {
                continue;
            }
            // A language server names the files it wants changed, and nothing
            // in the protocol keeps those names inside the project. The module
            // already refuses server-driven creates, renames, and deletes for
            // the same reason; this is the same boundary applied to the text
            // edits themselves, so a rename or code action cannot reach
            // `~/.bashrc` through a `file://` URI.
            let resolved = self.resolve_working_path(path.clone());
            if crate::path_safety::ensure_within_root(&self.project_root, &resolved).is_err() {
                return Err(format!(
                    "{} refused: it is outside {}",
                    display_path(&path),
                    display_path(&self.project_root)
                ));
            }
            let identity = workspace_edit_path_identity(&resolved)
                .map_err(|error| format!("cannot resolve {}: {error}", display_path(&path)))?;

            if let Some(group) = grouped_paths.get(&identity).copied() {
                let grouped = &mut grouped[group];
                if let (Some(left), Some(right)) = (grouped.version, document.version)
                    && left != right
                {
                    return Err(format!(
                        "{} has conflicting language-server versions {left} and {right}",
                        display_path(&path)
                    ));
                }
                grouped.version = grouped.version.or(document.version);
                grouped.edits.extend(document.edits);
            } else {
                grouped_paths.insert(identity.clone(), grouped.len());
                grouped.push(GroupedEdit {
                    path: resolved,
                    identity,
                    version: document.version,
                    edits: document.edits,
                });
            }
        }

        let mut planned = Vec::with_capacity(grouped.len());
        for document in grouped {
            let path = document.path;
            let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
                (!self.closed_buffers.contains(&index)
                    && buffer.path.as_deref().is_some_and(|candidate| {
                        workspace_edit_path_identity(candidate)
                            .is_ok_and(|identity| identity == document.identity)
                    }))
                .then_some(index)
            });
            if let Some(guards) = guards.filter(|guards| !guards.is_empty()) {
                match (guards.get(&document.identity), existing) {
                    (Some((guarded_buffer, guarded_revision)), Some(buffer_id))
                        if *guarded_buffer == buffer_id
                            && self.buffers[buffer_id].revision() == *guarded_revision
                            && self.buffers[buffer_id].path.as_deref().is_some_and(|path| {
                                workspace_edit_path_identity(path)
                                    .is_ok_and(|current| current == document.identity)
                            }) => {}
                    (Some(_), _) => {
                        return Err(format!(
                            "{} changed, closed, or moved since the language-server request",
                            display_path(&path)
                        ));
                    }
                    (None, Some(_)) => {
                        return Err(format!(
                            "{} opened since the language-server request",
                            display_path(&path)
                        ));
                    }
                    (None, None) => {}
                }
            }
            let buffer_id = if let Some(expected) = document.version {
                let Some(buffer_id) = existing.filter(|buffer_id| {
                    self.lsp_documents.get(buffer_id).is_some_and(|state| {
                        state.version == expected
                            && workspace_edit_path_identity(&state.path)
                                .is_ok_and(|identity| identity == document.identity)
                    })
                }) else {
                    return Err(format!(
                        "{} changed since language-server version {expected}",
                        display_path(&path)
                    ));
                };
                Some(buffer_id)
            } else {
                existing
            };
            let mut staged = match buffer_id {
                Some(buffer_id) => self.buffers[buffer_id].clone(),
                None => open_or_new(&path, self.config.editor.show_hidden_files)
                    .map_err(|error| error.to_string())?,
            };
            if staged.is_read_only() {
                return Err(format!("{} is read-only", display_path(&path)));
            }
            let mut changes = document
                .edits
                .iter()
                .map(|edit| {
                    let (from, to) = checked_lsp_range(staged.text(), edit.range, encoding)
                        .ok_or_else(|| {
                            format!(
                                "{} has an invalid language-server edit range",
                                display_path(&path)
                            )
                        })?;
                    Ok(Change::new(from, to, edit.new_text.clone()))
                })
                .collect::<Result<Vec<_>, String>>()?;
            changes.sort_by_key(|change| (change.from, change.to));
            if changes.windows(2).any(|pair| {
                pair[1].from < pair[0].to
                    || (pair[0].from == pair[0].to
                        && pair[1].from == pair[1].to
                        && pair[0].from == pair[1].from)
            }) {
                return Err(format!(
                    "{} has overlapping language-server edits",
                    display_path(&path)
                ));
            }
            let transaction = Transaction::new(changes);
            if !staged.apply(&transaction) {
                continue;
            }
            let target = match buffer_id {
                Some(buffer_id) => PlannedTarget::Existing { buffer_id, staged },
                None => {
                    let syntax = parse_buffer(&staged, &self.registry);
                    PlannedTarget::New { staged, syntax }
                }
            };
            planned.push(PlannedEdit {
                target,
                transaction,
                edit_count: document.edits.len(),
            });
        }

        let changed_files = planned.len();
        let changed_edits = planned.iter().map(|edit| edit.edit_count).sum();
        let mut synchronized = true;
        for edit in planned {
            match edit.target {
                PlannedTarget::Existing { buffer_id, staged } => {
                    let language_before = buffer_language(&self.buffers[buffer_id], &self.registry);
                    let watched = self.syntax[buffer_id].is_some()
                        || self.lsp_documents.contains_key(&buffer_id);
                    let before = watched.then(|| self.buffers[buffer_id].text().clone());
                    self.buffers[buffer_id] = staged;
                    let delivered = self.reconcile_applied_transaction(
                        buffer_id,
                        language_before,
                        before.as_ref(),
                        &edit.transaction,
                    );
                    if let Some((language, generation)) = command_server {
                        synchronized &= delivered
                            && self.lsp_documents.get(&buffer_id).is_some_and(|document| {
                                document.language == language && !document.desynced
                            })
                            && self.lsp_servers.get(language).is_some_and(|server| {
                                server.generation == generation
                                    && server.sync.change != ChangeSync::None
                            });
                    }
                }
                PlannedTarget::New { staged, syntax } => {
                    self.buffers.push(staged);
                    self.syntax.push(syntax);
                    let buffer_id = self.buffers.len() - 1;
                    let opened = self.lsp_touch(buffer_id);
                    if let Some((language, generation)) = command_server {
                        synchronized &= opened
                            && self.lsp_documents.get(&buffer_id).is_some_and(|document| {
                                document.language == language && !document.desynced
                            })
                            && self.lsp_servers.get(language).is_some_and(|server| {
                                server.generation == generation && server.sync.open_close
                            });
                    }
                }
            }
        }
        Ok((changed_files, changed_edits, synchronized))
    }

    /// The buffer holding `path`, opening it if necessary without stealing the
    /// active pane.
    /// Opens a location and puts the primary selection on it.
    fn jump_to(&mut self, location: &crate::lsp::Location) -> Result<()> {
        self.open_file(location.path.clone())?;
        let buffer_id = self.active().buffer;
        let text = self.buffers[buffer_id].text();
        let from = from_lsp_position(text, location.range.start, location.encoding);
        let to = from_lsp_position(text, location.range.end, location.encoding);
        let pane = self.active_mut();
        let selection = if to > from {
            Selection::single(Range::new(from, to.saturating_sub(1)))
        } else {
            Selection::point(from)
        };
        pane.replace_selection(selection);
        pane.mark_selection_semantics(SelectionSemantics::Runyte);
        pane.preserve_scroll = false;
        Ok(())
    }

    fn jump_to_workspace_search_target(&mut self, found: &WorkspaceSearchTarget) -> Result<()> {
        self.open_file(found.path.clone())?;
        let buffer = self.active_buffer();
        let row = found.row.min(buffer.last_row());
        let start = buffer.line_to_offset(row) + found.column.min(buffer.line_len(row));
        let end = (start + found.length).min(buffer.line_to_offset(row) + buffer.line_len(row));
        let selection = if end > start {
            Selection::single(Range::new(start, end - 1))
        } else {
            Selection::point(start)
        };
        self.active_mut().replace_selection(selection);
        self.active_mut()
            .mark_selection_semantics(SelectionSemantics::Runyte);
        self.active_mut().preserve_scroll = false;
        Ok(())
    }

    pub(super) fn open_workspace_search_result(&mut self) -> Result<()> {
        let row = self.active_buffer().offset_to_row(self.active().head());
        let Some(target) = self
            .active_buffer()
            .workspace_search_target_at(row)
            .cloned()
        else {
            self.status("this workspace-search row is informational");
            return Ok(());
        };
        self.jump_to_workspace_search_target(&target)
    }

    /// Handles a key while the completion popup is open.
    ///
    /// Returns `false` when the key was not the popup's, in which case normal
    /// Insert-mode dispatch continues and the popup re-filters against what
    /// was typed.
    pub(super) fn handle_completion_key(&mut self, key: KeyStroke) -> bool {
        let directory_buffer = self.active_buffer().is_directory();
        let Some(state) = self.completion.as_mut() else {
            return false;
        };
        let source = state.source;
        let control = key.modifiers.contains(Modifiers::CONTROL);
        match (key.code, control) {
            (KeyCode::Escape, _) => {
                self.completion = None;
                // Word completion appears automatically during ordinary
                // prose, so dismissing it must not interpose an extra modal
                // step between Insert and Normal. Let the registry handle
                // the same Escape after closing the popup.
                if source == CompletionSource::Word {
                    return false;
                }
                // A slash in an explorer row is both path syntax and the
                // directory-kind marker, so ordinary editing can open path
                // completion there without an explicit request. Escape's
                // primary meaning in this modal buffer remains leaving
                // Insert mode; let the registry handle it after dismissing
                // the popup instead of requiring a second press.
                !directory_buffer
            }
            (KeyCode::Char('c'), true) => {
                self.completion = None;
                true
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                state.step(true);
                true
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) => {
                state.step(false);
                true
            }
            (KeyCode::Tab, _) => {
                self.accept_completion();
                true
            }
            // A completion popup can open on its own — Word for any
            // three-character prefix, Language after `.`/`:`, Path after
            // `/` — so treating Enter as acceptance would make finishing a
            // line, or opening one after a path, a lottery between the
            // ordinary newline and an unwanted candidate. Every source
            // accepts only with Tab; Enter always dismisses and falls
            // through to its usual newline, the same as if nothing had
            // opened.
            (KeyCode::Enter, _) => {
                self.completion = None;
                false
            }
            _ => false,
        }
    }

    /// Inserts the selected completion as a single transaction, so an added
    /// import and the word it justifies undo together.
    fn accept_completion(&mut self) {
        let Some(state) = self.completion.take() else {
            return;
        };
        let source = state.source;
        let Some(item) = state.selected_item().cloned() else {
            return;
        };
        let buffer_id = state.buffer;
        if self.active().buffer != buffer_id {
            return;
        }
        let encoding = self
            .language_of(buffer_id)
            .map_or(Encoding::default(), |language| self.encoding_for(&language));
        let head = self.active().head();
        let mut changes = Vec::with_capacity(item.additional.len() + 1);
        for edit in &item.additional {
            let Some((from, to)) =
                checked_lsp_range(self.buffers[buffer_id].text(), edit.range, encoding)
            else {
                self.error("completion has an invalid language-server edit range");
                return;
            };
            changes.push(Change::new(from, to, edit.new_text.clone()));
        }
        // A server-supplied range is authoritative: only the server knows how
        // much of what was typed it means to replace.
        let primary = match &item.edit {
            Some((range, text)) => {
                let Some((from, to)) =
                    checked_lsp_range(self.buffers[buffer_id].text(), *range, encoding)
                else {
                    self.error("completion has an invalid language-server edit range");
                    return;
                };
                Change::new(from, to.max(head), text.clone())
            }
            None => Change::new(state.anchor.min(head), head, item.insert.clone()),
        };
        let primary_end = primary.to;
        changes.push(primary);
        changes.sort_by_key(|change| (change.from, change.to));
        if changes.windows(2).any(|pair| {
            pair[1].from < pair[0].to
                || (pair[0].from == pair[0].to
                    && pair[1].from == pair[1].to
                    && pair[0].from == pair[1].from)
        }) {
            self.error("completion has overlapping language-server edits");
            return;
        }
        let transaction = Transaction::new(changes);
        // Carets follow the inserted text rather than sitting before it.
        let end = transaction.map_offset(primary_end, Assoc::After);
        if self.apply_to_buffer(buffer_id, &transaction) {
            let clamped = self.buffers[buffer_id].clamp_offset(end, true);
            self.active_mut()
                .replace_selection(Selection::point(clamped));
        }
        self.report_new_registry_errors();
        self.signature = None;
        if source == CompletionSource::Path && item.insert.ends_with('/') {
            self.path_completion();
        }
    }

    /// Handles a key while a result picker is open.
    pub(super) fn handle_list_key(&mut self, key: KeyStroke) -> Result<()> {
        if self.terminal_action_menu.is_some() {
            return self.handle_terminal_action_key(key);
        }
        #[cfg(unix)]
        if self.session_action_menu.is_some() {
            return self.handle_session_action_key(key);
        }
        if self.buffer_action_menu.is_some() {
            return self.handle_buffer_action_key(key);
        }
        if self
            .list
            .as_ref()
            .is_some_and(|list| list.purpose == ListPurpose::Report)
        {
            let page = 10;
            let control = key.modifiers.contains(Modifiers::CONTROL);
            match (key.code, control) {
                (KeyCode::Escape, _) | (KeyCode::Char('c'), true) => self.list = None,
                (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                    self.list.as_mut().unwrap().report_down();
                }
                (KeyCode::Up, _) | (KeyCode::Char('p'), true) => {
                    self.list.as_mut().unwrap().report_up();
                }
                (KeyCode::PageDown, _) | (KeyCode::Char('d'), true) => {
                    self.list.as_mut().unwrap().report_page_down(page);
                }
                (KeyCode::PageUp, _) | (KeyCode::Char('u'), true) => {
                    self.list.as_mut().unwrap().report_page_up(page);
                }
                (KeyCode::Home, _) => self.list.as_mut().unwrap().report_first(),
                (KeyCode::End, _) => self.list.as_mut().unwrap().report_last(),
                _ => {}
            }
            return Ok(());
        }
        let page = 10;
        let control = key.modifiers.contains(Modifiers::CONTROL);
        let mut preview_changed = false;
        match (key.code, control) {
            (KeyCode::Escape, _) | (KeyCode::Char('c'), true) => {
                if self.settings_view.is_some() {
                    self.cancel_settings_picker();
                } else {
                    self.list = None;
                }
                return Ok(());
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                self.list.as_mut().unwrap().down();
                preview_changed = true;
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) | (KeyCode::BackTab, _) => {
                self.list.as_mut().unwrap().up();
                preview_changed = true;
            }
            (KeyCode::Tab, _) => {
                #[cfg(unix)]
                if matches!(self.selected_list_action(), Some(ListAction::Workspace(_))) {
                    self.open_session_actions();
                    return Ok(());
                }
                if matches!(self.selected_list_action(), Some(ListAction::Terminal(_))) {
                    self.open_terminal_actions();
                    return Ok(());
                }
                if matches!(self.selected_list_action(), Some(ListAction::Buffer(_))) {
                    self.open_buffer_actions();
                } else if self.list.as_ref().is_some_and(ListPicker::has_tags) {
                    self.list.as_mut().unwrap().cycle_tag();
                    preview_changed = true;
                } else {
                    self.list.as_mut().unwrap().down();
                    preview_changed = true;
                }
            }
            (KeyCode::PageDown, _) | (KeyCode::Char('d'), true) => {
                self.list.as_mut().unwrap().page_down(page);
                preview_changed = true;
            }
            (KeyCode::PageUp, _) | (KeyCode::Char('u'), true) => {
                self.list.as_mut().unwrap().page_up(page);
                preview_changed = true;
            }
            (KeyCode::Home, _) => {
                self.list.as_mut().unwrap().first();
                preview_changed = true;
            }
            (KeyCode::End, _) => {
                self.list.as_mut().unwrap().last();
                preview_changed = true;
            }
            (KeyCode::Char('t'), true)
                if self.list.as_ref().is_some_and(ListPicker::has_preview) =>
            {
                let picker = self.list.as_mut().unwrap();
                picker.show_preview = !picker.show_preview;
            }
            (KeyCode::Backspace, _) => {
                self.list.as_mut().unwrap().pop_filter();
                preview_changed = true;
            }
            (KeyCode::Delete, _) => {
                self.list.as_mut().unwrap().clear_filter();
                preview_changed = true;
            }
            (KeyCode::Enter, _) => {
                self.activate_list_selection()?;
            }
            (KeyCode::Char(character), false)
                if !key.modifiers.intersects(Modifiers::ALT | Modifiers::SUPER) =>
            {
                #[cfg(unix)]
                if ('1'..='9').contains(&character) && self.session_number_shortcut_is_armed() {
                    self.attach_numbered_session(character);
                    return Ok(());
                }
                if self
                    .list
                    .as_ref()
                    .is_some_and(ListPicker::accepts_filter_input)
                {
                    self.list.as_mut().unwrap().push_filter(character);
                    preview_changed = true;
                }
            }
            _ => {}
        }
        if preview_changed {
            self.preview_selected_setting_value();
            #[cfg(unix)]
            self.request_selected_workspace_preview();
        }
        Ok(())
    }

    /// Whether a digit in the session manager is a shortcut rather than text.
    ///
    /// Only while nothing has been typed. Runyte names workspaces `runyte`,
    /// `runyte-2`, `runyte-3` and their paths are full of digits, so a digit
    /// has to stay ordinary filter input the moment somebody is filtering;
    /// what it cannot be is the *first* thing typed, because that is the
    /// keystroke `Space Space 1` is made of. Clearing the filter arms it again,
    /// so
    /// the rule is the state of the filter rather than a mode to keep track of.
    #[cfg(unix)]
    fn session_number_shortcut_is_armed(&self) -> bool {
        self.list
            .as_ref()
            .is_some_and(|list| list.title.starts_with("Sessions") && list.filter.is_empty())
    }

    /// Attaches to the session a digit names, from the session manager.
    #[cfg(unix)]
    fn attach_numbered_session(&mut self, digit: char) {
        let Some(number) = digit.to_digit(10).map(|number| number as u8) else {
            return;
        };
        let Some(path) = self
            .workspace_rows
            .iter()
            .find(|row| row.number == Some(number))
            .map(|row| row.project_root.clone())
        else {
            self.error(format!("no session is numbered {number}"));
            return;
        };
        if !self.persistent_session {
            self.error("attaching sessions needs workspace.mode: persistent");
            return;
        }
        self.list = None;
        self.session_action_menu = None;
        if self.request_workspace_switch(path) {
            self.should_quit = true;
        }
    }

    #[cfg(unix)]
    fn open_session_actions(&mut self) {
        let Some(ListAction::Workspace(row)) = self.selected_list_action() else {
            return;
        };
        let Some(entry) = self.workspace_rows.get(row) else {
            return;
        };
        // Only what the row's own state can answer: stopping belongs to a
        // running session, forgetting the history record to a stopped one.
        let actions = if entry.running {
            vec![
                SessionAction::Open,
                SessionAction::Rename,
                SessionAction::Number,
                SessionAction::Close,
                SessionAction::ForceClose,
            ]
        } else {
            vec![
                SessionAction::Open,
                SessionAction::Rename,
                SessionAction::Number,
                SessionAction::Forget,
            ]
        };
        self.session_action_menu = Some(SessionActionMenu {
            row,
            actions,
            selected: 0,
            force_armed: false,
        });
    }

    #[cfg(unix)]
    fn handle_session_action_key(&mut self, key: KeyStroke) -> Result<()> {
        let control = key.modifiers.contains(Modifiers::CONTROL);
        match (key.code, control) {
            (KeyCode::Escape, _) | (KeyCode::Char('c'), true) | (KeyCode::Tab, _) => {
                self.session_action_menu = None;
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                let menu = self.session_action_menu.as_mut().unwrap();
                if !menu.actions.is_empty() {
                    menu.selected = (menu.selected + 1) % menu.actions.len();
                    menu.force_armed = false;
                }
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) | (KeyCode::BackTab, _) => {
                let menu = self.session_action_menu.as_mut().unwrap();
                if !menu.actions.is_empty() {
                    menu.selected = (menu.selected + menu.actions.len() - 1) % menu.actions.len();
                    menu.force_armed = false;
                }
            }
            (KeyCode::Enter, _) => {
                if self.session_action_menu.as_ref().is_some_and(|menu| {
                    menu.selected_action() == Some(SessionAction::ForceClose) && !menu.force_armed
                }) && self.session_action_menu.as_ref().is_some_and(|menu| {
                    self.workspace_rows
                        .get(menu.row)
                        .is_some_and(|row| row.running)
                }) {
                    self.session_action_menu.as_mut().unwrap().force_armed = true;
                    self.status(
                        "force close discards protected buffers, waiters, and live terminals; press Enter again to confirm",
                    );
                    return Ok(());
                }
                let chosen = self.session_action_menu.as_ref().and_then(|menu| {
                    let action = menu.selected_action()?;
                    let row = self.workspace_rows.get(menu.row)?;
                    Some((
                        row.project_root.clone(),
                        row.name.clone(),
                        row.running,
                        action,
                    ))
                });
                match chosen {
                    Some((_, _, _, SessionAction::Open)) => {
                        self.session_action_menu = None;
                        self.activate_list_selection()?;
                    }
                    Some((selector, name, _, SessionAction::Rename)) => {
                        self.list = None;
                        self.session_action_menu = None;
                        self.session_rename_target = Some(selector);
                        self.open_prompt(PromptKind::SessionRename);
                        self.command = name.unwrap_or_default();
                        self.command_cursor = self.command.chars().count();
                    }
                    Some((selector, _, _, SessionAction::Number)) => {
                        let current = self
                            .workspace_rows
                            .iter()
                            .find(|row| row.project_root == selector)
                            .and_then(|row| row.number);
                        self.list = None;
                        self.session_action_menu = None;
                        self.session_number_target = Some(selector);
                        self.open_prompt(PromptKind::SessionNumber);
                        // Prefilled with the number it already has, so the
                        // prompt shows what is being changed and an empty
                        // answer is a deliberate clearing rather than the
                        // state it opened in.
                        self.command = current.map(|number| number.to_string()).unwrap_or_default();
                        self.command_cursor = self.command.chars().count();
                    }
                    Some((selector, _, true, SessionAction::Close)) => self.stop_session(selector),
                    Some((_, _, false, SessionAction::Close)) => {
                        self.status("this session is already stopped")
                    }
                    Some((selector, _, true, SessionAction::ForceClose)) => {
                        self.stop_session_force(selector)
                    }
                    Some((_, _, false, SessionAction::ForceClose)) => {
                        self.status("this session is already stopped")
                    }
                    Some((selector, _, false, SessionAction::Forget)) => {
                        let _ = self.forget_workspace(selector);
                    }
                    Some((_, _, true, SessionAction::Forget)) => {
                        self.status("stop this session before forgetting it")
                    }
                    None => {}
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn open_terminal_actions(&mut self) {
        let Some(ListAction::Terminal(id)) = self.selected_list_action() else {
            return;
        };
        if self.terminals.get(id).is_none() {
            self.error("that terminal is gone");
            return;
        }
        self.terminal_action_menu = Some(TerminalActionMenu {
            id,
            actions: vec![
                TerminalAction::Show,
                TerminalAction::Rename,
                TerminalAction::Close,
                TerminalAction::Create,
            ],
            selected: 0,
            close_armed: false,
        });
    }

    fn handle_terminal_action_key(&mut self, key: KeyStroke) -> Result<()> {
        let control = key.modifiers.contains(Modifiers::CONTROL);
        match (key.code, control) {
            (KeyCode::Escape, _) | (KeyCode::Char('c'), true) | (KeyCode::Tab, _) => {
                self.terminal_action_menu = None;
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                let menu = self.terminal_action_menu.as_mut().unwrap();
                menu.selected = (menu.selected + 1) % menu.actions.len();
                menu.close_armed = false;
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) | (KeyCode::BackTab, _) => {
                let menu = self.terminal_action_menu.as_mut().unwrap();
                menu.selected = (menu.selected + menu.actions.len() - 1) % menu.actions.len();
                menu.close_armed = false;
            }
            (KeyCode::Enter, _) => {
                let (id, action, armed) = {
                    let menu = self.terminal_action_menu.as_ref().unwrap();
                    (menu.id, menu.selected_action(), menu.close_armed)
                };
                let Some(action) = action else {
                    return Ok(());
                };
                if action == TerminalAction::Close
                    && self.terminals.get(id).is_some_and(TerminalSession::live)
                    && self.active_terminal() != Some(id)
                    && !armed
                {
                    self.terminal_action_menu.as_mut().unwrap().close_armed = true;
                    self.status("closing ends this hidden live process; press Enter again");
                    return Ok(());
                }
                self.terminal_action_menu = None;
                match action {
                    TerminalAction::Show => {
                        self.list = None;
                        self.show_terminal(id);
                    }
                    TerminalAction::Rename => {
                        self.list = None;
                        self.show_terminal(id);
                        self.open_terminal_rename_prompt();
                    }
                    TerminalAction::Close => {
                        self.close_terminal_id(id);
                        if self.terminals.is_empty() {
                            self.list = None;
                        } else {
                            self.open_terminal_list();
                        }
                    }
                    TerminalAction::Create => {
                        self.list = None;
                        self.open_terminal(None);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_buffer_action_key(&mut self, key: KeyStroke) -> Result<()> {
        let control = key.modifiers.contains(Modifiers::CONTROL);
        match (key.code, control) {
            (KeyCode::Escape, _) | (KeyCode::Char('c'), true) | (KeyCode::Tab, _) => {
                self.buffer_action_menu = None;
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                let menu = self.buffer_action_menu.as_mut().unwrap();
                if !menu.actions.is_empty() {
                    menu.selected = (menu.selected + 1) % menu.actions.len();
                }
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) | (KeyCode::BackTab, _) => {
                let menu = self.buffer_action_menu.as_mut().unwrap();
                if !menu.actions.is_empty() {
                    menu.selected = (menu.selected + menu.actions.len() - 1) % menu.actions.len();
                }
            }
            (KeyCode::Enter, _) => {
                let chosen = self
                    .buffer_action_menu
                    .as_ref()
                    .and_then(|menu| menu.selected_action().map(|action| (menu.buffer, action)));
                if let Some((buffer, action)) = chosen {
                    self.run_buffer_action(buffer, action)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn selected_list_action(&self) -> Option<ListAction> {
        self.list
            .as_ref()
            .and_then(ListPicker::selected_item)
            .and_then(|item| self.list_actions.get(item.index))
            .cloned()
    }

    pub(super) fn open_context_actions(&mut self) -> bool {
        let actions = self
            .keymap
            .context_actions(self.key_binding_scope())
            .copied()
            .collect::<Vec<_>>();
        if actions.is_empty() {
            return false;
        }
        self.context_action_menu = Some(ContextActionMenu {
            actions,
            selected: 0,
        });
        self.grammar.reset();
        true
    }

    pub(super) fn handle_context_action_key(&mut self, key: KeyStroke) -> Result<()> {
        let control = key.modifiers.contains(Modifiers::CONTROL);
        match (key.code, control) {
            (KeyCode::Escape, _) | (KeyCode::Char('c'), true) | (KeyCode::Tab, _) => {
                self.context_action_menu = None;
            }
            (KeyCode::Down, false) | (KeyCode::Char('j'), false) => {
                let menu = self.context_action_menu.as_mut().unwrap();
                menu.selected = (menu.selected + 1) % menu.actions.len();
            }
            (KeyCode::Up, false) | (KeyCode::Char('k'), false) | (KeyCode::BackTab, _) => {
                let menu = self.context_action_menu.as_mut().unwrap();
                menu.selected = (menu.selected + menu.actions.len() - 1) % menu.actions.len();
            }
            (KeyCode::Enter, false) => {
                if let Some(action) = self
                    .context_action_menu
                    .as_ref()
                    .and_then(ContextActionMenu::selected_action)
                {
                    self.run_context_action(action)?;
                }
            }
            _ => {
                let pressed = key.canonical_for_binding();
                if let Some(action) = self.context_action_menu.as_ref().and_then(|menu| {
                    menu.actions
                        .iter()
                        .find(|action| action.mnemonic == pressed)
                        .copied()
                }) {
                    self.run_context_action(action)?;
                }
            }
        }
        Ok(())
    }

    fn run_context_action(&mut self, action: ContextAction) -> Result<()> {
        self.context_action_menu = None;
        let key = format!("Tab {}", action.mnemonic.label());
        let outcome = self.execute(action.target.invocation()?)?;
        self.report_completed_action(&key, action.description, outcome);
        Ok(())
    }

    pub(crate) fn path_popup_open(&self) -> bool {
        self.path_popup.is_some()
    }

    pub(crate) fn path_action_menu_open(&self) -> bool {
        self.path_action_menu.is_some()
    }

    /// `:path` — opens a read-only popup showing the active buffer's
    /// absolute path. File and directory buffers both keep it in `path`,
    /// the latter kept in step as the explorer navigates.
    pub(super) fn open_path_popup(&mut self) {
        match self.active_buffer().path.as_deref() {
            Some(path) => {
                self.path_popup = Some(PathPopup {
                    path: path.display().to_string(),
                });
            }
            None => self.error("buffer has no path"),
        }
    }

    pub(super) fn handle_path_popup_key(&mut self, key: KeyStroke) -> Result<()> {
        match key.code {
            KeyCode::Tab => {
                self.path_action_menu = Some(PathActionMenu {
                    actions: vec![PathClipboardTarget::System, PathClipboardTarget::Register],
                    selected: 0,
                });
            }
            KeyCode::Escape => {
                self.path_popup = None;
            }
            KeyCode::Char('c') if key.modifiers.contains(Modifiers::CONTROL) => {
                self.path_popup = None;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn handle_path_action_key(&mut self, key: KeyStroke) -> Result<()> {
        let control = key.modifiers.contains(Modifiers::CONTROL);
        match (key.code, control) {
            (KeyCode::Escape, _) | (KeyCode::Char('c'), true) | (KeyCode::Tab, _) => {
                self.path_action_menu = None;
            }
            (KeyCode::Down, false) | (KeyCode::Char('j'), false) => {
                let menu = self.path_action_menu.as_mut().unwrap();
                menu.selected = (menu.selected + 1) % menu.actions.len();
            }
            (KeyCode::Up, false) | (KeyCode::Char('k'), false) | (KeyCode::BackTab, _) => {
                let menu = self.path_action_menu.as_mut().unwrap();
                menu.selected = (menu.selected + menu.actions.len() - 1) % menu.actions.len();
            }
            (KeyCode::Enter, false) => {
                if let Some(action) = self
                    .path_action_menu
                    .as_ref()
                    .and_then(PathActionMenu::selected_action)
                {
                    self.run_path_action(action);
                }
            }
            _ => {
                if let KeyCode::Char(character) = key.code
                    && let Some(action) = self.path_action_menu.as_ref().and_then(|menu| {
                        menu.actions
                            .iter()
                            .find(|action| action.mnemonic() == character)
                            .copied()
                    })
                {
                    self.run_path_action(action);
                }
            }
        }
        Ok(())
    }

    fn run_path_action(&mut self, target: PathClipboardTarget) {
        let Some(popup) = self.path_popup.take() else {
            self.path_action_menu = None;
            return;
        };
        self.path_action_menu = None;
        match target {
            PathClipboardTarget::System => match self.ports.clipboard().write(&popup.path) {
                Ok(()) => self.status("copied path to system clipboard"),
                Err(error) => self.error(error.to_string()),
            },
            PathClipboardTarget::Register => {
                let selected = self.selected_register;
                self.write_selected_register(Register {
                    text: popup.path,
                    linewise: false,
                    directory: None,
                });
                if selected == '"' {
                    self.status("copied path to register");
                } else {
                    self.status(format!("copied path to register {selected}"));
                }
            }
        }
    }

    fn open_buffer_actions(&mut self) {
        let Some(ListAction::Buffer(buffer)) = self.selected_list_action() else {
            return;
        };
        let actions = self.available_buffer_actions(buffer);
        if actions.is_empty() {
            if self.buffers[buffer].is_directory() {
                self.status("explorer buffers have no management actions here");
            } else {
                self.status("this modified buffer must be opened before it can be managed");
            }
            return;
        }
        self.buffer_action_menu = Some(BufferActionMenu {
            buffer,
            actions,
            selected: 0,
        });
    }

    fn available_buffer_actions(&self, buffer: usize) -> Vec<BufferAction> {
        let Some(buffer_state) = self.buffers.get(buffer) else {
            return Vec::new();
        };
        if self.closed_buffers.contains(&buffer) || buffer_state.is_directory() {
            return Vec::new();
        }
        if !buffer_state.dirty {
            return vec![BufferAction::Close];
        }
        match buffer_state.kind {
            BufferKind::File => vec![BufferAction::Save, BufferAction::Discard],
            BufferKind::Scratch | BufferKind::CommitMessage => {
                vec![BufferAction::Discard]
            }
            BufferKind::Virtual { .. }
            | BufferKind::Settings { .. }
            | BufferKind::Notifications { .. }
            | BufferKind::GitStatus
            | BufferKind::GitBranches
            | BufferKind::GitWorktrees
            | BufferKind::GitLog
            | BufferKind::GitBlame
            | BufferKind::GitStash
            | BufferKind::GitCommit { .. }
            | BufferKind::WorkspaceSearch { .. }
            | BufferKind::Help
            | BufferKind::Directory => Vec::new(),
        }
    }

    fn run_buffer_action(&mut self, buffer: usize, action: BufferAction) -> Result<()> {
        match action {
            BufferAction::Save => {
                self.buffer_action_menu = None;
                self.save_buffer(buffer, None, false)?;
                self.refresh_buffer_picker();
            }
            BufferAction::Discard => {
                self.buffer_action_menu = None;
                self.buffer_discard_confirmation = Some(buffer);
                self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
                self.status(format!(
                    "Discard changes to {}?\nEnter confirms.\nEscape cancels.",
                    self.buffers[buffer].display_name()
                ));
            }
            BufferAction::Close => self.close_buffer(buffer),
        }
        Ok(())
    }

    fn refresh_buffer_picker(&mut self) {
        let Some(picker) = self
            .list
            .as_ref()
            .filter(|picker| picker.purpose == ListPurpose::Manager)
        else {
            return;
        };
        let filter = picker.filter.clone();
        let selected = picker.selected;
        self.rebuild_buffer_picker(filter, selected);
    }

    pub(super) fn discard_buffer_changes(&mut self, buffer: usize) -> Result<()> {
        if self.closed_buffers.contains(&buffer) || self.buffers[buffer].is_directory() {
            self.error("this buffer cannot be discarded here");
            return Ok(());
        }
        let kind = self.buffers[buffer].kind.clone();
        match kind {
            BufferKind::File => {
                let language_before = buffer_language(&self.buffers[buffer], &self.registry);
                self.buffers[buffer].reload()?;
                self.resync_replaced_buffer(buffer, language_before);
            }
            BufferKind::CommitMessage => {
                // A blank commit message is not a thing anyone wants left
                // open, so discarding one abandons it outright.
                self.abandon_commit_message(buffer);
                return Ok(());
            }
            BufferKind::Scratch => {
                self.buffers[buffer].discard_changes_to("")?;
                self.clear_syntax_history(buffer);
                self.stale_syntax.remove(&buffer);
                self.syntax[buffer] = None;
            }
            BufferKind::Virtual { .. }
            | BufferKind::Settings { .. }
            | BufferKind::Notifications { .. }
            | BufferKind::GitStatus
            | BufferKind::GitBranches
            | BufferKind::GitWorktrees
            | BufferKind::GitLog
            | BufferKind::GitBlame
            | BufferKind::GitStash
            | BufferKind::GitCommit { .. }
            | BufferKind::WorkspaceSearch { .. }
            | BufferKind::Help => {
                self.error("virtual buffers have no changes to discard");
                return Ok(());
            }
            BufferKind::Directory => {
                self.error("this buffer cannot be discarded here");
                return Ok(());
            }
        }
        self.normalize_buffer(buffer);
        self.status(format!(
            "discarded changes to {}",
            self.buffers[buffer].display_name()
        ));
        self.report_new_registry_errors();
        self.refresh_buffer_picker();
        Ok(())
    }

    /// Closes the active buffer while preserving every pane in the layout.
    ///
    /// A plain close refuses unsaved text; losing it requires the deliberately
    /// command-only `:close!`. Every pane that showed the retired identity
    /// returns to its own most recently displayed live buffer, or uses another
    /// live buffer and finally a new scratch when no history remains.
    pub(super) fn close_active_buffer(&mut self, force: bool) {
        if self.active_terminal().is_some() {
            self.error("a terminal is not a buffer; close it explicitly in :terminals");
            return;
        }
        let buffer = self.active().buffer;
        if self.buffers[buffer].dirty && !force {
            self.error("modified buffer; use :close! to discard its unsaved changes");
            return;
        }
        if force {
            self.close_buffer_discarding(buffer);
        } else {
            self.close_buffer_returning_from_commit(buffer);
        }
    }

    /// Closes a buffer whose unsaved text the person has agreed to lose.
    pub(super) fn close_buffer_discarding(&mut self, buffer: usize) {
        if self.closed_buffers.contains(&buffer) {
            return;
        }
        if self.buffers[buffer].is_directory() {
            // An explorer's unapplied edits are a proposal about the
            // filesystem rather than text, so discarding them means restoring
            // the listing from disk, not blanking it.
            let _ = self.reload_directory_buffer(buffer);
        } else {
            // Closing is itself the discard, so the guard in `close_buffer` —
            // which protects callers that have not asked — is answered.
            self.buffers[buffer].mark_saved();
        }
        self.close_buffer_returning_from_commit(buffer);
    }

    /// Retires a buffer and completes the commit-message cancellation detour.
    pub(super) fn close_buffer_returning_from_commit(&mut self, buffer: usize) {
        let commit_message = self.buffers[buffer].is_commit_message();
        self.close_buffer(buffer);
        if commit_message {
            self.return_from_commit();
            self.status("commit cancelled; nothing was committed and the index is unchanged");
        }
    }

    pub(super) fn close_buffer(&mut self, buffer: usize) {
        self.retire_buffer(buffer, true);
    }

    fn retire_buffer(&mut self, buffer: usize, announce: bool) {
        if self.closed_buffers.contains(&buffer) {
            return;
        }
        if self.buffers[buffer].dirty {
            self.buffer_action_menu = None;
            self.error("modified buffers must be saved or discarded before closing");
            return;
        }
        self.invalidate_partial_guards(buffer);

        let name = self.buffers[buffer].display_name();
        let explorer_directory = self.buffers[buffer]
            .is_directory()
            .then(|| self.buffers[buffer].path.clone())
            .flatten();
        let affected = self
            .panes
            .iter()
            .filter_map(|(pane_id, pane)| (pane.buffer == buffer).then_some(*pane_id))
            .collect::<Vec<_>>();
        let mut replacements = Vec::with_capacity(affected.len());
        let mut scratch = None;
        for pane_id in affected {
            let fallback = loop {
                let candidate = self
                    .panes
                    .get_mut(&pane_id)
                    .and_then(|pane| pane.buffer_history.pop());
                match candidate {
                    Some(candidate)
                        if candidate != buffer
                            && candidate < self.buffers.len()
                            && !self.closed_buffers.contains(&candidate) =>
                    {
                        break candidate;
                    }
                    Some(_) => continue,
                    None => {
                        let fallback = (1..self.buffers.len())
                            .map(|distance| (buffer + distance) % self.buffers.len())
                            .find(|candidate| {
                                *candidate != buffer && !self.closed_buffers.contains(candidate)
                            })
                            .unwrap_or_else(|| {
                                *scratch.get_or_insert_with(|| {
                                    self.buffers.push(Buffer::scratch());
                                    self.syntax.push(None);
                                    self.buffers.len() - 1
                                })
                            });
                        break fallback;
                    }
                }
            };
            replacements.push((pane_id, fallback));
        }
        // Retire while this buffer is still active in any pane, so transient
        // hover and signature UI cannot survive its document identity.
        self.retire_lsp_buffer(buffer);
        for (pane_id, pane) in &mut self.panes {
            let jump_fallback = replacements
                .iter()
                .find_map(|(affected, fallback)| (*affected == *pane_id).then_some(*fallback))
                .unwrap_or(pane.buffer);
            pane.jumps.retire_buffer(buffer, jump_fallback);
            pane.buffer_history.retain(|previous| *previous != buffer);
            let owned_explorer = pane.directory_buffer == Some(buffer);
            if owned_explorer {
                pane.directory_buffer = None;
            }
            if (pane.buffer == buffer || owned_explorer)
                && let Some(directory) = explorer_directory.as_ref()
            {
                pane.last_explorer_directory = Some(directory.clone());
            }
        }
        for (pane_id, fallback) in replacements {
            let selection = self
                .take_pending_launch_selection(fallback)
                .unwrap_or_else(|| Selection::point(0));
            let pane = self.panes.get_mut(&pane_id).expect("affected pane exists");
            pane.replace_closed_buffer(fallback);
            pane.replace_selection(selection);
            pane.scroll_row = 0;
            pane.scroll_wrap = 0;
            pane.scroll_col = 0;
            pane.preserve_scroll = false;
        }
        self.stale_syntax.remove(&buffer);
        self.syntax[buffer] = None;
        self.closed_buffers.insert(buffer);
        self.special_buffer_recency
            .retain(|recent| *recent != buffer);
        self.word_index_notify_remove(buffer);
        // Keep the arena slot as a stable asynchronous identity, but drop the
        // text, path, directory projection, and undo/redo allocations it held.
        self.buffers[buffer] = Buffer::scratch();
        if announce {
            self.status(format!("closed {name}"));
        }
        self.refresh_buffer_picker();
    }

    /// Retains the two most recently active clean special buffers and retires
    /// the least recent detached one when a third is opened.
    ///
    /// Dirty special buffers are deliberately retained and discoverable, and
    /// a visible buffer is never evicted out from under another pane. Empty
    /// scratch buffers keep their independent immediate-retirement policy.
    pub(super) fn retire_detached_ephemeral_buffers(&mut self) {
        for pane in self.panes.values_mut() {
            let Some(explorer) = pane
                .directory_buffer
                .filter(|explorer| pane.buffer != *explorer)
            else {
                continue;
            };
            if let Some(directory) = self.buffers[explorer].path.clone() {
                pane.last_explorer_directory = Some(directory);
            }
        }
        self.special_buffer_recency.retain(|index| {
            !self.closed_buffers.contains(index)
                && self.buffers.get(*index).is_some_and(Buffer::is_special)
        });
        // A special buffer can be created and activated by an asynchronous
        // result between lifecycle passes. Discover unknown identities before
        // touching the current one, so an immediate history jump records the
        // destination as newer than the asynchronous view it just left.
        for (index, buffer) in self.buffers.iter().enumerate() {
            if !self.closed_buffers.contains(&index)
                && buffer.is_special()
                && !self.special_buffer_recency.contains(&index)
            {
                self.special_buffer_recency.push(index);
            }
        }
        if self.active_terminal().is_none() {
            let active = self.active().buffer;
            if self.buffers[active].is_special() {
                self.special_buffer_recency
                    .retain(|recent| *recent != active);
                self.special_buffer_recency.push(active);
            }
        }

        let visible = self
            .panes
            .values()
            .filter(|pane| pane.terminal.is_none())
            .map(|pane| pane.buffer)
            .collect::<HashSet<_>>();
        loop {
            let clean_special_count = self
                .buffers
                .iter()
                .enumerate()
                .filter(|(index, buffer)| {
                    !self.closed_buffers.contains(index) && buffer.is_special() && !buffer.dirty
                })
                .count();
            if clean_special_count <= SPECIAL_BUFFER_RETENTION_LIMIT {
                break;
            }
            let candidate = self.special_buffer_recency.iter().copied().find(|index| {
                !visible.contains(index)
                    && !self.closed_buffers.contains(index)
                    && self.buffers[*index].is_special()
                    && !self.buffers[*index].dirty
            });
            let Some(candidate) = candidate else {
                break;
            };
            if self.buffers[candidate].is_commit_message() {
                self.commit_origin = None;
            }
            self.retire_buffer(candidate, false);
        }

        loop {
            let visible = self
                .panes
                .values()
                .filter(|pane| pane.terminal.is_none())
                .map(|pane| pane.buffer)
                .collect::<HashSet<_>>();
            let referenced = self
                .panes
                .values()
                .map(|pane| pane.buffer)
                .collect::<HashSet<_>>();
            let durable_fallback_exists = self.buffers.iter().enumerate().any(|(index, buffer)| {
                !self.closed_buffers.contains(&index)
                    && (buffer.dirty || !buffer.is_special() && !buffer.is_empty_clean_scratch())
            });
            let candidate = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
                if self.closed_buffers.contains(&index) || buffer.dirty {
                    return None;
                }
                (buffer.is_empty_clean_scratch()
                    && !visible.contains(&index)
                    && (!referenced.contains(&index) || durable_fallback_exists))
                    .then_some(index)
            });
            let Some(candidate) = candidate else {
                break;
            };
            if self.buffers[candidate].is_commit_message() {
                self.commit_origin = None;
            }
            self.retire_buffer(candidate, false);
        }
    }

    fn activate_list_selection(&mut self) -> Result<()> {
        let chosen = self.selected_list_action();
        if self.settings_view.is_some() && chosen.is_none() {
            self.error("no matching setting choice · clear the filter or press Esc");
            return Ok(());
        }
        if let Some(ListAction::SettingValue { setting, value }) = chosen {
            self.persist_selected_setting(setting, value);
            return Ok(());
        }
        #[cfg(unix)]
        if matches!(chosen, Some(ListAction::Workspace(_))) && !self.persistent_session {
            self.error("attaching sessions needs workspace.mode: persistent");
            return Ok(());
        }
        self.list = None;
        self.buffer_action_menu = None;
        match chosen {
            Some(ListAction::Jump(location)) => self.jump_to(&location)?,
            Some(ListAction::CodeAction(index)) => self.run_code_action(index),
            Some(ListAction::Buffer(buffer)) => self.switch_buffer(buffer),
            Some(ListAction::SyntaxOutline { buffer, target }) => {
                self.jump_to_syntax_outline(buffer, target)
            }
            Some(ListAction::Macro(register)) => self.replay_macro(register, 1)?,
            Some(ListAction::GitCommit(oid)) => self.open_git_commit_oid(oid),
            Some(ListAction::Terminal(id)) => self.show_terminal(id),
            #[cfg(unix)]
            Some(ListAction::Workspace(row)) => {
                if let Some(path) = self
                    .workspace_rows
                    .get(row)
                    .map(|workspace| workspace.project_root.clone())
                    && self.request_workspace_switch(path)
                {
                    self.should_quit = true;
                }
            }
            Some(ListAction::SettingValue { .. }) => {
                unreachable!("settings actions return before closing the shared picker")
            }
            None => {}
        }
        Ok(())
    }

    // -- Presentation-neutral diagnostic queries, for rendering -------------

    /// The sign a row's gutter should carry.
    pub fn row_severity(&self, buffer_id: usize, row: usize) -> Option<crate::lsp::Severity> {
        let path = self.buffers.get(buffer_id)?.path.as_deref()?;
        self.diagnostics.severity_for_row(path, row)
    }

    /// Diagnostic spans on one row, as character offsets into the buffer.
    ///
    /// Converted here rather than when the diagnostic arrived because
    /// publishing is asynchronous: a diagnostic can outlive the text it
    /// described, and converting late clamps it into the current document
    /// instead of pointing past the end of it.
    pub fn diagnostic_spans(
        &self,
        buffer_id: usize,
        row: usize,
    ) -> Vec<(Offset, Offset, crate::lsp::Severity)> {
        let Some(buffer) = self.buffers.get(buffer_id) else {
            return Vec::new();
        };
        let Some(path) = buffer.path.as_deref() else {
            return Vec::new();
        };
        let encoding = self
            .language_of(buffer_id)
            .map_or(Encoding::default(), |language| self.encoding_for(&language));
        let text = buffer.text();
        self.diagnostics
            .for_row(path, row)
            .into_iter()
            .map(|diagnostic| {
                let (from, to) = from_lsp_range(text, diagnostic.range, encoding);
                // A zero-width diagnostic still has to be visible.
                (from, to.max(from + 1), diagnostic.severity)
            })
            .collect()
    }

    /// The message shown at the end of a row, when the caret is on it.
    pub fn inline_diagnostic(
        &self,
        buffer_id: usize,
        row: usize,
    ) -> Option<(String, crate::lsp::Severity)> {
        let path = self.buffers.get(buffer_id)?.path.as_deref()?;
        let diagnostic = self.diagnostics.for_row(path, row).into_iter().next()?;
        Some((diagnostic.label(), diagnostic.severity))
    }

    /// A short language-server summary for the status line, or `None` when no
    /// server is attached to the active buffer.
    pub fn lsp_summary(&self) -> Option<String> {
        let language = self.language_of(self.active().buffer)?;
        let server = self.lsp_servers.get(&language)?;
        let (errors, warnings) = self.diagnostics.counts();
        Some(match (errors, warnings) {
            (0, 0) => server.name.clone(),
            _ => format!("{} {errors}E {warnings}W", server.name),
        })
    }

    /// Drops every transient popup. Called whenever the caret moves or the
    /// mode changes, because a popup anchored to a position the caret has left
    /// is worse than no popup.
    pub(super) fn dismiss_popups(&mut self) {
        self.completion = None;
        self.signature = None;
        self.hover = None;
        let buffer = self.active().buffer;
        let cancelled: Vec<u64> = self
            .lsp_requests
            .iter()
            .filter_map(|(token, request)| {
                (request.buffer == buffer && request.pending.transient_group().is_some())
                    .then_some(*token)
            })
            .collect();
        for token in cancelled {
            self.cancel_lsp_request(token);
        }
    }
}
