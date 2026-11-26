// SPDX-License-Identifier: MPL-2.0

//! Nonempty semantic programs, independent of their preset authoring format.

use super::{BindingTarget, actions::Action};

pub const MAX_STEPS: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
enum Steps {
    One(Action),
    Many(Box<[Action]>),
}

/// A bounded, nonempty list of semantic actions. Single-step defaults need no
/// allocation. Admission and continuation validation belong to the compiler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Program(Steps);

impl Program {
    pub fn single(target: BindingTarget) -> Self {
        Self(Steps::One(match target {
            BindingTarget::Editor(command) => Action::Editor(command),
            _ => Action::Target(target),
        }))
    }

    pub fn new(mut steps: Vec<Action>) -> Result<Self, &'static str> {
        match steps.len() {
            1 => Ok(Self(Steps::One(steps.pop().expect("one step")))),
            2..=MAX_STEPS => Ok(Self(Steps::Many(steps.into_boxed_slice()))),
            _ => Err("expected 1 through 16 program steps"),
        }
    }

    pub fn as_slice(&self) -> &[Action] {
        match &self.0 {
            Steps::One(step) => std::slice::from_ref(step),
            Steps::Many(steps) => steps,
        }
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Action> {
        self.as_slice().iter()
    }

    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn get(&self, index: usize) -> Option<&Action> {
        self.as_slice().get(index)
    }

    pub fn last(&self) -> &Action {
        self.as_slice().last().expect("nonempty program")
    }

    /// Resolve before executing any step, so unsupported counts cannot leave
    /// preceding selection, register, or text effects behind.
    pub fn count_step(&self) -> Option<usize> {
        self.iter().position(|step| {
            step.target()
                .id()
                .descriptor()
                .is_some_and(|descriptor| descriptor.policy.count.accepts())
        })
    }
}
