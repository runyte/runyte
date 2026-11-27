// SPDX-License-Identifier: MPL-2.0

//! Bounded semantic programs, independent of their preset authoring format.

use super::{BindingScope, BindingTarget, actions::Action};
use crate::command::Mode;

pub const MAX_STEPS: usize = 16;
pub const MAX_DEPTH: usize = 4;

/// Internal conditions used by built-in programs. Public syntax lands in stage 5.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Condition {
    Mode(Mode),
    RecordingMacro,
    TreeEntryExpanded,
    TreeEntryIsDirectory,
    TreeEntryIsRoot,
}

impl Condition {
    pub fn applicable_in(self, scope: BindingScope) -> bool {
        match self {
            Self::Mode(_) | Self::RecordingMacro => true,
            _ => scope == BindingScope::DirectoryTree,
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Mode(mode) => format!("mode: {}", super::actions::mode_name(mode)),
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
        self.validate_paths(scope, false)
    }

    fn validate_paths(&self, scope: BindingScope, outer_tail: bool) -> Result<(), &'static str> {
        for (index, step) in self.steps().iter().enumerate() {
            let tail = outer_tail || index + 1 < self.steps().len();
            match step {
                Step::Action(action) if tail && !action.can_continue() => {
                    return Err("prompt, background or plugin action must end every program path");
                }
                Step::Action(_) => {}
                Step::If {
                    condition,
                    then,
                    otherwise,
                } => {
                    if !condition.applicable_in(scope) {
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
                Step::Action(_) => 0,
                Step::If {
                    then, otherwise, ..
                } => 1 + then.depth().max(otherwise.depth()),
            })
            .max()
            .unwrap_or(0)
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
                    Some(Step::Action(action)) => return Some(action),
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
                Step::Action(_) => 1,
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
