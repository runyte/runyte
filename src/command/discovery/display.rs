// SPDX-License-Identifier: MPL-2.0
//! Shared user-facing vocabulary for action contracts and availability.
use crate::command::{
    ArgumentKind, CommandArguments,
    descriptor::{ActionEffect, ArgumentDelivery, CountBehavior, OperandArity},
};
use crate::{keymap::BindingAvailability, service_health::CommandAvailability};

pub fn effects(values: &[ActionEffect]) -> String {
    values
        .iter()
        .map(|value| match value {
            ActionEffect::Selection => "selection",
            ActionEffect::Text => "text",
            ActionEffect::Mode => "mode",
            ActionEffect::View => "view",
            ActionEffect::Pane => "pane",
            ActionEffect::File => "file",
            ActionEffect::Operand => "character input",
            ActionEffect::Prompt => "prompt",
            ActionEffect::Background => "background work",
            ActionEffect::Register => "register",
            ActionEffect::SearchState => "search state",
            ActionEffect::Configuration => "configuration",
            ActionEffect::Macro => "macro",
            ActionEffect::History => "history",
            ActionEffect::TerminalInput => "terminal input",
            ActionEffect::Application => "application",
        })
        .collect::<Vec<_>>()
        .join(", ")
}
pub fn count(value: CountBehavior) -> &'static str {
    match value {
        CountBehavior::Reject => "not accepted",
        CountBehavior::Repeat => "repeats",
        CountBehavior::Interpret => "interpreted as an address or extent",
    }
}
pub fn operands(value: OperandArity) -> &'static str {
    match value {
        OperandArity::Zero => "none",
        OperandArity::One => "one character",
        OperandArity::Two => "two characters",
    }
}
pub fn continuation(descriptor: crate::command::descriptor::ActionDescriptor) -> &'static str {
    if descriptor.policy.effects.contains(&ActionEffect::Prompt)
        && !descriptor
            .policy
            .effects
            .contains(&ActionEffect::Background)
        && descriptor
            .argument()
            .is_some_and(|arg| arg.delivery == ArgumentDelivery::ReplacesPrompt)
    {
        "last unless an argument supplies the prompt input"
    } else {
        position(descriptor.can_continue(&crate::command::InvocationParameters::None))
    }
}
pub fn position(value: bool) -> &'static str {
    match value {
        true => "can be followed by another action",
        false => "must be the last action",
    }
}
pub fn arguments(value: CommandArguments) -> &'static str {
    match value {
        CommandArguments::None => "none",
        CommandArguments::Optional(ArgumentKind::Path) => "optional path",
        CommandArguments::Required(ArgumentKind::Path) => "required path",
        CommandArguments::Optional(ArgumentKind::FreeText) => "optional text",
        CommandArguments::Required(ArgumentKind::FreeText) => "required text",
    }
}
pub fn delivery(value: ArgumentDelivery) -> &'static str {
    match value {
        ArgumentDelivery::Invocation => "passed to the action",
        ArgumentDelivery::ReplacesPrompt => "replaces the prompt",
    }
}
pub fn binding_availability(value: BindingAvailability) -> String {
    match value {
        BindingAvailability::Implemented => "available".into(),
        BindingAvailability::Planned(reason) => format!("planned: {reason}"),
        BindingAvailability::Unsupported(reason) => format!("unsupported: {reason}"),
    }
}
pub fn command_availability(value: &CommandAvailability) -> String {
    match value {
        CommandAvailability::Available => "available".into(),
        CommandAvailability::Unavailable(reason) => format!("unavailable: {reason}"),
    }
}
