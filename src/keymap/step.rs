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
        || arg.is_some_and(|arg| {
            arg.len() > 4096
                || arg.chars().any(|c| {
                    c.is_control() && !(name == "insert-text" && matches!(c, '\n' | '\r' | '\t'))
                })
        })
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
    pub options: program::StepOptions,
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
    BufferType(BufferTypeCondition),
    Name(ConditionName),
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct ModeCondition {
    #[serde(deserialize_with = "strict_string")]
    pub mode: String,
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct BufferTypeCondition {
    #[serde(rename = "buffer-type", deserialize_with = "strict_string")]
    pub buffer_type: String,
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ConditionName {
    SelectionHasText,
    MultipleSelections,
    RegisterEmpty,
    ReadOnly,
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
        let has_options = self.count.is_some() || self.register.is_some() || self.extend.is_some();
        if has_options && self.action.is_none() {
            return Err(E::custom("step options require an action"));
        }
        let options = program::StepOptions {
            count: parse_prefix(self.count, |v| {
                v.as_u64()
                    .filter(|n| (1..=999_999).contains(n))
                    .map(|n| n as usize)
            })
            .map_err(E::custom)?,
            register: parse_prefix(self.register, |v| {
                v.as_str().and_then(|s| {
                    let mut chars = s.chars();
                    let c = chars.next()?;
                    (chars.next().is_none() && !c.is_control()).then_some(c)
                })
            })
            .map_err(E::custom)?,
            extend: self
                .extend
                .map(|v| {
                    v.as_bool()
                        .ok_or_else(|| E::custom("extend must be a boolean"))
                })
                .transpose()?,
        };
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
                    options,
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

fn parse_prefix<T>(
    value: Option<serde_yaml::Value>,
    fixed: impl FnOnce(&serde_yaml::Value) -> Option<T>,
) -> Result<program::Prefix<T>, &'static str> {
    use program::Prefix;
    let Some(value) = value else {
        return Ok(Prefix::Consume);
    };
    match value.as_str() {
        Some("inherit") => Ok(Prefix::Inherit),
        Some("none") => Ok(Prefix::None),
        _ => fixed(&value)
            .map(Prefix::Fixed)
            .ok_or("invalid step prefix override"),
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
                    options,
                }) => {
                    let step = if action == "mode-enter" {
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
                    };
                    let S::Action(action) = step else {
                        unreachable!()
                    };
                    options.validate(&action)?;
                    if *options == program::StepOptions::default() {
                        S::Action(action)
                    } else {
                        S::ActionWithOptions {
                            action,
                            options: *options,
                        }
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
                        ConditionDef::BufferType(b) => {
                            if !super::named::required_buffer_types()
                                .contains(&b.buffer_type.as_str())
                            {
                                return Err(format!("unknown buffer type {:?}", b.buffer_type));
                            }
                            C::BufferType(b.buffer_type.clone())
                        }
                        ConditionDef::Name(n) => match n {
                            ConditionName::SelectionHasText => C::SelectionHasText,
                            ConditionName::MultipleSelections => C::MultipleSelections,
                            ConditionName::RegisterEmpty => C::RegisterEmpty,
                            ConditionName::ReadOnly => C::ReadOnly,
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

/// Stable discovery label from executable identities and typed arguments.
/// Authored prose is separate; generated labels are bounded valid metadata.
pub(super) fn description(program: &program::Program) -> String {
    use crate::command::{HelpInvocation, InvocationParameters as P};
    let mut text = String::new();
    for action in program.iter() {
        if !text.is_empty() {
            text.push_str(" → ");
        }
        text.push_str(action.target().name());
        if let actions::Action::Invocation(invocation) = action {
            let argument = match invocation.parameters() {
                P::None => invocation.execution().character().map(|first| {
                    let mut text = first.to_string();
                    if let Some(second) = invocation.execution().second_character() {
                        text.push(second);
                    }
                    text
                }),
                P::Path(path) | P::OptionalPath(Some(path)) => Some(path.display().to_string()),
                P::OptionalText(Some(text)) | P::TemporaryMode(text) => Some(text.clone()),
                P::SessionRename { workspace, name } => {
                    Some(format!("{} {name}", workspace.display()))
                }
                P::Grammar(Some(grammar)) => Some(grammar.name().into()),
                P::Help(HelpInvocation::Manual(Some(topic))) => Some(topic.clone()),
                _ => None,
            };
            if let Some(argument) = argument {
                text.push(' ');
                text.push_str(&argument);
            }
        }
    }
    let mut label = String::new();
    for ch in text.chars() {
        if ch.is_control() {
            let escaped = ch.escape_default().to_string();
            if label.len() + escaped.len() > 1021 {
                label.push('…');
                break;
            }
            label.push_str(&escaped);
        } else {
            if label.len() + ch.len_utf8() > 1021 {
                label.push('…');
                break;
            }
            label.push(ch);
        }
    }
    label
}

pub(super) fn yaml(program: &program::Program) -> Result<serde_yaml::Value, String> {
    use serde_yaml::{Mapping, Value};
    fn path_argument(path: &std::path::Path, selector: bool) -> Result<String, String> {
        let path = path.to_str().ok_or("binding path is not Unicode")?;
        if !path.starts_with(['\'', '"']) && (!selector || !path.chars().any(char::is_whitespace)) {
            return Ok(path.into());
        }
        let quoted = ['"', '\'']
            .into_iter()
            .filter_map(|quote| {
                let inner = if cfg!(windows) {
                    (!path.contains(quote)).then(|| path.to_owned())?
                } else {
                    let mut escaped = String::new();
                    let mut chars = path.chars().peekable();
                    while let Some(ch) = chars.next() {
                        if ch == quote
                            || ch == '\\'
                                && chars
                                    .peek()
                                    .is_none_or(|next| *next == quote || *next == '\\')
                        {
                            escaped.push('\\');
                        }
                        escaped.push(ch);
                    }
                    escaped
                };
                Some(format!("{quote}{inner}{quote}"))
            })
            .min_by_key(String::len);
        quoted.ok_or_else(|| "binding path has no public quoted spelling".into())
    }
    fn value(step: &program::Step) -> Result<Value, String> {
        Ok(match step {
            program::Step::ActionWithOptions { action, options } => {
                let plain = value(&program::Step::Action(action.clone()))?;
                let mut map = match plain {
                    Value::Mapping(map) => map,
                    name => {
                        let mut map = Mapping::new();
                        map.insert("action".into(), name);
                        map
                    }
                };
                fn prefix<T: ToString>(p: program::Prefix<T>) -> Option<Value> {
                    use program::Prefix::*;
                    match p {
                        Consume => Option::None,
                        Inherit => Some("inherit".into()),
                        None => Some("none".into()),
                        Fixed(v) => Some(v.to_string().into()),
                    }
                }
                if let Some(v) = prefix(options.count) {
                    let v = if let program::Prefix::Fixed(n) = options.count {
                        Value::Number(n.into())
                    } else {
                        v
                    };
                    map.insert("count".into(), v);
                }
                if let Some(v) = prefix(options.register) {
                    map.insert("register".into(), v);
                }
                if let Some(v) = options.extend {
                    map.insert("extend".into(), v.into());
                }
                Value::Mapping(map)
            }
            program::Step::Action(action) => {
                let name = action.target().name();
                match action {
                    actions::Action::Invocation(invocation) => {
                        use crate::command::InvocationParameters as P;
                        let mut map = Mapping::new();
                        map.insert("action".into(), name.into());
                        match invocation.parameters() {
                            P::None => {
                                if let Some(first) = invocation.execution().character() {
                                    let mut arg = first.to_string();
                                    if let Some(second) = invocation.execution().second_character()
                                    {
                                        arg.push(second);
                                    }
                                    map.insert("arg".into(), arg.into());
                                } else {
                                    return Ok(name.into());
                                }
                            }
                            P::OptionalText(Some(arg)) => {
                                map.insert("arg".into(), arg.clone().into());
                            }
                            P::TemporaryMode(arg) => {
                                map.insert("arg".into(), arg.clone().into());
                                map.insert("temporary".into(), true.into());
                            }
                            P::OptionalPath(None) | P::OptionalText(None) | P::Grammar(None) => {
                                return Ok(name.into());
                            }
                            P::Path(path) | P::OptionalPath(Some(path)) => {
                                map.insert("arg".into(), path_argument(path, false)?.into());
                            }
                            P::SessionRename { workspace, name } => {
                                let workspace = path_argument(workspace, true)?;
                                map.insert("arg".into(), format!("{workspace} {name}").into());
                            }
                            P::Grammar(Some(grammar)) => {
                                map.insert("arg".into(), grammar.name().into());
                            }
                            P::Help(crate::command::HelpInvocation::Manual(None)) => {
                                return Ok("help".into());
                            }
                            P::Help(crate::command::HelpInvocation::Manual(Some(topic))) => {
                                map.insert("arg".into(), topic.clone().into());
                            }
                            P::Help(crate::command::HelpInvocation::ActiveView) => {
                                return Ok(name.into());
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
                    program::Condition::BufferType(name) => {
                        let mut map = serde_yaml::Mapping::new();
                        map.insert("buffer-type".into(), name.clone().into());
                        map.into()
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
    fn supplied_path_and_colon_arguments_round_trip() {
        let mut cases = vec![
            ("open", "some file.txt".to_owned()),
            ("open", " x ".to_owned()),
            ("open", "a".repeat(4096)),
            ("open", format!(" {} ", "a".repeat(4094))),
            ("open", r#"'"quoted" path'"#.to_owned()),
            ("session-rename", r#""old session" new name"#.to_owned()),
            ("session-rename", "old new name".to_owned()),
            ("write-config", "".to_owned()),
            ("help", "key-actions".to_owned()),
            ("help", "".to_owned()),
            ("grammar", "helix".to_owned()),
        ];
        if cfg!(windows) {
            cases.push(("open", r#""C:\work files\file.txt""#.to_owned()));
            cases.push((
                "session-rename",
                r#""C:\work files\project" New name"#.to_owned(),
            ));
        } else {
            cases.push((
                "session-rename",
                r#""old \"quoted\" session" New name"#.to_owned(),
            ));
            cases.push(("open", r#""a\\b""#.to_owned()));
        }
        #[cfg(not(windows))]
        cases.push(("open", format!("'\"{}a'", "\\z".repeat(2046))));
        for (name, arg) in cases {
            let action = serde_yaml::to_value(std::collections::BTreeMap::from([
                ("action", name),
                ("arg", arg.as_str()),
            ]))
            .unwrap();
            let def: StepDef = serde_yaml::from_value(action).unwrap();
            let original = compile(&[def], 0, false).unwrap();
            let rendered = yaml(&original).unwrap();
            let def: StepDef = serde_yaml::from_value(rendered.clone()).unwrap();
            let reloaded = compile(&[def], 0, false).unwrap();
            assert_eq!(original, reloaded, "{name} {arg:?}");
            assert_eq!(rendered, yaml(&reloaded).unwrap());
        }
    }

    #[test]
    fn public_control_flow_round_trips_without_changing_semantics() {
        for source in [
            "[{action: mode-enter, arg: select, temporary: true}, {if: {mode: select}, then: [yank], else: []}]",
            "[{action: mode-enter, arg: review, temporary: false}, {if: {mode: review}, then: [move-right], else: [move-left]}]",
            "[{if: recording-macro, then: [macro-stop-recording], else: []}]",
            "[{if: {buffer-type: text}, then: [yank], else: []}, {if: selection-has-text, then: [yank], else: []}, {if: multiple-selections, then: [yank], else: []}, {if: register-empty, then: [yank], else: []}, {if: read-only, then: [yank], else: []}]",
            "[{action: find-next-char, arg: λ}, {action: surround-replace, arg: '(]'}]",
            "[{action: search, arg: abc}, {action: join-selections, arg: ''}]",
            "[{action: move-right, count: 3, extend: true}, {action: yank, register: a}]",
            "[{action: move-right, count: inherit, extend: false}, {action: paste-after, count: none, register: none}, {action: yank, register: inherit}]",
            r#"[{action: insert-text, arg: "λ\n\t"}]"#,
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
            "{action: yank, extend: null}",
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
