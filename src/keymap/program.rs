// SPDX-License-Identifier: MPL-2.0

//! Bounded semantic programs, independent of their preset authoring format.

use super::{BindingScope, BindingTarget, actions::Action};
use crate::command::Mode;

pub const MAX_STEPS: usize = 16;
pub const MAX_DEPTH: usize = 4;

/// Conditions shared by embedded and configured programs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Condition {
    Mode(Mode),
    NamedMode(String),
    BufferType(String),
    SelectionHasText,
    MultipleSelections,
    RegisterEmpty,
    ReadOnly,
    RecordingMacro,
    TreeEntryExpanded,
    TreeEntryIsDirectory,
    TreeEntryIsRoot,
}

impl Condition {
    pub fn matches_buffer_type(&self, scope: BindingScope) -> bool {
        matches!(self, Self::BufferType(name) if name == super::named::buffer_type(scope))
    }

    pub fn applicable_in(&self, scope: BindingScope) -> bool {
        match self {
            Self::Mode(_)
            | Self::NamedMode(_)
            | Self::BufferType(_)
            | Self::RegisterEmpty
            | Self::ReadOnly
            | Self::RecordingMacro => true,
            Self::SelectionHasText | Self::MultipleSelections => !matches!(
                scope,
                BindingScope::Media | BindingScope::DirectoryTree | BindingScope::SessionManager
            ),
            _ => scope == BindingScope::DirectoryTree,
        }
    }

    pub fn applicable_to(&self, scope: BindingScope, mode: Mode) -> bool {
        self.applicable_in(scope)
            && !(mode == Mode::List
                && matches!(self, Self::SelectionHasText | Self::MultipleSelections))
    }

    pub fn label(&self) -> String {
        match self {
            Self::Mode(mode) => format!("mode: {}", super::actions::mode_name(*mode)),
            Self::NamedMode(name) => format!("mode: {name}"),
            Self::BufferType(name) => format!("buffer-type: {name}"),
            Self::SelectionHasText => "selection-has-text".into(),
            Self::MultipleSelections => "multiple-selections".into(),
            Self::RegisterEmpty => "register-empty".into(),
            Self::ReadOnly => "read-only".into(),
            Self::RecordingMacro => "recording-macro".into(),
            Self::TreeEntryExpanded => "tree-entry-expanded".into(),
            Self::TreeEntryIsDirectory => "tree-entry-is-directory".into(),
            Self::TreeEntryIsRoot => "tree-entry-is-root".into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Step {
    Action(Action),
    ActionWithOptions {
        action: Action,
        options: StepOptions,
    },
    If {
        condition: Condition,
        then: Box<Program>,
        otherwise: Box<Program>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Steps {
    One(Step),
    Many(Box<[Step]>),
}

/// Empty branch bodies are permitted; a binding still requires a nonempty root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Program(Steps);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Prefix<T> {
    #[default]
    Consume,
    Inherit,
    None,
    Fixed(T),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StepOptions {
    pub count: Prefix<usize>,
    pub register: Prefix<char>,
    pub extend: Option<bool>,
}

impl StepOptions {
    pub fn validate(self, action: &Action) -> Result<(), &'static str> {
        if matches!(self.count, Prefix::Fixed(n) if !(1..=999_999).contains(&n)) {
            return Err("count must be between 1 and 999999");
        }
        if matches!(self.register, Prefix::Fixed(c) if c.is_control()) {
            return Err("invalid register name");
        }
        let descriptor = action.target().id().descriptor();
        if self.count != Prefix::Consume && !descriptor.is_some_and(|d| d.policy.count.accepts()) {
            return Err("count requires an action that accepts a count");
        }
        if self.register != Prefix::Consume && !descriptor.is_some_and(|d| d.uses_register) {
            return Err("register requires an action that uses a register");
        }
        if self.extend.is_some() && !descriptor.is_some_and(|d| d.supports_extend) {
            return Err("extend requires a supported motion");
        }
        Ok(())
    }
}

impl Program {
    pub fn conditional(condition: Condition, then: Self, otherwise: Self) -> Self {
        Self::from_steps(vec![Step::If {
            condition,
            then: Box::new(then),
            otherwise: Box::new(otherwise),
        }])
        .expect("bounded built-in conditional")
    }

    pub fn validate(&self, scope: BindingScope) -> Result<(), &'static str> {
        if self.is_empty() {
            return Err("binding program must not be empty");
        }
        self.validate_paths(Some(scope), false)
    }

    pub(super) fn validate_structure(&self) -> Result<(), &'static str> {
        if self.is_empty() {
            return Err("binding program must not be empty");
        }
        self.validate_paths(None, false)
    }

    fn validate_paths(
        &self,
        scope: Option<BindingScope>,
        outer_tail: bool,
    ) -> Result<(), &'static str> {
        for (index, step) in self.steps().iter().enumerate() {
            if let Step::ActionWithOptions { action, options } = step {
                options.validate(action)?;
                if options.extend.is_some()
                    && scope.is_some_and(|scope| {
                        !action
                            .target()
                            .id()
                            .descriptor()
                            .is_some_and(|d| d.supports_extend_in(scope))
                    })
                {
                    return Err("extend is unavailable for this buffer type");
                }
            }
            let tail = outer_tail || index + 1 < self.steps().len();
            match step {
                Step::Action(action) | Step::ActionWithOptions { action, .. }
                    if tail && !action.can_continue() =>
                {
                    return Err("prompt, background or plugin action must end every program path");
                }
                Step::Action(_) | Step::ActionWithOptions { .. } => {}
                Step::If {
                    condition,
                    then,
                    otherwise,
                } => {
                    if scope.is_some_and(|scope| !condition.applicable_in(scope)) {
                        return Err("condition is not available for this buffer type");
                    }
                    then.validate_paths(scope, tail)?;
                    otherwise.validate_paths(scope, tail)?;
                }
            }
        }
        Ok(())
    }

    pub fn single(target: BindingTarget) -> Self {
        Self(Steps::One(Step::Action(match target {
            BindingTarget::Editor(command) => Action::Editor(command),
            _ => Action::Target(target),
        })))
    }

    pub fn new(actions: Vec<Action>) -> Result<Self, &'static str> {
        if actions.is_empty() {
            return Err("expected 1 through 16 program steps");
        }
        Self::from_steps(actions.into_iter().map(Step::Action).collect())
    }

    pub fn from_steps(mut steps: Vec<Step>) -> Result<Self, &'static str> {
        let program = match steps.len() {
            1 => Self(Steps::One(steps.pop().unwrap())),
            _ => Self(Steps::Many(steps.into_boxed_slice())),
        };
        if program.len() > MAX_STEPS || program.depth() > MAX_DEPTH {
            return Err("program exceeds 16 total steps or 4 nesting levels");
        }
        Ok(program)
    }

    pub fn steps(&self) -> &[Step] {
        match &self.0 {
            Steps::One(step) => std::slice::from_ref(step),
            Steps::Many(steps) => steps,
        }
    }

    fn depth(&self) -> usize {
        self.steps()
            .iter()
            .map(|step| match step {
                Step::Action(_) | Step::ActionWithOptions { .. } => 0,
                Step::If {
                    then, otherwise, ..
                } => 1 + then.depth().max(otherwise.depth()),
            })
            .max()
            .unwrap_or(0)
    }

    pub fn inherits_count(&self) -> bool {
        self.steps().iter().any(|step| match step {
            Step::Action(_) => false,
            Step::ActionWithOptions { options, .. } => options.count == Prefix::Inherit,
            Step::If {
                then, otherwise, ..
            } => then.inherits_count() || otherwise.inherits_count(),
        })
    }

    pub fn has_options(&self) -> bool {
        self.steps().iter().any(|step| match step {
            Step::Action(_) => false,
            Step::ActionWithOptions { .. } => true,
            Step::If {
                then, otherwise, ..
            } => then.has_options() || otherwise.has_options(),
        })
    }

    pub fn has_conditions(&self) -> bool {
        self.depth() > 0
    }

    /// Visit every action in both branches for metadata, never for execution.
    pub fn iter(&self) -> impl Iterator<Item = &Action> {
        let mut current = self.steps().iter();
        let mut stack = Vec::new();
        std::iter::from_fn(move || {
            loop {
                match current.next() {
                    Some(Step::Action(action) | Step::ActionWithOptions { action, .. }) => {
                        return Some(action);
                    }
                    Some(Step::If {
                        then, otherwise, ..
                    }) => {
                        stack.push(current.clone());
                        stack.push(otherwise.steps().iter());
                        current = then.steps().iter();
                    }
                    None => current = stack.pop()?,
                }
            }
        })
    }

    pub fn len(&self) -> usize {
        self.steps()
            .iter()
            .map(|step| match step {
                Step::Action(_) | Step::ActionWithOptions { .. } => 1,
                Step::If {
                    then, otherwise, ..
                } => 1 + then.len() + otherwise.len(),
            })
            .sum()
    }
    pub fn is_empty(&self) -> bool {
        self.steps().is_empty()
    }
    pub fn get(&self, index: usize) -> Option<&Action> {
        self.iter().nth(index)
    }
    pub fn last(&self) -> &Action {
        self.iter().last().expect("binding has an action")
    }
    pub fn count_step(&self) -> Option<usize> {
        self.iter().position(|step| {
            step.target()
                .id()
                .descriptor()
                .is_some_and(|d| d.policy.count.accepts())
        })
    }
}

/// Public condition inventory used by the generated guide.
pub const CONDITION_DESCRIPTORS: &[(&str, &str, &str)] = &[
    (
        "selection-has-text",
        "At least one range has different anchor and head offsets; a caret is false",
        "Buffer types with ordinary text selections, including generated buffer types and terminal review; excludes media, directory-tree and UI owners",
    ),
    (
        "multiple-selections",
        "Two or more selection ranges",
        "Buffer types with ordinary text selections, including generated buffer types and terminal review; excludes media, directory-tree and UI owners",
    ),
    (
        "register-empty",
        "Pending selected register has no text or directory entries; after consumption, tests the unnamed register",
        "Everywhere programs execute; excludes UI owners",
    ),
    (
        "read-only",
        "The active buffer type's text cannot be edited, including terminal and media surfaces",
        "Everywhere programs execute; excludes UI owners",
    ),
    (
        "{mode: <name>}",
        "Role membership for normal/select/insert/replace; exact named mode otherwise",
        "Everywhere programs execute; excludes UI owners",
    ),
    (
        "{buffer-type: <type>}",
        "Exact buffer-types key; unknown names are rejected",
        "Everywhere programs execute; excludes UI owners",
    ),
    (
        "recording-macro",
        "A macro is currently being recorded",
        "Everywhere programs execute; excludes UI owners",
    ),
    (
        "tree-entry-expanded",
        "The selected tree entry is expanded",
        "directory-tree",
    ),
    (
        "tree-entry-is-directory",
        "The selected tree entry is a directory",
        "directory-tree",
    ),
    (
        "tree-entry-is-root",
        "The selected tree entry is the tree root",
        "directory-tree",
    ),
];
