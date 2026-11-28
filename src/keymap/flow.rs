// SPDX-License-Identifier: MPL-2.0
//! Abstract execution of programs after effective lookup has been constructed.
use super::{
    actions::Action,
    mode_graph::{EditingRole, ModeId},
    program::{Condition, Program, Step},
    *,
};
use crate::command::{
    ColonCommand as CC, CommandId, EditorCommand as C, InvocationParameters as P,
    descriptor::ActionEffect,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct State {
    kind: usize,
    mode: ModeId,
    temporary: Option<(Mode, ModeId)>,
    tree_return: Option<(usize, ModeId)>,
}

#[derive(Clone)]
pub(super) struct Failure {
    pub binding: usize,
    pub start: ModeId,
    pub kind: String,
    pub reason: String,
}
impl Failure {
    pub fn message(&self, map: &Keymap) -> String {
        format!(
            "buffer type {:?}, mode {:?}, key {:?}: {}",
            self.kind,
            map.mode_graph().unwrap().get(self.start).unwrap().name(),
            map.bindings[self.binding].sequence.to_string(),
            self.reason
        )
    }
    pub fn violation(&self, map: &Keymap) -> validate::Violation {
        validate::Violation {
            named_mode: Some(self.start),
            kind: validate::ViolationKind::InvalidProgram,
            mode: map
                .mode_graph()
                .unwrap()
                .editing_role(&self.kind, self.start)
                .unwrap()
                .editing_mode(),
            scope: map.bindings[self.binding].scope,
            sequences: vec![map.bindings[self.binding].sequence.clone()],
            message: self.message(map),
        }
    }
}
struct Analysis<'a> {
    map: &'a Keymap,
    admitted: std::sync::Arc<AdmissionInventory>,
    kinds: Vec<&'a str>,
    destinations: Vec<Vec<ModeId>>,
    work: usize,
    work_limit: usize,
    owner: BindingScope,
    configured: bool,
    history: HashSet<State>,
    changed_history: HashSet<usize>,
    observed: Vec<State>,
    destinations_changed: bool,
}
const MAX_WORK: usize = 2_000_000;

impl Analysis<'_> {
    fn graph(&self) -> &mode_graph::ModeGraph {
        self.map.mode_graph().unwrap()
    }
    fn role(&self, s: State) -> Mode {
        if matches!(
            self.graph().get(s.mode).unwrap().name(),
            "list" | "session-manager"
        ) {
            return Mode::List;
        }
        self.graph()
            .editing_role(self.kinds[s.kind], s.mode)
            .unwrap()
            .editing_mode()
    }
    fn scope(&self, s: State) -> BindingScope {
        if self.graph().get(s.mode).unwrap().name() == "session-manager" {
            return BindingScope::SessionManager;
        }
        BindingScope::ALL
            .iter()
            .copied()
            .find(|scope| named::buffer_type(*scope) == self.kinds[s.kind])
            .unwrap_or(BindingScope::Global)
    }
    fn enter(&self, mut s: State, name: &str, temporary: bool) -> Result<State, String> {
        if self.role(s) == Mode::List {
            return Err("UI list modes cannot enter editor modes".into());
        }
        let destination = self
            .graph()
            .entry_mode(self.kinds[s.kind], name)
            .ok_or_else(|| format!("mode-enter role {name:?} is unavailable"))?;
        s.temporary = if temporary {
            s.temporary.or(Some((self.role(s), s.mode)))
        } else {
            None
        };
        s.mode = destination;
        Ok(s)
    }
    fn restore_temporary(&self, mut state: State) -> State {
        let Some((editing, name)) = state.temporary.take() else {
            return state;
        };
        let role = EditingRole::ALL
            .into_iter()
            .find(|role| role.editing_mode() == editing)
            .unwrap_or(EditingRole::Normal);
        let kind = self.kinds[state.kind];
        let available = self.graph().role_mode(kind, role);
        let effective = if available.is_some() {
            role
        } else {
            EditingRole::Normal
        };
        state.mode =
            if available.is_some() && self.graph().editing_role(kind, name) == Some(effective) {
                name
            } else {
                self.graph().role_mode(kind, effective).unwrap()
            };
        state
    }
    fn destination(&self, source: State, kind: usize, mode: ModeId, may_reuse: bool) -> Vec<State> {
        let output = State {
            kind,
            mode,
            temporary: None,
            tree_return: None,
        };
        let mut states = vec![output];
        // Only an origin surviving dispatch can survive reopening the same
        // view. Creating a new destination always ends the gesture.
        if may_reuse && kind == source.kind && source.temporary.is_some() {
            states.push(State {
                temporary: source.temporary,
                ..output
            });
        }
        states
    }
    fn dynamic_destination(&self, state: State) -> Vec<State> {
        let mut outputs = self.dynamic(state, false);
        if state.temporary.is_some() {
            outputs.push(state);
        }
        outputs
    }
    fn dynamic(&self, s: State, same_kind: bool) -> Vec<State> {
        self.destinations
            .iter()
            .enumerate()
            .filter(|(kind, _)| !same_kind || *kind == s.kind)
            .flat_map(|(kind, modes)| {
                modes.iter().map(move |&mode| State {
                    kind,
                    mode,
                    temporary: None,
                    tree_return: None,
                })
            })
            .collect()
    }
    fn step(&self, action: &Action, state: State, follow: bool) -> Result<Vec<State>, String> {
        let mut outputs = self.step_path(action, state, follow, false)?;
        if state.temporary.is_some()
            && matches!(
                action.target().id(),
                CommandId::Editor(C::MoveFileStart | C::MoveFileEnd)
            )
        {
            outputs.extend(self.step_path(action, state, follow, true)?);
        }
        Ok(outputs)
    }
    fn step_path(
        &self,
        action: &Action,
        mut s: State,
        follow: bool,
        counted_boundary: bool,
    ) -> Result<Vec<State>, String> {
        let target = action.target();
        let id = target.id();
        if let Action::Invocation(invocation) = action
            && id == CommandId::Editor(C::ModeEnter)
        {
            return match invocation.parameters() {
                P::OptionalText(Some(name)) => Ok(vec![self.enter(s, name, false)?]),
                P::TemporaryMode(name) => Ok(vec![self.enter(s, name, true)?]),
                _ => Err("mode-enter requires an argument".into()),
            };
        }
        if let CommandId::Editor(command) = id {
            let early_tree = matches!(command, C::FocusDirectoryTree | C::ToggleDirectoryTree)
                || (self.scope(s) == BindingScope::DirectoryTree
                    && !admission::TREE_PASSES.contains(&command));
            let default_invocation = target.invocation().ok();
            let parameters = match action {
                Action::Invocation(invocation) => Some(invocation.parameters()),
                _ => default_invocation
                    .as_ref()
                    .map(|invocation| invocation.parameters()),
            };
            let direct_invocation = parameters.is_some_and(|parameters| match parameters {
                P::None => false,
                P::OptionalText(None) => command != C::OpenThemeSettings,
                P::OptionalPath(None) => !matches!(
                    command,
                    C::Save
                        | C::SplitVertical
                        | C::SplitHorizontal
                        | C::OpenWorkingDirectoryExplorer
                ),
                P::Help(crate::command::HelpInvocation::ActiveView) => false,
                _ => true,
            });
            let early_merge = matches!(
                command,
                C::MergeBranch
                    | C::OpenGitConflicts
                    | C::OpenGitConflict
                    | C::ContinueMerge
                    | C::AbortMerge
                    | C::NextConflict
                    | C::PreviousConflict
                    | C::KeepConflictCurrent
                    | C::TakeConflictOther
                    | C::InspectConflictSides
                    | C::ResolveConflictLiteralMarkers
                    | C::ResolveConflict
                    | C::KeepConflictFileCurrent
                    | C::TakeConflictFileOther
                    | C::ReturnToGitConflicts
                    | C::MergeReviewApprove
                    | C::MergeReviewCancel
                    | C::MergeReviewBack
                    | C::MergeReviewEnter
                    | C::MergeReviewNext
                    | C::MergeReviewPrevious
                    | C::MergeReviewLeft
                    | C::MergeReviewRight
            );
            if temporary::discards_temporary(command)
                || matches!(
                    command,
                    C::DeleteSelection
                        | C::DeleteSelectionForChange
                        | C::ReplaceWithRegister
                        | C::CutSelection
                        | C::CutSelectionForChange
                )
            {
                s.temporary = None;
            } else if !counted_boundary
                && !matches!(
                    command,
                    C::FindNextChar
                        | C::FindPreviousChar
                        | C::FindTillNextChar
                        | C::FindTillPreviousChar
                        | C::SelectRegister
                        | C::RecordMacro
                )
                && !early_tree
                && !early_merge
                && !direct_invocation
                && !temporary::continues_temporary(command)
            {
                s = self.restore_temporary(s);
            }
        }
        let mode = self.role(s);
        let scope = self.scope(s);
        // View capabilities are runtime refusals; a failed step has no tail.
        if !admission::admits(scope, target) {
            return if self.configured && scope == self.owner {
                Err(format!(
                    "{} is unavailable in {}",
                    target.name(),
                    scope.name()
                ))
            } else {
                Ok(Vec::new())
            };
        }
        if self.configured
            && self.owner == BindingScope::Terminal
            && scope == self.owner
            && mode == Mode::Insert
            && super::scopes::admit(scope, mode, target.name()).is_none()
        {
            return Err(format!(
                "{} is unavailable in terminal insert",
                target.name()
            ));
        }
        let policy = match id {
            CommandId::Editor(command) => Some(command.action_policy()),
            CommandId::Colon(command) => Some(command.action_policy()),
            CommandId::Plugin(_) => None,
        };
        if let Some(policy) = policy {
            let scoped = self
                .admitted
                .get(&(scope, mode))
                .is_some_and(|targets| targets.contains(&target));
            if !policy.global_binding_modes.contains(&mode) && !scoped {
                return Err(format!(
                    "{} is unavailable in {} {}",
                    target.name(),
                    scope.name(),
                    actions::mode_name(mode)
                ));
            }
        }
        let fixed = match id {
            CommandId::Editor(C::EnterNormalMode) => Some("normal"),
            CommandId::Editor(C::EnterSelectMode) => Some("select"),
            CommandId::Editor(C::EnterReplaceMode) => Some("replace"),
            CommandId::Editor(
                C::EnterInsertMode
                | C::AppendAfter
                | C::InsertLineStart
                | C::InsertLineEnd
                | C::OpenLineAbove
                | C::OpenLineBelow
                | C::TerminalEnterInput,
            ) => Some("insert"),
            _ => None,
        };
        if let Some(role) = fixed {
            return Ok(vec![self.enter(s, role, false)?]);
        }
        if matches!(id, CommandId::Editor(C::SelectionUndo | C::SelectionRedo)) {
            let mut states: Vec<_> = self
                .dynamic(s, true)
                .into_iter()
                .filter(|state| matches!(self.role(*state), Mode::Normal | Mode::Select))
                .collect();
            states.extend(
                self.history
                    .iter()
                    .copied()
                    .filter(|state| state.kind == s.kind),
            );
            if !follow {
                states.retain(|state| state.temporary.is_some());
            }
            return Ok(states);
        }
        if id == CommandId::Editor(C::ShrinkSyntaxSelection) {
            return Ok(if follow {
                self.dynamic(s, true)
            } else {
                Vec::new()
            });
        }
        if !follow && s.temporary.is_none() {
            return Ok(Vec::new());
        }
        if id == CommandId::Editor(C::FocusDirectoryTree) {
            if self.scope(s) == BindingScope::DirectoryTree {
                return Ok(vec![s]);
            }
            let kind = self
                .kinds
                .iter()
                .position(|name| *name == "directory-tree")
                .unwrap();
            let mode = self
                .graph()
                .role_mode("directory-tree", EditingRole::Normal)
                .unwrap();
            return Ok(vec![State {
                kind,
                mode,
                temporary: None,
                tree_return: Some((s.kind, s.mode)),
            }]);
        }
        if self.scope(s) == BindingScope::DirectoryTree
            && matches!(
                id,
                CommandId::Editor(C::DirectoryTreeClose | C::ToggleDirectoryTree | C::CloseWindow)
            )
        {
            if let Some((kind, mode)) = s.tree_return {
                return Ok(vec![State {
                    kind,
                    mode,
                    temporary: None,
                    tree_return: None,
                }]);
            }
            return Ok(self.dynamic_destination(s));
        }
        if matches!(
            id,
            CommandId::Editor(
                C::OpenTerminal
                    | C::ShowTerminal
                    | C::OpenTerminalFileDirectory
                    | C::OpenTerminalDirectoryRoot
                    | C::OpenTerminalSelectedDirectory
                    | C::OpenTerminalSessionDirectory
            )
        ) {
            let kind = self
                .kinds
                .iter()
                .position(|name| *name == "terminal")
                .unwrap();
            return Ok([EditingRole::Normal, EditingRole::Insert]
                .into_iter()
                .filter_map(|role| self.graph().role_mode("terminal", role))
                .flat_map(|mode| {
                    self.destination(s, kind, mode, id == CommandId::Editor(C::ShowTerminal))
                })
                .collect());
        }
        if id == CommandId::Editor(C::NewBuffer) {
            let role = EditingRole::ALL
                .into_iter()
                .find(|role| role.editing_mode() == self.role(s))
                .unwrap_or(EditingRole::Normal);
            return Ok(["text", "markdown"]
                .into_iter()
                .map(|name| {
                    let kind = self
                        .kinds
                        .iter()
                        .position(|candidate| *candidate == name)
                        .unwrap();
                    let mode = self
                        .graph()
                        .role_mode(name, role)
                        .or_else(|| self.graph().role_mode(name, EditingRole::Normal))
                        .unwrap();
                    State {
                        kind,
                        mode,
                        temporary: None,
                        tree_return: None,
                    }
                })
                .collect());
        }
        let destination = match id {
            CommandId::Editor(C::ShowHelp) => Some("help"),
            CommandId::Editor(C::OpenSettings) => Some("settings"),
            CommandId::Editor(C::CopyTerminalOutput | C::ShowAbout | C::ShowTutorial)
            | CommandId::Colon(CC::Notifications | CC::LogOpen) => Some("text"),
            _ => None,
        };
        if let Some(kind) = destination {
            let kind = self.kinds.iter().position(|name| *name == kind).unwrap();
            let mode = self
                .graph()
                .role_mode(self.kinds[kind], EditingRole::Normal)
                .ok_or_else(|| {
                    format!(
                        "destination buffer type {:?} has no normal role",
                        self.kinds[kind]
                    )
                })?;
            return Ok(self.destination(s, kind, mode, true));
        }
        // Closing a buffer or pane may reveal a retained destination even though
        // these legacy descriptors do not yet list Mode among their effects.
        if matches!(
            id,
            CommandId::Colon(
                CC::CloseBuffer
                    | CC::ForceCloseBuffer
                    | CC::Quit
                    | CC::ForceQuit
                    | CC::WriteQuit
                    | CC::WriteBufferClose
                    | CC::Open
                    | CC::DiffThis
                    | CC::DiffDisk
                    | CC::DiffRemote
                    | CC::DiffOff
                    | CC::GitDiffSideBySide
                    | CC::GitRefresh
            )
        ) {
            return Ok(self.dynamic_destination(s));
        }
        if matches!(
            id,
            CommandId::Editor(
                C::OpenWorkspaceSearchResult
                    | C::MergeReviewCancel
                    | C::MergeReviewBack
                    | C::MergeReviewEnter
                    | C::MergeReviewNext
                    | C::MergeReviewPrevious
                    | C::MergeReviewLeft
                    | C::MergeReviewRight
            )
        ) {
            return Ok(self.dynamic_destination(s));
        }
        if policy.is_some_and(|p| p.effects.contains(&ActionEffect::Mode)) {
            return Ok(self.dynamic_destination(s));
        }
        Ok(vec![s])
    }
    fn remember_history(&mut self, mut state: State) {
        if self.role(state) == Mode::List {
            return;
        }
        if !self.destinations[state.kind].contains(&state.mode) {
            self.destinations[state.kind].push(state.mode);
            self.destinations_changed = true;
            self.observed.push(state);
        }
        if state.temporary.is_none() {
            return;
        }
        if !matches!(self.role(state), Mode::Normal | Mode::Select) {
            let Some(mode) = self
                .graph()
                .role_mode(self.kinds[state.kind], EditingRole::Normal)
            else {
                return;
            };
            state.mode = mode;
        }
        if self.history.insert(state) {
            self.changed_history.insert(state.kind);
        }
    }
    fn charge(&mut self, amount: usize) -> Result<(), String> {
        self.work = self.work.saturating_add(amount);
        if self.work > self.work_limit {
            Err("mode-flow analysis exceeds its work bound".into())
        } else {
            Ok(())
        }
    }
    fn append_states(
        &mut self,
        next: &mut Vec<State>,
        seen: &mut HashSet<State>,
        outputs: Vec<State>,
    ) -> Result<(), String> {
        self.charge(outputs.len())?;
        next.extend(outputs.into_iter().filter(|state| seen.insert(*state)));
        Ok(())
    }
    fn walk(
        &mut self,
        program: &Program,
        mut states: Vec<State>,
        prefix: &str,
        first: bool,
        outer_tail: bool,
    ) -> Result<Vec<State>, String> {
        for (index, step) in program.steps().iter().enumerate() {
            self.charge(states.len())?;
            let path = format!("{prefix}{}", index + 1);
            let mut next = Vec::new();
            let mut seen = HashSet::new();
            for &state in &states {
                self.remember_history(state);
            }

            match step {
                Step::Action(action) => {
                    for state in states {
                        let outputs = self
                            .step(
                                action,
                                state,
                                outer_tail || index + 1 < program.steps().len(),
                            )
                            .map_err(|reason| format!("step {path}: {reason}"))?;
                        self.append_states(&mut next, &mut seen, outputs)?;
                    }
                }
                Step::If {
                    condition,
                    then,
                    otherwise,
                } => {
                    let mut yes = Vec::new();
                    let mut no = Vec::new();
                    for mut state in states {
                        if first && index == 0 {
                            state = self.restore_temporary(state);
                        }
                        if !condition.applicable_in(self.scope(state)) {
                            return Err(format!(
                                "step {path}: condition is unavailable in {}",
                                self.kinds[state.kind]
                            ));
                        }
                        let matches = match condition {
                            Condition::Mode(mode) => Some(self.role(state) == *mode),
                            Condition::NamedMode(name) => {
                                Some(self.graph().id(name) == Some(state.mode))
                            }
                            _ => None,
                        };
                        if matches != Some(false) {
                            yes.push(state);
                        }
                        if matches != Some(true) {
                            no.push(state);
                        }
                    }
                    let outputs = self.walk(
                        then,
                        yes,
                        &format!("{path}.then."),
                        false,
                        outer_tail || index + 1 < program.steps().len(),
                    )?;
                    self.append_states(&mut next, &mut seen, outputs)?;
                    let outputs = self.walk(
                        otherwise,
                        no,
                        &format!("{path}.else."),
                        false,
                        outer_tail || index + 1 < program.steps().len(),
                    )?;
                    self.append_states(&mut next, &mut seen, outputs)?;
                }
            }
            states = next;
        }
        Ok(states)
    }
}

type AdmissionInventory = HashMap<(BindingScope, Mode), HashSet<BindingTarget>>;

fn admission_inventory(admission: &Keymap) -> std::sync::Arc<AdmissionInventory> {
    static DEFAULT: std::sync::OnceLock<std::sync::Arc<AdmissionInventory>> =
        std::sync::OnceLock::new();
    static FAST: std::sync::OnceLock<std::sync::Arc<AdmissionInventory>> =
        std::sync::OnceLock::new();
    if std::ptr::eq(admission, super::default_keymap()) {
        return DEFAULT
            .get_or_init(|| std::sync::Arc::new(build_admission_inventory(admission)))
            .clone();
    }
    if std::ptr::eq(admission, super::keymap_for(true).as_ref()) {
        return FAST
            .get_or_init(|| std::sync::Arc::new(build_admission_inventory(admission)))
            .clone();
    }
    std::sync::Arc::new(build_admission_inventory(admission))
}
fn build_admission_inventory(admission: &Keymap) -> AdmissionInventory {
    let mut admitted = HashMap::new();
    for &scope in BindingScope::ALL {
        for mode in [
            Mode::Normal,
            Mode::Select,
            Mode::Insert,
            Mode::Replace,
            Mode::List,
        ] {
            let mut targets: HashSet<_> = admission
                .bindings_for_scope(mode, scope)
                .filter(|b| b.availability == BindingAvailability::Implemented)
                .flat_map(|b| std::iter::once(b.target).chain(b.program.iter().map(Action::target)))
                .collect();
            if matches!(mode, Mode::Normal | Mode::Select) {
                targets.extend(admission.context_actions(scope).map(|a| a.target));
            }
            if mode == Mode::List {
                targets.insert(C::ListClearFilter.into());
            }
            admitted.insert((scope, mode), targets);
        }
    }
    admitted
}

#[derive(Clone, Default)]
struct Reachability {
    states: HashSet<State>,
    history: HashSet<State>,
    destinations: Vec<Vec<ModeId>>,
}

// Bindings without mode-enter cannot create a new named/temporary mode origin.
// Reuse its fixed point only while every effective preset program remains
// available in each mode. Removed transitions require a fresh analysis: a
// superset of reachable states could incorrectly reject a now-valid program.
fn preset_reachability(map: &Keymap) -> Option<Reachability> {
    let preset = super::default_keymap();
    if !map.mode_graph()?.same_structure(preset.mode_graph()?)
        || map
            .bindings
            .iter()
            .filter(|b| b.configured_actions)
            .any(|b| {
                b.program
                    .iter()
                    .any(|a| a.target().id() == CommandId::Editor(C::ModeEnter))
            })
    {
        return None;
    }
    // Only known preset programs may be skipped. Index by semantic identity,
    // not key spelling: prefix remapping and fast-pane aliases change keys.
    type Identity = (BindingScope, Option<String>, BindingTarget);
    static PROGRAMS: std::sync::OnceLock<HashMap<Identity, Vec<usize>>> =
        std::sync::OnceLock::new();
    let programs = PROGRAMS.get_or_init(|| {
        let mut result: HashMap<Identity, Vec<usize>> = HashMap::new();
        for (index, binding) in preset.bindings.iter().enumerate() {
            result
                .entry((
                    binding.scope,
                    binding.binding_mode.as_deref().map(str::to_owned),
                    binding.target,
                ))
                .or_default()
                .push(index);
        }
        result
    });
    let identity = |b: &Binding| {
        programs
            .get(&(
                b.scope,
                b.binding_mode.as_deref().map(str::to_owned),
                b.target,
            ))
            .and_then(|indices| {
                indices.iter().copied().find(|&index| {
                    let p = &preset.bindings[index];
                    p.program == b.program && p.availability == b.availability
                })
            })
    };
    if map
        .bindings
        .iter()
        .filter(|b| !b.configured_actions)
        .any(|b| identity(b).is_none())
    {
        return None;
    }
    let effective = |keymap: &Keymap, mode| -> HashSet<usize> {
        keymap
            .lookup_index
            .get(&LookupContext::Named(mode))
            .and_then(|entries| entries.get(&KeySequence(Vec::new())))
            .into_iter()
            .flat_map(|entry| &entry.continuations)
            .filter_map(|&index| {
                let binding = &keymap.bindings[index];
                (!binding.configured_actions)
                    .then(|| identity(binding))
                    .flatten()
            })
            .collect()
    };
    for (mode, definition) in preset.mode_graph()?.modes() {
        let candidate = map.mode_graph()?.id(definition.name())?;
        if !effective(preset, mode).is_subset(&effective(map, candidate)) {
            return None;
        }
    }
    static PRESET: std::sync::OnceLock<Option<Reachability>> = std::sync::OnceLock::new();
    let seed = PRESET
        .get_or_init(|| {
            let (failures, reachability) = analyze(preset, preset, MAX_WORK, None);
            failures.is_empty().then_some(reachability)
        })
        .as_ref()?;
    let translate_mode = |id| {
        map.mode_graph()
            .unwrap()
            .id(preset.mode_graph().unwrap().get(id).unwrap().name())
            .unwrap()
    };
    let translate = |state: &State| State {
        mode: translate_mode(state.mode),
        temporary: state
            .temporary
            .map(|(role, mode)| (role, translate_mode(mode))),
        tree_return: state
            .tree_return
            .map(|(kind, mode)| (kind, translate_mode(mode))),
        ..*state
    };
    Some(Reachability {
        states: seed.states.iter().map(translate).collect(),
        history: seed.history.iter().map(translate).collect(),
        destinations: seed
            .destinations
            .iter()
            .map(|modes| modes.iter().copied().map(translate_mode).collect())
            .collect(),
    })
}

pub(super) fn failures(map: &Keymap, admission: &Keymap) -> Vec<Failure> {
    #[cfg(feature = "startup-timing")]
    let started = std::time::Instant::now();
    let seed = preset_reachability(map);
    let result = analyze(map, admission, MAX_WORK, seed.as_ref()).0;
    #[cfg(feature = "startup-timing")]
    super::configured::record_mode_flow_time_ns(started.elapsed().as_nanos());
    result
}

#[cfg(test)]
fn failures_with_limit(map: &Keymap, admission: &Keymap, work_limit: usize) -> Vec<Failure> {
    analyze(map, admission, work_limit, None).0
}

fn analyze(
    map: &Keymap,
    admission: &Keymap,
    work_limit: usize,
    seed: Option<&Reachability>,
) -> (Vec<Failure>, Reachability) {
    let Some(tables) = &map.overrides.1 else {
        return (Vec::new(), Reachability::default());
    };
    let kinds: Vec<_> = tables.buffer_types.keys().map(String::as_str).collect();
    // Every role is a possible initial state. Named entry expands reachability;
    // inherited bindings and tombstones are read through the completed index.
    let mut destinations = Vec::new();
    let empty = KeySequence(Vec::new());
    let mut indices = HashMap::new();
    for (mode, _) in tables.graph.modes() {
        indices.insert(
            mode,
            map.lookup_index
                .get(&LookupContext::Named(mode))
                .and_then(|entries| entries.get(&empty))
                .map(|entry| entry.continuations.clone())
                .unwrap_or_default(),
        );
    }
    for kind in &kinds {
        destinations.push(
            EditingRole::ALL
                .into_iter()
                .filter_map(|role| tables.graph.role_mode(kind, role))
                .collect(),
        );
    }
    let admitted = admission_inventory(admission);
    let mut analysis = Analysis {
        map,
        admitted,
        kinds,
        destinations,
        work: 0,
        work_limit,
        owner: BindingScope::Global,
        configured: false,
        history: HashSet::new(),
        changed_history: HashSet::new(),
        observed: Vec::new(),
        destinations_changed: false,
    };
    let mut pending: HashSet<_> = analysis
        .destinations
        .iter()
        .enumerate()
        .flat_map(|(kind, modes)| {
            modes.iter().map(move |&mode| State {
                kind,
                mode,
                temporary: None,
                tree_return: None,
            })
        })
        .collect();
    if let Some(kind) = analysis.kinds.iter().position(|name| *name == "text") {
        for name in ["list", "session-manager"] {
            if let Some(mode) = tables.graph.id(name) {
                pending.insert(State {
                    kind,
                    mode,
                    temporary: None,
                    tree_return: None,
                });
            }
        }
    }
    if let Some(seed) = seed {
        pending = seed.states.clone();
        analysis.history = seed.history.clone();
        analysis.destinations = seed.destinations.clone();
    }
    let mut seen = HashSet::new();
    // The same binding/state recurs when history grows. Programs without any
    // mode effect have no dependency on that growing destination/history set.
    let independent: Vec<_> = map
        .bindings
        .iter()
        .map(|b| {
            b.program.iter().all(|a| {
                a.target().id().descriptor().is_some_and(|d| {
                    !d.policy.effects.iter().any(|effect| {
                        matches!(
                            effect,
                            ActionEffect::Mode
                                | ActionEffect::View
                                | ActionEffect::Pane
                                | ActionEffect::Background
                                | ActionEffect::Prompt
                        )
                    })
                }) && !matches!(
                    a.target().id(),
                    CommandId::Editor(
                        C::SelectionUndo | C::SelectionRedo | C::ShrinkSyntaxSelection
                    )
                )
            })
        })
        .collect();
    let mut memo: HashMap<(State, usize), Result<Vec<State>, String>> = HashMap::new();
    let mut errors = Vec::new();
    let mut failed = HashSet::new();
    while let Some(state) = pending.iter().next().copied() {
        pending.remove(&state);
        if !seen.insert(state) {
            continue;
        }
        for &index in &indices[&state.mode] {
            let binding = &map.bindings[index];
            if binding.availability != BindingAvailability::Implemented
                || (seed.is_some() && !binding.configured_actions)
            {
                continue;
            }
            analysis.owner = binding.scope;
            analysis.configured = binding.configured_actions;
            let result = if let Some(cached) = memo.get(&(state, index)) {
                cached.clone()
            } else {
                let result = analysis.walk(&binding.program, vec![state], "", true, false);
                if independent[index] {
                    memo.insert((state, index), result.clone());
                }
                result
            };
            match result {
                Ok(outputs) => {
                    for &output in &outputs {
                        analysis.remember_history(output);
                    }
                    // All ordinary role states were seeded; newly observed named
                    // states are queued below. Only temporary gestures carry
                    // additional state from one key program into the next.
                    pending.extend(
                        outputs
                            .into_iter()
                            .filter(|state| state.temporary.is_some() && !seen.contains(state)),
                    )
                }
                Err(reason) if failed.insert((index, state.kind, state.mode)) => {
                    errors.push(Failure {
                        binding: index,
                        start: state.mode,
                        kind: analysis.kinds[state.kind].into(),
                        reason,
                    })
                }
                Err(_) => {}
            }
            if analysis.work > analysis.work_limit {
                return (errors, Reachability::default());
            }
        }
        pending.extend(analysis.observed.drain(..));
        if pending.is_empty()
            && (analysis.destinations_changed || !analysis.changed_history.is_empty())
        {
            // Only consumers in the same buffer type can restore these states.
            seen.retain(|state| {
                if analysis.destinations_changed || analysis.changed_history.contains(&state.kind) {
                    pending.insert(*state);
                    false
                } else {
                    true
                }
            });
            analysis.changed_history.clear();
            analysis.destinations_changed = false;
        }
    }
    (
        errors,
        Reachability {
            states: seen,
            history: analysis.history,
            destinations: analysis.destinations,
        },
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn exhausted_work_budget_rejects_instead_of_accepting_partial_analysis() {
        let map = super::super::default_keymap();
        let failures = super::failures_with_limit(map, map, 0);
        assert_eq!(failures.len(), 1);
        assert!(
            failures[0]
                .message(map)
                .contains("mode-flow analysis exceeds its work bound")
        );
        assert!(!failures[0].kind.is_empty());
    }

    #[test]
    fn dynamic_expansion_is_charged_before_accumulation() {
        use super::*;
        let map = super::super::default_keymap();
        let modes: Vec<_> = EditingRole::ALL
            .into_iter()
            .filter_map(|role| map.mode_graph().unwrap().role_mode("text", role))
            .collect();
        let initial = State {
            kind: 0,
            mode: modes[0],
            temporary: None,
            tree_return: None,
        };
        let mut analysis = Analysis {
            map,
            admitted: std::sync::Arc::new(HashMap::new()),
            kinds: vec!["text"],
            destinations: vec![modes],
            work: 0,
            work_limit: 3,
            owner: BindingScope::Global,
            configured: false,
            history: HashSet::new(),
            changed_history: HashSet::new(),
            observed: Vec::new(),
            destinations_changed: false,
        };
        let program = Program::new(vec![Action::Editor(C::NextWindow)]).unwrap();
        let error = analysis
            .walk(&program, vec![initial], "", true, true)
            .unwrap_err();
        assert!(error.contains("work bound"), "{error}");
        assert_eq!(
            analysis.work, 5,
            "one input plus four generated destinations"
        );
    }

    #[test]
    fn incremental_admission_matches_full_analysis_for_changed_programs() {
        use super::*;
        let preset = super::super::default_keymap();
        for source in [
            "[move-right, yank]",
            "[insert-newline]",
            "[focus-window-right, yank]",
            "[selection-undo, insert-newline]",
            "[{if: {mode: select}, then: [yank], else: [insert-newline]}]",
        ] {
            let definitions: Vec<super::super::step::StepDef> =
                serde_yaml::from_str(source).unwrap();
            let program = super::super::step::compile(&definitions, 0, false).unwrap();
            let mut map = preset.clone();
            let mut binding = map
                .bindings
                .iter()
                .find(|b| b.sequence.to_string() == "j" && b.scope == BindingScope::Global)
                .unwrap()
                .clone();
            binding.sequence = KeySequence::parse("F6").unwrap();
            binding.program = program;
            binding.configured_actions = true;
            map.bindings.push(binding);
            map.rebuild_lookup_index();
            let seed = preset_reachability(&map).expect("non-expanding override is incremental");
            let normalize = |failures: Vec<Failure>| {
                let mut messages: Vec<_> = failures
                    .into_iter()
                    .map(|failure| {
                        (
                            failure.binding,
                            map.mode_graph()
                                .unwrap()
                                .get(failure.start)
                                .unwrap()
                                .name()
                                .to_owned(),
                            failure.kind,
                        )
                    })
                    .collect();
                messages.sort();
                messages.dedup();
                messages
            };
            assert_eq!(
                normalize(analyze(&map, preset, MAX_WORK, Some(&seed)).0),
                normalize(analyze(&map, preset, MAX_WORK, None).0),
                "{source}"
            );
        }
    }

    #[test]
    fn removed_preset_transitions_require_full_analysis() {
        use super::*;
        let preset = super::super::default_keymap();
        for key in ["x", "X", "v"] {
            let mut map = preset.clone();
            let before = map.bindings.len();
            map.bindings
                .retain(|binding| binding.sequence.to_string() != key);
            assert!(map.bindings.len() < before);
            map.rebuild_lookup_index();
            assert!(preset_reachability(&map).is_none(), "removed {key}");
        }
    }

    #[test]
    fn preview_fixture_preserves_preset_reachability() {
        use super::*;
        let config = serde_yaml::from_str(
            "editor:\n  fast_pane_keys: true\nkeys:\n  bind:\n    normal:\n      Ctrl-Left: focus-window-left\n      Ctrl-Right: focus-window-right\n      Ctrl-Up: focus-window-up\n      Ctrl-Down: focus-window-down\n",
        ).unwrap();
        let result = configured::compile_configuration(&config, super::super::default_keymap());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(preset_reachability(&result.keymap).is_some());
    }

    #[test]
    fn shipped_preset_has_no_invalid_mode_paths() {
        let map = super::super::default_keymap();
        let failures = super::failures(map, map);
        let mut lines = failures
            .iter()
            .map(|f| {
                format!(
                    "{} {:?} {}: {}",
                    f.kind,
                    map.mode_graph().unwrap().get(f.start).unwrap().name(),
                    map.bindings[f.binding].sequence,
                    f.reason
                )
            })
            .collect::<Vec<_>>();
        lines.sort();
        lines.dedup();
        assert!(lines.is_empty(), "{}", lines.join("\n"));
    }
}
