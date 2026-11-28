// SPDX-License-Identifier: MPL-2.0
//! Typed step syntax shared by embedded presets and configured bindings.
use super::{BindingTarget, actions, mode_graph, program};
use crate::command::{CommandInvocation, Mode};
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};
use std::fmt;

pub(super) const SYNTAX_HINT: &str = "use an action name or { action: <name>, arg: <argument> }; { command:, argument: } and { name: arg } are not supported";

pub(super) fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

fn strict_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    present_string(d).map(Option::unwrap)
}

pub(super) fn present_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    struct StrictString;
    impl<'de> Visitor<'de> for StrictString {
        type Value = String;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a string")
        }
        fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<String, E> {
            Ok(s.into())
        }
        fn visit_string<E: serde::de::Error>(self, s: String) -> Result<String, E> {
            Ok(s)
        }
    }
    d.deserialize_any(StrictString).map(Some)
}

pub(super) fn validate_action(name: &str, arg: Option<&str>) -> Result<(), &'static str> {
    if name.is_empty()
        || name.len() > 128
        || name.chars().any(char::is_control)
        || arg.is_some_and(|arg| arg.len() > 4096 || arg.chars().any(char::is_control))
    {
        return Err("action name or argument exceeds its bound or contains control characters");
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
pub(super) enum StepDef {
    Name(String),
    Action(ActionDef),
    Conditional(Conditional),
}
#[derive(Debug, PartialEq)]
pub(super) struct ActionDef {
    pub action: String,
    pub arg: Option<String>,
    pub temporary: Option<bool>,
}
#[derive(Debug, PartialEq)]
pub(super) struct Conditional {
    pub condition: ConditionDef,
    pub then: Vec<StepDef>,
    pub otherwise: Vec<StepDef>,
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(untagged)]
pub(super) enum ConditionDef {
    Mode(ModeCondition),
    Name(ConditionName),
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct ModeCondition {
    #[serde(deserialize_with = "strict_string")]
    pub mode: String,
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ConditionName {
    RecordingMacro,
    TreeEntryExpanded,
    TreeEntryIsDirectory,
    TreeEntryIsRoot,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct StepRecord {
    #[serde(default, deserialize_with = "present_string")]
    pub action: Option<String>,
    #[serde(default, deserialize_with = "present_string")]
    pub arg: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub temporary: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    pub command: Option<serde_yaml::Value>,
    #[serde(default, deserialize_with = "present")]
    pub argument: Option<serde_yaml::Value>,
    #[serde(default, deserialize_with = "present")]
    pub count: Option<serde_yaml::Value>,
    #[serde(default, deserialize_with = "present")]
    pub register: Option<serde_yaml::Value>,
    #[serde(default, deserialize_with = "present")]
    pub extend: Option<serde_yaml::Value>,
    #[serde(rename = "if", default, deserialize_with = "present")]
    pub condition: Option<ConditionDef>,
    #[serde(default, deserialize_with = "present")]
    pub then: Option<Vec<StepDef>>,
    #[serde(rename = "else", default, deserialize_with = "present")]
    pub otherwise: Option<Vec<StepDef>>,
}
impl StepRecord {
    pub(super) fn into_step<E: serde::de::Error>(self) -> Result<StepDef, E> {
        if self.command.is_some() || self.argument.is_some() {
            return Err(E::custom(SYNTAX_HINT));
        }
        if self.count.is_some() || self.register.is_some() || self.extend.is_some() {
            return Err(E::custom(
                "count, register and extend step fields are not available until stage 5d",
            ));
        }
        if self.temporary.is_some() && self.action.as_deref() != Some("mode-enter") {
            return Err(E::custom("temporary requires mode-enter"));
        }
        match (
            self.action,
            self.arg,
            self.condition,
            self.then,
            self.otherwise,
        ) {
            (Some(action), arg, None, None, None) => {
                validate_action(&action, arg.as_deref()).map_err(E::custom)?;
                Ok(StepDef::Action(ActionDef {
                    action,
                    arg,
                    temporary: self.temporary,
                }))
            }
            (None, None, Some(condition), Some(then), Some(otherwise)) => {
                Ok(StepDef::Conditional(Conditional {
                    condition,
                    then,
                    otherwise,
                }))
            }
            _ => Err(E::custom(SYNTAX_HINT)),
        }
    }
}
impl<'de> Deserialize<'de> for StepDef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct StepVisitor;
        impl<'de> Visitor<'de> for StepVisitor {
            type Value = StepDef;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(SYNTAX_HINT)
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<StepDef, E> {
                validate_action(s, None).map_err(E::custom)?;
                Ok(StepDef::Name(s.into()))
            }
            fn visit_string<E: serde::de::Error>(self, s: String) -> Result<StepDef, E> {
                validate_action(&s, None).map_err(E::custom)?;
                Ok(StepDef::Name(s))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<StepDef, A::Error> {
                StepRecord::deserialize(serde::de::value::MapAccessDeserializer::new(map))?
                    .into_step()
            }
        }
        d.deserialize_any(StepVisitor)
    }
}

pub(super) fn compile(
    steps: &[StepDef],
    depth: usize,
    embedded: bool,
) -> Result<program::Program, String> {
    use program::{Condition as C, Program, Step as S};
    if depth > program::MAX_DEPTH || steps.len() > program::MAX_STEPS {
        return Err("program exceeds bounds".into());
    }
    let steps = steps
        .iter()
        .map(|step| {
            Ok(match step {
                StepDef::Name(name) => {
                    if name == "mode-enter" {
                        return Err("mode-enter requires action/arg".into());
                    }
                    S::Action(if embedded {
                        match super::presets::target(name)? {
                            BindingTarget::Editor(c) => actions::Action::Editor(c),
                            t => actions::Action::Target(t),
                        }
                    } else {
                        actions::resolve_action(name, None, Mode::Normal, true)?.0
                    })
                }
                StepDef::Action(ActionDef {
                    action,
                    arg,
                    temporary,
                }) => {
                    if action == "mode-enter" {
                        let arg = arg.as_ref().ok_or("mode-enter requires action/arg")?;
                        mode_graph::valid_name(arg)?;
                        S::Action(actions::Action::Invocation(
                            CommandInvocation::from_parts(
                                crate::command::CommandId::Editor(
                                    crate::command::EditorCommand::ModeEnter,
                                ),
                                if temporary.unwrap_or(false) {
                                    crate::command::InvocationParameters::TemporaryMode(arg.clone())
                                } else {
                                    crate::command::InvocationParameters::OptionalText(Some(
                                        arg.clone(),
                                    ))
                                },
                                crate::command::CommandExecutionContext::default(),
                            )
                            .map_err(|error| error.to_string())?,
                        ))
                    } else {
                        S::Action(
                            actions::resolve_action(action, arg.as_deref(), Mode::Normal, true)?.0,
                        )
                    }
                }
                StepDef::Conditional(c) => S::If {
                    condition: match &c.condition {
                        ConditionDef::Mode(m) => {
                            mode_graph::valid_name(&m.mode)?;
                            match mode_graph::EditingRole::named(&m.mode) {
                                Some(role) => C::Mode(role.editing_mode()),
                                None => C::NamedMode(m.mode.clone()),
                            }
                        }
                        ConditionDef::Name(n) => match n {
                            ConditionName::RecordingMacro => C::RecordingMacro,
                            ConditionName::TreeEntryExpanded => C::TreeEntryExpanded,
                            ConditionName::TreeEntryIsDirectory => C::TreeEntryIsDirectory,
                            ConditionName::TreeEntryIsRoot => C::TreeEntryIsRoot,
                        },
                    },
                    then: Box::new(compile(&c.then, depth + 1, embedded)?),
                    otherwise: Box::new(compile(&c.otherwise, depth + 1, embedded)?),
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let program = Program::from_steps(steps).map_err(str::to_owned)?;
    if !embedded
        && program.iter().any(|action| {
            matches!(action.target(), BindingTarget::Editor(command)
        if crate::command::INTERNAL_EDITOR_COMMANDS.contains(&command))
        })
    {
        return Err("internal editor action is not publicly bindable".into());
    }
    Ok(program)
}

pub(super) fn yaml(program: &program::Program) -> Result<serde_yaml::Value, String> {
    use serde_yaml::{Mapping, Value};
    fn value(step: &program::Step) -> Result<Value, String> {
        Ok(match step {
            program::Step::Action(action) => {
                let name = action.target().name();
                match action {
                    actions::Action::Invocation(invocation) => {
                        use crate::command::InvocationParameters as P;
                        let mut map = Mapping::new();
                        map.insert("action".into(), name.into());
                        match invocation.parameters() {
                            P::None => return Ok(name.into()),
                            P::OptionalText(Some(arg)) => {
                                map.insert("arg".into(), arg.clone().into());
                            }
                            P::TemporaryMode(arg) => {
                                map.insert("arg".into(), arg.clone().into());
                                map.insert("temporary".into(), true.into());
                            }
                            _ => {
                                return Err(
                                    "binding arguments require their authored configuration source"
                                        .into(),
                                );
                            }
                        }
                        Value::Mapping(map)
                    }
                    _ => name.into(),
                }
            }
            program::Step::If {
                condition,
                then,
                otherwise,
            } => {
                let condition = match condition {
                    program::Condition::Mode(mode) => {
                        let mut map = Mapping::new();
                        map.insert("mode".into(), actions::mode_name(*mode).into());
                        Value::Mapping(map)
                    }
                    program::Condition::NamedMode(name) => {
                        let mut map = Mapping::new();
                        map.insert("mode".into(), name.clone().into());
                        Value::Mapping(map)
                    }
                    _ => condition.label().into(),
                };
                let mut map = Mapping::new();
                map.insert("if".into(), condition);
                map.insert("then".into(), sequence(then)?);
                map.insert("else".into(), sequence(otherwise)?);
                Value::Mapping(map)
            }
        })
    }
    fn sequence(program: &program::Program) -> Result<Value, String> {
        program
            .steps()
            .iter()
            .map(value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Sequence)
    }
    if program.steps().len() == 1 {
        value(&program.steps()[0])
    } else {
        sequence(program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_control_flow_round_trips_without_changing_semantics() {
        for source in [
            "[{action: mode-enter, arg: select, temporary: true}, {if: {mode: select}, then: [yank], else: []}]",
            "[{action: mode-enter, arg: review, temporary: false}, {if: {mode: review}, then: [move-right], else: [move-left]}]",
            "[{if: recording-macro, then: [macro-stop-recording], else: []}]",
            "[{if: tree-entry-expanded, then: [directory-tree-collapse], else: [directory-tree-expand]}]",
            "[{if: tree-entry-is-directory, then: [directory-tree-expand], else: []}]",
            "[{if: tree-entry-is-root, then: [], else: [directory-tree-select-parent]}]",
        ] {
            let definitions: Vec<StepDef> = serde_yaml::from_str(source).unwrap();
            let original = compile(&definitions, 0, false).unwrap();
            let value = yaml(&original).unwrap();
            let decoded = if value.is_sequence() {
                serde_yaml::from_value::<Vec<StepDef>>(value).unwrap()
            } else {
                vec![serde_yaml::from_value::<StepDef>(value).unwrap()]
            };
            assert_eq!(compile(&decoded, 0, false).unwrap(), original, "{source}");
        }
    }

    #[test]
    fn direct_and_value_decoders_agree_on_types_presence_and_bounds() {
        let mut cases = vec![
            ("yank".to_owned(), true),
            ("{action: help, arg: ''}".to_owned(), true),
            ("{action: help, arg: key-actions}".to_owned(), true),
        ];
        for source in [
            "{action: yank, then: null}",
            "{action: yank, else: null}",
            "{action: yank, if: null}",
            "{action: help, arg: null}",
            "{action: help, arg: 2}",
            "{if: {mode: null}, then: [], else: []}",
            "{if: {mode: 1}, then: [], else: []}",
            "{if: {mode: true}, then: [], else: []}",
            "{action: true}",
            "{action: null}",
            "{action: ''}",
            "{command: null}",
            "{argument: null, action: yank}",
            "{pipe: sort}",
            "{action: yank, count: null}",
            "{action: yank, register: null}",
            "{action: yank, extend: false}",
            "{action: yank, temporary: false}",
            "{action: yank, extra: true}",
        ] {
            cases.push((source.into(), false));
        }
        cases.push((
            format!("{{action: help, arg: '{}'}}", "x".repeat(4096)),
            true,
        ));
        cases.push((
            format!("{{action: help, arg: '{}'}}", "x".repeat(4097)),
            false,
        ));
        cases.push(("x".repeat(129), false));
        cases.push((r#"{action: help, arg: "a\nb"}"#.into(), false));
        for (source, valid) in cases {
            let direct = serde_yaml::from_str::<StepDef>(&source);
            let value = serde_yaml::from_value::<StepDef>(serde_yaml::from_str(&source).unwrap());
            assert_eq!(direct.is_ok(), valid, "{source}: {direct:?}");
            assert_eq!(value.is_ok(), valid, "{source}: {value:?}");
            if valid {
                assert_eq!(direct.unwrap(), value.unwrap());
            }
        }
    }
}
