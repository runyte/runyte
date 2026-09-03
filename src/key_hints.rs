// SPDX-License-Identifier: MPL-2.0

use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use crate::{
    command::{CommandCapability, Mode},
    input::{KeyCode, KeyStroke, Modifiers},
    keymap::{
        Binding, BindingAvailability, BindingNamespace, BindingRole, BindingScope, BindingTarget,
        Key, KeySequence, Keymap, Lookup,
    },
    service_health::AppCapabilitySnapshot,
};

pub const DEFAULT_MESSAGE_TIMEOUT: Duration = Duration::from_millis(1_200);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyHintRow {
    pub sequence: KeySequence,
    /// A shorter or historical spelling of the same command, when the registry
    /// names one. Discovery shows it so reaching a command through its
    /// namespace still teaches the key someone will want next time.
    pub alias: Option<KeySequence>,
    /// Modes in which a cross-mode alias is active. `None` means the row's
    /// current mode, so ordinary same-mode aliases stay compact.
    pub alias_modes: Option<&'static [Mode]>,
    pub target: Option<BindingTarget>,
    pub description: &'static str,
    pub availability: BindingAvailability,
    pub capability: Option<CommandCapability>,
    pub unavailable_reason: Option<String>,
    pub role: BindingRole,
    pub exact: bool,
    pub namespace: bool,
}

impl KeyHintRow {
    fn from_binding(binding: &Binding, exact: bool) -> Self {
        Self {
            sequence: binding.sequence.clone(),
            alias: binding.alias.clone(),
            alias_modes: binding.alias_modes,
            target: Some(binding.target),
            description: binding.description,
            availability: binding.availability,
            capability: binding.target.id().capability(),
            unavailable_reason: None,
            role: binding.role,
            exact,
            namespace: false,
        }
    }

    fn from_namespace(namespace: &BindingNamespace) -> Self {
        Self {
            sequence: namespace.sequence.clone(),
            alias: None,
            alias_modes: None,
            target: None,
            description: namespace.description,
            availability: BindingAvailability::Implemented,
            capability: namespace.capability,
            unavailable_reason: None,
            role: BindingRole::Primary,
            exact: false,
            namespace: true,
        }
    }

    /// Applies one active-buffer capability snapshot to this presentation row.
    /// Static planned/unsupported metadata remains authoritative when present.
    pub fn apply_capabilities(&mut self, capabilities: &AppCapabilitySnapshot) {
        self.unavailable_reason = if self.availability.is_implemented() {
            self.capability.and_then(|capability| {
                capabilities
                    .capability_availability(capability)
                    .reason()
                    .map(str::to_owned)
            })
        } else {
            None
        };
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HintEventResult {
    Forward,
    Consumed,
}

/// Presentation-neutral state for modal key discovery.
///
/// Discovery is an exploration tool, not a report of what just ran: the popup
/// is open only while a prefix is pending and more keys are expected. A
/// binding that resolves on its first key executes without ever opening it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyHintState {
    pending: KeySequence,
    message: Option<String>,
    expires_at: Option<Instant>,
    message_timeout: Duration,
    scroll_offset: usize,
    scroll_limit: Cell<Option<usize>>,
    counting: bool,
}

impl Default for KeyHintState {
    fn default() -> Self {
        Self::with_timeout(DEFAULT_MESSAGE_TIMEOUT)
    }
}

impl KeyHintState {
    pub fn with_timeout(message_timeout: Duration) -> Self {
        Self {
            pending: KeySequence::default(),
            message: None,
            expires_at: None,
            message_timeout,
            scroll_offset: 0,
            scroll_limit: Cell::new(None),
            counting: false,
        }
    }

    pub fn pending(&self) -> &KeySequence {
        &self.pending
    }

    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn is_visible(&self) -> bool {
        self.is_pending() || self.message.is_some()
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_limit
            .get()
            .map_or(self.scroll_offset, |limit| self.scroll_offset.min(limit))
    }

    /// Records the maximum offset for the geometry used by the latest frame.
    ///
    /// Geometry remains a renderer concern; retaining only its resolved bound
    /// lets repeated input saturate without teaching this state about rows,
    /// columns, or terminal cells.
    pub fn note_scroll_limit(&self, maximum_offset: usize) {
        self.scroll_limit.set(Some(maximum_offset));
    }

    /// Whether a plain vertical arrow is free to scroll the open hint popup.
    ///
    /// Registered continuations win so bindings such as `z Down` still reach
    /// the grammar. `Alt-j`/`Alt-k` remain the fallback when an arrow is
    /// claimed by the pending sequence.
    pub fn scrolls_with_arrow_in(
        &self,
        code: KeyCode,
        mode: Mode,
        scope: BindingScope,
        keymap: &Keymap,
    ) -> bool {
        if !self.is_pending()
            || !matches!(mode, Mode::Normal | Mode::Select)
            || !matches!(code, KeyCode::Up | KeyCode::Down)
        {
            return false;
        }

        let mut candidate = self.pending.clone();
        candidate.push(KeyStroke::plain(code));
        matches!(keymap.lookup_in(mode, scope, &candidate), Lookup::NoMatch)
    }

    /// Low-level mutation retained for callers that reconstruct state.
    pub fn push(&mut self, key: Key) {
        self.dismiss_transient();
        self.pending.push(key);
        self.reset_scroll();
        self.counting = false;
    }

    pub fn backspace(&mut self) -> Option<Key> {
        self.reset_scroll();
        self.pending.pop()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
        self.message = None;
        self.expires_at = None;
        self.reset_scroll();
        self.counting = false;
    }

    pub fn display_pending(&self) -> String {
        self.pending.to_string()
    }

    pub fn observe(
        &mut self,
        key: impl Into<KeyStroke>,
        mode: Mode,
        keymap: &Keymap,
    ) -> HintEventResult {
        self.observe_in(key, mode, BindingScope::Global, keymap)
    }

    pub fn observe_in(
        &mut self,
        key: impl Into<KeyStroke>,
        mode: Mode,
        scope: BindingScope,
        keymap: &Keymap,
    ) -> HintEventResult {
        self.observe_at_in(key, mode, scope, keymap, Instant::now())
    }

    pub fn observe_at(
        &mut self,
        key: impl Into<KeyStroke>,
        mode: Mode,
        keymap: &Keymap,
        now: Instant,
    ) -> HintEventResult {
        self.observe_at_in(key, mode, BindingScope::Global, keymap, now)
    }

    pub fn observe_at_in(
        &mut self,
        key: impl Into<KeyStroke>,
        mode: Mode,
        scope: BindingScope,
        keymap: &Keymap,
        now: Instant,
    ) -> HintEventResult {
        let key = key.into();
        self.expire_at(now);
        if matches!(mode, Mode::Insert | Mode::Replace)
            && !self.is_pending()
            && key != KeyStroke::ctrl('w')
        {
            self.clear();
            return HintEventResult::Forward;
        }
        if !matches!(
            mode,
            Mode::Normal | Mode::Select | Mode::Insert | Mode::Replace
        ) {
            self.clear();
            return HintEventResult::Forward;
        }

        let scroll_down = if self.is_pending()
            && key.modifiers == Modifiers::CONTROL
            && matches!(key.code, KeyCode::Char('n') | KeyCode::Char('p'))
        {
            Some(key.code == KeyCode::Char('n'))
        } else if self.is_pending()
            && key.modifiers.contains(Modifiers::ALT)
            && matches!(key.code, KeyCode::Char('j') | KeyCode::Char('k'))
        {
            Some(key.code == KeyCode::Char('j'))
        } else if key.modifiers.is_empty()
            && self.scrolls_with_arrow_in(key.code, mode, scope, keymap)
        {
            Some(key.code == KeyCode::Down)
        } else {
            None
        };
        if let Some(down) = scroll_down {
            self.scroll(down);
            return HintEventResult::Consumed;
        }

        if key.code == KeyCode::Escape {
            self.clear();
            return HintEventResult::Forward;
        }

        if self.pending.is_empty()
            && key.modifiers.is_empty()
            && let KeyCode::Char(digit @ '0'..='9') = key.code
            && (digit != '0' || self.counting)
        {
            self.dismiss_transient();
            self.counting = true;
            return HintEventResult::Forward;
        }
        self.counting = false;

        self.dismiss_transient();
        if key.code == KeyCode::Backspace && self.pending.len() > 1 {
            self.pending.pop();
            self.reset_scroll();
            return HintEventResult::Forward;
        }

        let inside_prefix = !self.pending.is_empty();
        let mut candidate = self.pending.clone();
        candidate.push(key);
        match keymap.lookup_in(mode, scope, &candidate) {
            Lookup::NoMatch => {
                self.pending.clear();
                self.reset_scroll();
                // Only report a dead end that was reached from an open menu; a
                // stray unbound key outside one is not worth a popup.
                if inside_prefix {
                    let candidate = candidate.to_string();
                    self.message = Some(format!("No binding: {candidate}"));
                    self.expires_at = Some(now + self.message_timeout);
                }
            }
            Lookup::Exact(_) => {
                self.pending.clear();
                self.expires_at = None;
                self.reset_scroll();
            }
            Lookup::Prefix(_) | Lookup::ExactAndPrefix { .. } => {
                self.pending = candidate;
                self.expires_at = None;
                self.reset_scroll();
            }
        }
        HintEventResult::Forward
    }

    pub fn rows(&self, keymap: &Keymap, mode: Mode) -> Vec<KeyHintRow> {
        self.rows_in(keymap, mode, BindingScope::Global)
    }

    pub fn rows_in(&self, keymap: &Keymap, mode: Mode, scope: BindingScope) -> Vec<KeyHintRow> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let namespaces = keymap
            .namespaces_for_scope(mode, scope)
            .filter(|namespace| {
                namespace.sequence.len() == self.pending.len() + 1
                    && namespace.sequence.starts_with(&self.pending)
            })
            .collect::<Vec<_>>();
        let mut rows = match keymap.lookup_in(mode, scope, &self.pending) {
            Lookup::NoMatch => Vec::new(),
            Lookup::Exact(binding) => vec![KeyHintRow::from_binding(binding, true)],
            Lookup::Prefix(bindings) => bindings
                .into_iter()
                .filter(|binding| {
                    binding.sequence.len() == self.pending.len() + 1
                        || !namespaces
                            .iter()
                            .any(|namespace| binding.sequence.starts_with(&namespace.sequence))
                })
                .map(|binding| KeyHintRow::from_binding(binding, false))
                .collect(),
            Lookup::ExactAndPrefix {
                exact,
                continuations,
            } => std::iter::once(KeyHintRow::from_binding(exact, true))
                .chain(
                    continuations
                        .into_iter()
                        .filter(|binding| {
                            binding.sequence.len() == self.pending.len() + 1
                                || !namespaces.iter().any(|namespace| {
                                    binding.sequence.starts_with(&namespace.sequence)
                                })
                        })
                        .map(|binding| KeyHintRow::from_binding(binding, false)),
                )
                .collect(),
        };
        rows.extend(namespaces.into_iter().map(KeyHintRow::from_namespace));
        rows.sort_by(|left, right| {
            left.role
                .cmp(&right.role)
                .then_with(|| left.sequence.to_string().cmp(&right.sequence.to_string()))
        });
        rows
    }

    pub fn time_until_expiry(&self, now: Instant) -> Option<Duration> {
        self.expires_at
            .map(|deadline| deadline.saturating_duration_since(now))
    }

    pub fn expire_at(&mut self, now: Instant) -> bool {
        if self.expires_at.is_some_and(|deadline| deadline <= now) {
            self.message = None;
            self.expires_at = None;
            self.reset_scroll();
            true
        } else {
            false
        }
    }

    fn dismiss_transient(&mut self) {
        self.message = None;
        self.expires_at = None;
    }

    fn reset_scroll(&mut self) {
        self.scroll_offset = 0;
        self.scroll_limit.set(None);
    }

    fn scroll(&mut self, down: bool) {
        let limit = self.scroll_limit.get().unwrap_or(usize::MAX);
        self.scroll_offset = self.scroll_offset.min(limit);
        if down {
            self.scroll_offset = self.scroll_offset.saturating_add(1).min(limit);
        } else {
            self.scroll_offset = self.scroll_offset.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::{
        command::{ColonCommand, EditorCommand, Mode},
        input::{KeyCode, KeyStroke, Modifiers},
        keymap::{
            Binding, BindingAvailability, BindingRole, BindingScope, BindingTarget, Key,
            KeySequence, Keymap, default_keymap,
        },
        service_health::{AppCapabilitySnapshot, CommandAvailability},
    };

    use super::KeyHintRow;

    use super::{HintEventResult, KeyHintState};

    fn event(character: char) -> KeyStroke {
        KeyStroke::char(character)
    }

    #[test]
    fn rows_are_derived_from_the_registry_prefix() {
        let mut hints = KeyHintState::default();
        hints.observe(event(' '), Mode::Normal, default_keymap());

        let rows = hints.rows(default_keymap(), Mode::Normal);
        assert!(
            rows.iter()
                .any(|row| row.target == Some(BindingTarget::Editor(EditorCommand::OpenExplorer)))
        );
        assert!(rows.iter().any(|row| {
            row.namespace
                && row.sequence == KeySequence::from([Key::char(' '), Key::char('/')])
                && row.description == "Look past this buffer"
        }));
    }

    #[test]
    fn reload_hint_reads_its_description_from_the_registry() {
        let mut hints = KeyHintState::default();
        hints.observe(event(' '), Mode::Normal, default_keymap());

        let reload = hints
            .rows(default_keymap(), Mode::Normal)
            .into_iter()
            .find(|row| row.target == Some(BindingTarget::Colon(ColonCommand::Reload)))
            .expect("the Space namespace lists reload");
        assert_eq!(reload.description, "Reload the active view");
    }

    #[test]
    fn nested_space_namespaces_collapse_then_reveal_registry_leaves() {
        let mut hints = KeyHintState::default();
        hints.observe(event(' '), Mode::Normal, default_keymap());
        let root = hints.rows(default_keymap(), Mode::Normal);
        for (suffix, description) in [
            ('c', "Clipboard"),
            ('l', "Language (LSP)"),
            ('w', "Windows"),
            ('x', "Syntax (Tree-sitter)"),
        ] {
            assert!(root.iter().any(|row| {
                row.namespace
                    && row.target.is_none()
                    && row.sequence == KeySequence::from([Key::char(' '), Key::char(suffix)])
                    && row.description == description
            }));
        }
        assert!(!root.iter().any(|row| {
            row.target == Some(BindingTarget::Editor(EditorCommand::ExpandSyntaxSelection))
        }));

        hints.observe(event('l'), Mode::Normal, default_keymap());
        let language = hints.rows(default_keymap(), Mode::Normal);
        assert!(language.iter().any(|row| {
            row.namespace
                && row.sequence
                    == KeySequence::from([Key::char(' '), Key::char('l'), Key::char('g')])
        }));
        assert!(language.iter().any(|row| {
            row.target == Some(BindingTarget::Editor(EditorCommand::ShowDocumentation))
        }));
        let completion = language
            .iter()
            .find(|row| row.target == Some(BindingTarget::Editor(EditorCommand::TriggerCompletion)))
            .expect("the language namespace lists completion");
        assert_eq!(completion.alias, Some(KeySequence::from(Key::ctrl('x'))));
        assert_eq!(
            completion.alias_modes,
            Some(&[Mode::Insert, Mode::Replace][..])
        );

        hints.observe(event('g'), Mode::Normal, default_keymap());
        let navigation = hints.rows(default_keymap(), Mode::Normal);
        assert_eq!(navigation.len(), 5);
        assert!(
            navigation
                .iter()
                .all(|row| !row.namespace && row.target.is_some())
        );
    }

    #[test]
    fn language_namespace_keeps_manager_recovery_available() {
        let capabilities = AppCapabilitySnapshot {
            syntax: CommandAvailability::Available,
            lsp_manager: CommandAvailability::Available,
            lsp_document: CommandAvailability::Unavailable(
                "the active file is not attached".to_owned(),
            ),
            git_project: CommandAvailability::Unavailable("not a Git repository".to_owned()),
            persistent_session: CommandAvailability::Available,
        };
        let mut hints = KeyHintState::default();
        hints.observe(event(' '), Mode::Normal, default_keymap());
        let mut root = hints.rows(default_keymap(), Mode::Normal);
        for row in &mut root {
            row.apply_capabilities(&capabilities);
        }
        let language = root
            .iter()
            .find(|row| row.description == "Language (LSP)")
            .expect("the Space menu lists Language (LSP)");
        assert_eq!(
            language.unavailable_reason.as_deref(),
            Some("the active file is not attached")
        );

        hints.observe(event('l'), Mode::Normal, default_keymap());
        let mut children = hints.rows(default_keymap(), Mode::Normal);
        for row in &mut children {
            row.apply_capabilities(&capabilities);
        }
        let status = children
            .iter()
            .find(|row| row.target == Some(BindingTarget::Colon(ColonCommand::LspStatus)))
            .expect("the Language namespace lists LSP status");
        assert_eq!(status.unavailable_reason, None);
        let completion = children
            .iter()
            .find(|row| row.target == Some(BindingTarget::Editor(EditorCommand::TriggerCompletion)))
            .expect("the Language namespace lists completion");
        assert_eq!(
            completion.unavailable_reason.as_deref(),
            Some("the active file is not attached")
        );
    }

    /// `Space Space` is an exact binding with no namespace of its own, so its
    /// availability has to come from the command it targets. Standalone mode
    /// must grey it out the way a missing language server greys `Space l`.
    #[test]
    fn the_session_manager_greys_out_in_standalone_mode() {
        let snapshot = |persistent_session| AppCapabilitySnapshot {
            syntax: CommandAvailability::Available,
            lsp_manager: CommandAvailability::Available,
            lsp_document: CommandAvailability::Available,
            git_project: CommandAvailability::Available,
            persistent_session,
        };
        let manager_row = |capabilities: &AppCapabilitySnapshot| {
            let mut hints = KeyHintState::default();
            hints.observe(event(' '), Mode::Normal, default_keymap());
            let mut rows = hints.rows(default_keymap(), Mode::Normal);
            for row in &mut rows {
                row.apply_capabilities(capabilities);
            }
            rows.into_iter()
                .find(|row| row.target == Some(BindingTarget::Colon(ColonCommand::SessionList)))
                .expect("the Space menu lists the session manager")
        };

        assert_eq!(
            manager_row(&snapshot(CommandAvailability::Unavailable(
                crate::service_health::PERSISTENT_SESSION_STANDALONE_REASON.to_owned(),
            )))
            .unavailable_reason
            .as_deref(),
            Some(crate::service_health::PERSISTENT_SESSION_STANDALONE_REASON)
        );
        assert_eq!(
            manager_row(&snapshot(CommandAvailability::Available)).unavailable_reason,
            None
        );
    }

    #[test]
    fn namespace_rows_carry_the_aliases_their_bindings_advertise() {
        let rows_under = |key: char| {
            let mut hints = KeyHintState::default();
            hints.observe(event(' '), Mode::Normal, default_keymap());
            hints.observe(event(key), Mode::Normal, default_keymap());
            hints.rows(default_keymap(), Mode::Normal)
        };
        let alias_of = |rows: &[KeyHintRow], command: EditorCommand| {
            rows.iter()
                .find(|row| row.target == Some(BindingTarget::Editor(command)))
                .unwrap_or_else(|| panic!("the namespace lists {command:?}"))
                .alias
                .clone()
        };

        let selections = rows_under('s');
        for (command, key) in [
            (EditorCommand::AlignSelections, '&'),
            (EditorCommand::KeepPrimarySelection, ','),
        ] {
            assert_eq!(
                alias_of(&selections, command),
                Some(KeySequence::from(Key::char(key)))
            );
        }
        // A command reachable only through the namespace has nothing to name,
        // and a namespace row is not a command at all.
        assert_eq!(
            alias_of(&selections, EditorCommand::KeepMatchingSelections),
            None
        );
        assert!(
            selections
                .iter()
                .all(|row| !row.namespace || row.alias.is_none())
        );

        // A two-key alias is advertised the same way a one-key alias is.
        let project = rows_under('/');
        assert_eq!(
            alias_of(&project, EditorCommand::OpenFilePicker),
            Some(KeySequence::from([Key::char(' '), Key::char('f')]))
        );
        assert_eq!(alias_of(&project, EditorCommand::GlobalSearchRegex), None);
        assert_eq!(
            alias_of(&rows_under('p'), EditorCommand::ToggleWhitespace),
            None
        );
    }

    #[test]
    fn resolved_bindings_never_open_the_popup() {
        let mut hints = KeyHintState::default();

        // A single-key binding executes with no post factum trace.
        hints.observe(event('k'), Mode::Normal, default_keymap());
        assert!(!hints.is_visible());
        assert!(hints.rows(default_keymap(), Mode::Normal).is_empty());

        // An unbound key outside a menu stays silent too.
        hints.observe(event('~'), Mode::Normal, default_keymap());
        assert!(!hints.is_visible());
        assert!(hints.message().is_none());
    }

    #[test]
    fn completing_a_prefix_sequence_closes_the_popup() {
        let mut hints = KeyHintState::default();
        hints.observe(event(' '), Mode::Normal, default_keymap());
        assert!(hints.is_visible());

        hints.observe(event('e'), Mode::Normal, default_keymap());
        assert!(!hints.is_visible());
        assert!(hints.time_until_expiry(Instant::now()).is_none());
    }

    #[test]
    fn exact_and_prefix_waits_while_showing_both_choices() {
        const NORMAL: &[Mode] = &[Mode::Normal];
        let keymap = Keymap::new(vec![
            Binding::implemented(NORMAL, Key::char('g'), EditorCommand::MoveFileStart),
            Binding::implemented(
                NORMAL,
                [Key::char('g'), Key::char('e')],
                EditorCommand::MoveFileEnd,
            ),
        ])
        .unwrap();
        let mut hints = KeyHintState::default();

        hints.observe(event('g'), Mode::Normal, &keymap);

        assert!(hints.is_pending());
        let rows = hints.rows(&keymap, Mode::Normal);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.exact));
        assert!(rows.iter().any(|row| !row.exact));
        assert!(hints.time_until_expiry(Instant::now()).is_none());
    }

    #[test]
    fn dead_ends_inside_a_menu_report_and_expire() {
        let start = Instant::now();
        let timeout = Duration::from_millis(500);
        let mut hints = KeyHintState::with_timeout(timeout);

        hints.observe_at(event('g'), Mode::Normal, default_keymap(), start);
        hints.observe_at(event('~'), Mode::Normal, default_keymap(), start);
        assert_eq!(hints.message(), Some("No binding: g ~"));
        assert_eq!(hints.time_until_expiry(start), Some(timeout));
        assert!(!hints.expire_at(start + timeout - Duration::from_millis(1)));
        assert!(hints.expire_at(start + timeout));
        assert!(!hints.is_visible());
    }

    #[test]
    fn escape_backspace_and_non_modal_modes_are_deterministic() {
        let mut hints = KeyHintState::default();
        hints.push(Key::char('g'));
        hints.push(Key::char('e'));
        assert_eq!(
            hints.observe(
                KeyStroke::plain(KeyCode::Backspace),
                Mode::Normal,
                default_keymap(),
            ),
            HintEventResult::Forward
        );
        assert_eq!(hints.pending().to_string(), "g");

        hints.observe(
            KeyStroke::plain(KeyCode::Escape),
            Mode::Normal,
            default_keymap(),
        );
        assert!(!hints.is_visible());

        hints.observe(event('g'), Mode::Insert, default_keymap());
        assert!(!hints.is_visible());
    }

    #[test]
    fn count_digits_forward_without_showing_an_invalid_binding() {
        let mut hints = KeyHintState::default();
        hints.observe(event('3'), Mode::Normal, default_keymap());
        hints.observe(event('0'), Mode::Normal, default_keymap());
        assert!(!hints.is_visible());

        hints.observe(event(' '), Mode::Normal, default_keymap());
        assert!(hints.is_pending());
        assert!(hints.message().is_none());
    }

    #[test]
    fn unavailable_rows_preserve_registry_status() {
        const NORMAL: &[Mode] = &[Mode::Normal];
        let unavailable = Binding {
            modes: NORMAL,
            scope: BindingScope::Global,
            sequence: [Key::char('x'), Key::char('y')].into(),
            target: BindingTarget::Editor(EditorCommand::SelectLine),
            description: "Unavailable action",
            availability: BindingAvailability::Planned("requires a parser"),
            role: BindingRole::Primary,
            alias: None,
            alias_modes: None,
        };
        let keymap = Keymap::new(vec![unavailable]).unwrap();
        let mut hints = KeyHintState::default();
        hints.observe(event('x'), Mode::Normal, &keymap);

        let rows = hints.rows(&keymap, Mode::Normal);
        assert!(matches!(
            rows[0].availability,
            BindingAvailability::Planned("requires a parser")
        ));
    }

    #[test]
    fn alt_j_and_k_scroll_without_forwarding_a_pending_sequence() {
        let mut hints = KeyHintState::default();
        hints.observe(event(' '), Mode::Normal, default_keymap());
        let down = KeyStroke::new(KeyCode::Char('j'), Modifiers::ALT);
        let up = KeyStroke::new(KeyCode::Char('k'), Modifiers::ALT);

        assert_eq!(
            hints.observe(down, Mode::Normal, default_keymap()),
            HintEventResult::Consumed
        );
        assert_eq!(hints.scroll_offset(), 1);
        assert_eq!(
            hints.observe(up, Mode::Normal, default_keymap()),
            HintEventResult::Consumed
        );
        assert_eq!(hints.scroll_offset(), 0);
        assert_eq!(hints.pending().to_string(), "Space");
    }

    #[test]
    fn control_n_and_p_scroll_a_terminal_insert_prefix() {
        let mut hints = KeyHintState::default();
        hints.observe_in(
            KeyStroke::ctrl('w'),
            Mode::Insert,
            BindingScope::Terminal,
            default_keymap(),
        );

        assert_eq!(
            hints.observe_in(
                KeyStroke::ctrl('n'),
                Mode::Insert,
                BindingScope::Terminal,
                default_keymap(),
            ),
            HintEventResult::Consumed
        );
        assert_eq!(hints.scroll_offset(), 1);
        assert_eq!(hints.pending().to_string(), "Ctrl-w");
        assert_eq!(
            hints.observe_in(
                KeyStroke::ctrl('p'),
                Mode::Insert,
                BindingScope::Terminal,
                default_keymap(),
            ),
            HintEventResult::Consumed
        );
        assert_eq!(hints.scroll_offset(), 0);
        assert_eq!(hints.pending().to_string(), "Ctrl-w");
    }

    #[test]
    fn arrows_scroll_only_when_the_pending_sequence_does_not_claim_them() {
        let mut hints = KeyHintState::default();
        hints.observe(event(' '), Mode::Normal, default_keymap());
        hints.observe(event('g'), Mode::Normal, default_keymap());

        assert_eq!(
            hints.observe(
                KeyStroke::plain(KeyCode::Down),
                Mode::Normal,
                default_keymap(),
            ),
            HintEventResult::Consumed
        );
        assert_eq!(hints.scroll_offset(), 1);
        assert_eq!(hints.pending().to_string(), "Space g");
        assert_eq!(
            hints.observe(
                KeyStroke::plain(KeyCode::Up),
                Mode::Normal,
                default_keymap(),
            ),
            HintEventResult::Consumed
        );
        assert_eq!(hints.scroll_offset(), 0);

        let mut bound = KeyHintState::default();
        bound.observe(event('z'), Mode::Normal, default_keymap());
        assert_eq!(
            bound.observe(
                KeyStroke::plain(KeyCode::Down),
                Mode::Normal,
                default_keymap(),
            ),
            HintEventResult::Forward
        );
        assert_eq!(bound.scroll_offset(), 0);
        assert!(!bound.is_pending());
    }

    #[test]
    fn directory_scope_reports_the_contextual_command() {
        const NORMAL: &[Mode] = &[Mode::Normal];
        let keymap = Keymap::new(vec![
            Binding::implemented(
                NORMAL,
                [Key::char(' '), Key::char('e')],
                EditorCommand::OpenExplorer,
            ),
            Binding::implemented_in(
                NORMAL,
                BindingScope::Directory,
                [Key::char(' '), Key::char('o')],
                EditorCommand::OpenDirectoryEntry,
            ),
        ])
        .unwrap();
        let mut hints = KeyHintState::default();
        hints.observe_in(event(' '), Mode::Normal, BindingScope::Directory, &keymap);

        let rows = hints.rows_in(&keymap, Mode::Normal, BindingScope::Directory);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(
            |row| row.target == Some(BindingTarget::Editor(EditorCommand::OpenDirectoryEntry))
        ));
        assert_eq!(
            hints
                .rows_in(&keymap, Mode::Normal, BindingScope::Global)
                .len(),
            1
        );
    }
}
