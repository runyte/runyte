// SPDX-License-Identifier: MPL-2.0
//! Arguments supplied by key programs, separate from palette spellings.
use super::{EditorCommand as C, descriptor::OperandArity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuppliedArgument {
    Literal,
    Character,
    CharacterPair,
    Pattern,
    Delimiter,
}

impl SuppliedArgument {
    pub const fn description(self) -> &'static str {
        match self {
            Self::Literal => "nonempty literal text (up to 4096 bytes); inserts at each caret",
            Self::Character => "one Unicode scalar; replaces the character operand",
            Self::CharacterPair => "two Unicode scalars (old, new); replaces both operands",
            Self::Pattern => "nonempty pattern; replaces the prompt",
            Self::Delimiter => "literal delimiter (may be empty); replaces the prompt",
        }
    }
    pub fn validate(self, text: &str) -> Result<(), &'static str> {
        if text.len() > 4096
            || text.chars().any(|c| {
                c.is_control() && !(self == Self::Literal && matches!(c, '\n' | '\r' | '\t'))
            })
        {
            return Err("argument exceeds 4096 bytes or contains control characters");
        }
        match self {
            Self::Character if text.chars().count() != 1 => {
                Err("argument requires exactly one Unicode scalar")
            }
            Self::CharacterPair if text.chars().count() != 2 => {
                Err("argument requires exactly two Unicode scalars (old, new)")
            }
            Self::Literal if text.is_empty() => Err("literal text must not be empty"),
            Self::Pattern if text.is_empty() => Err("pattern must not be empty"),
            _ => Ok(()),
        }
    }

    pub const fn replaces_prompt(self) -> bool {
        matches!(self, Self::Pattern | Self::Delimiter)
    }
}

impl C {
    pub fn supplied_argument(self) -> Option<SuppliedArgument> {
        match self.action_policy().operands {
            OperandArity::One => return Some(SuppliedArgument::Character),
            OperandArity::Two => return Some(SuppliedArgument::CharacterPair),
            OperandArity::Zero => {}
        }
        match self {
            C::Search
            | C::SearchRegex
            | C::TerminalSearch
            | C::TerminalSearchRegex
            | C::DirectoryTreeSearch
            | C::GlobalSearch
            | C::GlobalSearchRegex
            | C::KeepMatchingSelections
            | C::RemoveMatchingSelections => Some(SuppliedArgument::Pattern),
            C::InsertText => Some(SuppliedArgument::Literal),
            C::JoinSelections => Some(SuppliedArgument::Delimiter),
            _ => None,
        }
    }
}
