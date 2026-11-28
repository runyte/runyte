// SPDX-License-Identifier: MPL-2.0
//! Embedded semantic presets. Parsing is independent of user override
//! admission, whose inventory itself depends on this default registry.
use super::*;
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};
use serde_yaml::Value;

pub const NAMES: &[&str] = &["runyte"];
pub const RUNYTE: &str = include_str!("presets/runyte.yaml");
static DOCUMENT: LazyLock<Preset> =
    LazyLock::new(|| serde_yaml::from_str(RUNYTE).expect("embedded preset YAML"));

// Preserve authored order without a second registry or an additional map
// dependency. Duplicate authored keys are errors before normalization too.
#[derive(Debug, PartialEq)]
struct Ordered<T>(Vec<(String, T)>);
impl<T> Default for Ordered<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Ordered<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Entries<T> {
            type Value = Ordered<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an ordered mapping")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                let mut seen = std::collections::HashSet::new();
                while let Some((key, value)) = map.next_entry::<String, T>()? {
                    if !seen.insert(key.clone()) {
                        return Err(serde::de::Error::custom(format!("duplicate key {key}")));
                    }
                    entries.push((key, value));
                }
                Ok(Ordered(entries))
            }
        }
        deserializer.deserialize_map(Entries(std::marker::PhantomData))
    }
}
type ModeTable = Ordered<Ordered<BindingDef>>;
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Preset {
    #[serde(default)]
    keys: Tables,
    #[serde(default)]
    modes: mode_graph::Definitions,
    #[serde(default)]
    buffer_types: mode_graph::BufferTypes,
    #[serde(default)]
    namespaces: Vec<Namespace>,
    #[serde(default)]
    context_actions: Vec<Context>,
    #[serde(default)]
    fast_pane: Tables,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Tables {
    #[serde(default)]
    bind: ModeTable,
}
#[derive(Debug, PartialEq)]
enum BindingDef {
    Detailed(Detail),
    Steps(Vec<StepDef>),
    Action(StepDef),
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Detail {
    steps: Vec<StepDef>,
    description: Option<String>,
    target: Option<String>,
    role: Option<Role>,
    availability: Option<Availability>,
    alias: Option<String>,
    alias_modes: Option<Vec<Mode>>,
    platforms: Option<Vec<Platform>>,
}
/// Embedded-only applicability; all bindings apply everywhere when omitted.
#[derive(Clone, Copy, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Platform {
    Macos,
    Windows,
    Linux,
    #[serde(skip)]
    Other,
}
impl Platform {
    const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Role {
    Primary,
    Fast,
    Compatibility,
}
#[derive(Debug, PartialEq)]
enum Availability {
    Planned(String),
    Unsupported(String),
}
impl<'de> Deserialize<'de> for Availability {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut values = Ordered::<String>::deserialize(deserializer)?.0;
        if values.len() != 1 {
            return Err(serde::de::Error::custom("expected one availability reason"));
        }
        let (kind, reason) = values.pop().unwrap();
        match kind.as_str() {
            "planned" => Ok(Self::Planned(reason)),
            "unsupported" => Ok(Self::Unsupported(reason)),
            _ => Err(serde::de::Error::custom("unknown availability")),
        }
    }
}

#[derive(Debug, PartialEq)]
enum StepDef {
    Name(String),
    Enter(Enter),
    Conditional(Conditional),
    Command(NamedCommand),
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct Enter {
    action: String,
    arg: String,
    #[serde(default)]
    temporary: bool,
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct NamedCommand {
    command: String,
    argument: Option<String>,
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct Conditional {
    #[serde(rename = "if")]
    condition: ConditionDef,
    then: Vec<StepDef>,
    #[serde(rename = "else")]
    otherwise: Vec<StepDef>,
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(untagged)]
enum ConditionDef {
    Mode(ModeCondition),
    Name(ConditionName),
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct ModeCondition {
    mode: Mode,
}
#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum ConditionName {
    RecordingMacro,
    TreeEntryExpanded,
    TreeEntryIsDirectory,
    TreeEntryIsRoot,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Namespace {
    scope: String,
    modes: Vec<Mode>,
    key: String,
    description: String,
    capability: Option<Capability>,
}
#[derive(Deserialize, Clone, Copy)]
enum Capability {
    GitProject,
    LspDocument,
    Syntax,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Context {
    scope: String,
    key: String,
    name: String,
    target: String,
    description: String,
    context: ContextKind,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
enum ContextKind {
    Row,
    Buffer,
}

// Dispatch on YAML shape directly. Untagged enum trial decoding buffers and
// retries every binding, including hundreds of simple action names.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct BindingMap {
    steps: Option<Vec<StepDef>>,
    description: Option<String>,
    target: Option<String>,
    role: Option<Role>,
    availability: Option<Availability>,
    alias: Option<String>,
    alias_modes: Option<Vec<Mode>>,
    platforms: Option<Vec<Platform>>,
    action: Option<String>,
    arg: Option<String>,
    temporary: Option<bool>,
    command: Option<String>,
    argument: Option<String>,
    #[serde(rename = "if")]
    condition: Option<ConditionDef>,
    then: Option<Vec<StepDef>>,
    #[serde(rename = "else")]
    otherwise: Option<Vec<StepDef>>,
}
impl BindingMap {
    fn into_step<E: serde::de::Error>(self) -> Result<StepDef, E> {
        if self.temporary.is_some() && self.action.as_deref() != Some("mode-enter") {
            return Err(E::custom("temporary requires mode-enter"));
        }
        if self.steps.is_some()
            || self.description.is_some()
            || self.target.is_some()
            || self.role.is_some()
            || self.availability.is_some()
            || self.alias.is_some()
            || self.alias_modes.is_some()
            || self.platforms.is_some()
        {
            return Err(E::custom("binding metadata requires steps"));
        }
        match (
            self.action,
            self.arg,
            self.command,
            self.argument,
            self.condition,
            self.then,
            self.otherwise,
        ) {
            (Some(action), Some(arg), None, None, None, None, None) => Ok(StepDef::Enter(Enter {
                action,
                arg,
                temporary: self.temporary.unwrap_or(false),
            })),
            (None, None, Some(command), argument, None, None, None) => {
                Ok(StepDef::Command(NamedCommand { command, argument }))
            }
            (None, None, None, None, Some(condition), Some(then), Some(otherwise)) => {
                Ok(StepDef::Conditional(Conditional {
                    condition,
                    then,
                    otherwise,
                }))
            }
            _ => Err(E::custom(
                "expected one complete action/arg, command/argument or if/then/else step",
            )),
        }
    }
}
impl<'de> Deserialize<'de> for BindingDef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct BindingVisitor;
        impl<'de> Visitor<'de> for BindingVisitor {
            type Value = BindingDef;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an action, step sequence, or described binding")
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<Self::Value, E> {
                Ok(BindingDef::Action(StepDef::Name(s.into())))
            }
            fn visit_string<E: serde::de::Error>(self, s: String) -> Result<Self::Value, E> {
                Ok(BindingDef::Action(StepDef::Name(s)))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut steps = Vec::new();
                while let Some(step) = seq.next_element()? {
                    steps.push(step);
                }
                Ok(BindingDef::Steps(steps))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                let mut record =
                    BindingMap::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                if record.platforms.as_ref().is_some_and(Vec::is_empty) {
                    return Err(serde::de::Error::custom("platforms must not be empty"));
                }
                if let Some(steps) = record.steps.take() {
                    if record.temporary.is_some()
                        || record.action.is_some()
                        || record.arg.is_some()
                        || record.command.is_some()
                        || record.argument.is_some()
                        || record.condition.is_some()
                        || record.then.is_some()
                        || record.otherwise.is_some()
                    {
                        return Err(serde::de::Error::custom(
                            "steps cannot mix with a single action",
                        ));
                    }
                    Ok(BindingDef::Detailed(Detail {
                        steps,
                        description: record.description,
                        target: record.target,
                        role: record.role,
                        availability: record.availability,
                        alias: record.alias,
                        alias_modes: record.alias_modes,
                        platforms: record.platforms,
                    }))
                } else {
                    record.into_step().map(BindingDef::Action)
                }
            }
        }
        d.deserialize_any(BindingVisitor)
    }
}
impl<'de> Deserialize<'de> for StepDef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct StepVisitor;
        impl<'de> Visitor<'de> for StepVisitor {
            type Value = StepDef;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an action name or program step")
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<Self::Value, E> {
                Ok(StepDef::Name(s.into()))
            }
            fn visit_string<E: serde::de::Error>(self, s: String) -> Result<Self::Value, E> {
                Ok(StepDef::Name(s))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                BindingMap::deserialize(serde::de::value::MapAccessDeserializer::new(map))?
                    .into_step()
            }
        }
        d.deserialize_any(StepVisitor)
    }
}

pub(super) fn load(fast: bool) -> Keymap {
    #[cfg(feature = "startup-timing")]
    let timer = std::time::Instant::now();
    let document = &*DOCUMENT;
    #[cfg(feature = "startup-timing")]
    TIMINGS[0].store(
        timer.elapsed().as_nanos() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    compile(document, fast).expect("embedded Runyte preset must validate")
}

pub(super) fn has_stage5_steps(value: &Value) -> bool {
    match value {
        Value::String(name) => name == "mode-enter",
        Value::Sequence(values) => values.iter().any(has_stage5_steps),
        Value::Mapping(map) => {
            map.contains_key("if")
                || map.contains_key("action")
                || map.get("steps").is_some_and(has_stage5_steps)
                || map
                    .get("command")
                    .is_some_and(|v| v.as_str() == Some("mode-enter"))
        }
        _ => false,
    }
}
pub(super) const STAGE5_DIAGNOSTIC: &str =
    "mode-enter and conditional steps are reserved for embedded presets until stage 5";
fn modes(values: &[Mode]) -> Result<&'static [Mode], String> {
    match values {
        [Mode::Normal, Mode::Select] => Ok(MODAL),
        [Mode::Insert, Mode::Replace] => Ok(INSERT),
        [m] => Ok(actions::mode_slice(*m)),
        _ => Err("unsupported preset mode group".into()),
    }
}
fn scope(name: &str) -> Result<BindingScope, String> {
    BindingScope::ALL
        .iter()
        .copied()
        .find(|s| s.name() == name)
        .ok_or_else(|| format!("unknown scope {name}"))
}
fn target(name: &str) -> Result<BindingTarget, String> {
    static TARGETS: LazyLock<HashMap<&'static str, BindingTarget>> = LazyLock::new(|| {
        let mut result = HashMap::new();
        for spec in COMMANDS {
            if let CommandId::Colon(c) = spec.id {
                result.insert(spec.name, c.into());
            }
        }
        for &c in EditorCommand::ALL {
            result.insert(c.metadata().name, c.into());
        }
        result
    });
    TARGETS
        .get(name)
        .copied()
        .ok_or_else(|| format!("unknown preset target {name}"))
}
fn program(steps: &[StepDef], depth: usize) -> Result<program::Program, String> {
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
                    S::Action(match target(name)? {
                        BindingTarget::Editor(c) => actions::Action::Editor(c),
                        t => actions::Action::Target(t),
                    })
                }
                StepDef::Enter(Enter {
                    action,
                    arg,
                    temporary,
                }) => {
                    if action != "mode-enter" {
                        return Err("invalid mode-enter action/arg".into());
                    }
                    mode_graph::valid_name(arg)?;
                    S::Action(actions::Action::Invocation(
                        CommandInvocation::from_parts(
                            crate::command::CommandId::Editor(
                                crate::command::EditorCommand::ModeEnter,
                            ),
                            if *temporary {
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
                }
                StepDef::Command(command) => {
                    if command.command == "mode-enter" {
                        return Err("mode-enter requires action/arg".into());
                    }
                    let mut value = serde_yaml::Mapping::new();
                    value.insert("command".into(), command.command.clone().into());
                    if let Some(arg) = &command.argument {
                        value.insert("argument".into(), arg.clone().into());
                    }
                    S::Action(actions::parse_action(&Value::Mapping(value), Mode::Normal, true)?.0)
                }
                StepDef::Conditional(c) => S::If {
                    condition: match &c.condition {
                        ConditionDef::Mode(m) => C::Mode(m.mode),
                        ConditionDef::Name(n) => match n {
                            ConditionName::RecordingMacro => C::RecordingMacro,
                            ConditionName::TreeEntryExpanded => C::TreeEntryExpanded,
                            ConditionName::TreeEntryIsDirectory => C::TreeEntryIsDirectory,
                            ConditionName::TreeEntryIsRoot => C::TreeEntryIsRoot,
                        },
                    },
                    then: Box::new(program(&c.then, depth + 1)?),
                    otherwise: Box::new(program(&c.otherwise, depth + 1)?),
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Program::from_steps(steps).map_err(str::to_owned)
}
fn validate_entry_targets(
    program: &program::Program,
    graph: &mode_graph::ModeGraph,
) -> Result<(), String> {
    for action in program.iter() {
        if let actions::Action::Invocation(invocation) = action
            && invocation.id() == CommandId::Editor(EditorCommand::ModeEnter)
            && let crate::command::InvocationParameters::OptionalText(Some(name))
            | crate::command::InvocationParameters::TemporaryMode(name) = invocation.parameters()
            && (matches!(name.as_str(), "list" | "session-manager")
                || (mode_graph::EditingRole::named(name).is_none() && graph.id(name).is_none()))
        {
            return Err(format!("unknown editor mode {name:?}"));
        }
    }
    Ok(())
}
fn binding(
    value: &'static BindingDef,
    scope: BindingScope,
    mode: Mode,
    key: &str,
) -> Result<Binding, String> {
    let (steps, metadata) = match value {
        BindingDef::Detailed(d) => (d.steps.as_slice(), Some(d)),
        BindingDef::Steps(s) => (s.as_slice(), None),
        BindingDef::Action(s) => (std::slice::from_ref(s), None),
    };
    let program = program(steps, 0)?;
    program.validate(scope).map_err(str::to_owned)?;
    let target = if let Some(name) = metadata.and_then(|d| d.target.as_deref()) {
        target(name)?
    } else {
        program
            .iter()
            .next()
            .ok_or("empty binding program")?
            .target()
    };
    let mut result = Binding::implemented_in(
        actions::mode_slice(mode),
        scope,
        KeySequence::parse(key)?,
        target,
    );
    result.program = program;
    if let Some(d) = metadata {
        if let Some(description) = &d.description {
            result.description = description.as_str().into();
        }
        result.role = match d.role {
            None | Some(Role::Primary) => BindingRole::Primary,
            Some(Role::Fast) => BindingRole::Fast,
            Some(Role::Compatibility) => BindingRole::Compatibility,
        };
        result.availability = match &d.availability {
            None => BindingAvailability::Implemented,
            Some(Availability::Planned(s)) => BindingAvailability::Planned(s),
            Some(Availability::Unsupported(s)) => BindingAvailability::Unsupported(s),
        };
        result.alias = d
            .alias
            .as_ref()
            .map(|s| KeySequence::parse(s))
            .transpose()?;
        result.alias_modes = d.alias_modes.as_ref().map(|ms| modes(ms)).transpose()?;
        if result.alias_modes.is_some() && result.alias.is_none() {
            return Err("alias-modes requires alias".into());
        }
    }
    Ok(result)
}
fn bindings(
    bind: &'static ModeTable,
    tables: &named::Tables,
    platform: Platform,
) -> Result<Vec<Binding>, String> {
    let mut result = Vec::new();
    for (name, entries) in &bind.0 {
        let id = tables
            .graph
            .id(name)
            .ok_or_else(|| format!("missing mode {name}"))?;
        let (scope, active) = if name == "list" {
            (BindingScope::Global, vec![Mode::List])
        } else if name == "session-manager" {
            (BindingScope::SessionManager, vec![Mode::List])
        } else {
            let mut owners = Vec::new();
            let mut active = Vec::new();
            for &scope in BindingScope::ALL {
                if scope == BindingScope::SessionManager {
                    continue;
                }
                for mode in [Mode::Normal, Mode::Select, Mode::Insert, Mode::Replace] {
                    if tables.mode_for(mode, scope).is_some_and(|start| {
                        tables.graph.chain(start).any(|ancestor| ancestor == id)
                    }) {
                        if tables.mode_for(mode, scope) == Some(id) && !owners.contains(&scope) {
                            owners.push(scope);
                        }
                        if !active.contains(&mode) {
                            active.push(mode);
                        }
                    }
                }
            }
            (
                if owners.len() == 1 {
                    owners[0]
                } else {
                    BindingScope::Global
                },
                active,
            )
        };
        let active = modes(&active)?;
        let first = *active
            .first()
            .ok_or_else(|| format!("preset mode {name} is unreachable"))?;
        for (key, value) in &entries.0 {
            if let BindingDef::Detailed(detail) = value
                && detail
                    .platforms
                    .as_ref()
                    .is_some_and(|allowed| !allowed.contains(&platform))
            {
                continue;
            }
            let mut binding = binding(value, scope, first, key)?;
            validate_entry_targets(&binding.program, &tables.graph)?;
            binding.modes = active;
            binding.binding_mode = Some(name.as_str().into());
            result.push(binding);
        }
    }
    Ok(result)
}
fn compile(document: &'static Preset, fast: bool) -> Result<Keymap, String> {
    compile_for_platform(document, fast, Platform::current())
}
fn compile_for_platform(
    document: &'static Preset,
    fast: bool,
    platform: Platform,
) -> Result<Keymap, String> {
    #[cfg(feature = "startup-timing")]
    let timer = std::time::Instant::now();
    let binding_modes = document
        .keys
        .bind
        .0
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    let graph = mode_graph::ModeGraph::compile(
        &document.modes,
        &binding_modes,
        &[],
        &document.buffer_types,
        &named::required_buffer_types(),
    )?;
    let tables = named::Tables {
        graph,
        definitions: document.modes.clone(),
        buffer_types: document.buffer_types.clone(),
        removed: HashMap::new(),
        plugin_modes: Default::default(),
    };
    let mut bindings = bindings(&document.keys.bind, &tables, platform)?;
    if fast {
        let replacements = self::bindings(&document.fast_pane.bind, &tables, platform)?;
        bindings.retain(|b| {
            !replacements
                .iter()
                .any(|r| r.sequence == b.sequence && r.binding_mode == b.binding_mode)
        });
        bindings.extend(replacements);
    }
    let namespaces = document
        .namespaces
        .iter()
        .map(|n| {
            Ok(BindingNamespace {
                scope: scope(&n.scope)?,
                modes: modes(&n.modes)?,
                sequence: KeySequence::parse(&n.key)?,
                description: &n.description,
                capability: n.capability.map(|c| match c {
                    Capability::GitProject => CommandCapability::GitProject,
                    Capability::LspDocument => CommandCapability::LspDocument,
                    Capability::Syntax => CommandCapability::Syntax,
                }),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let contexts = document
        .context_actions
        .iter()
        .map(|c| {
            Ok(ContextAction {
                scope: scope(&c.scope)?,
                mnemonic: Key::parse(&c.key)?,
                name: &c.name,
                target: target(&c.target)?,
                description: &c.description,
                context: match c.context {
                    ContextKind::Row => ActionContext::Row,
                    ContextKind::Buffer => ActionContext::Buffer,
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    // Cross-scope invariants of immutable embedded sources run in the preset
    // tests. Dynamic user overrides still run the full validator at load.
    #[cfg(feature = "startup-timing")]
    TIMINGS[1].store(
        timer.elapsed().as_nanos() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    #[cfg(feature = "startup-timing")]
    let timer = std::time::Instant::now();
    let mut map = Keymap::new_with_overrides(
        bindings,
        super::effective::Overrides(Default::default(), Some(Arc::new(tables))),
    )
    .map_err(|e| e.to_string())?
    .with_context_actions(contexts)
    .with_fast_pane_keys_enabled(fast);
    map.namespaces = namespaces;
    map.record_identity_spellings();
    let role_errors = named::role_target_errors(&map);
    if !role_errors.is_empty() {
        return Err(role_errors.join("; "));
    }
    #[cfg(feature = "startup-timing")]
    TIMINGS[2].store(
        timer.elapsed().as_nanos() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parsed_program(source: &str) -> Result<program::Program, String> {
        let steps: Vec<StepDef> = serde_yaml::from_str(source).map_err(|e| e.to_string())?;
        let program = program(&steps, 0)?;
        validate_entry_targets(&program, keymap_for(false).mode_graph().unwrap())?;
        Ok(program)
    }

    #[test]
    fn every_embedded_preset_validates_in_both_variants() {
        assert_eq!(NAMES, ["runyte"]);
        for (platform, name) in [
            (Platform::Macos, "macos"),
            (Platform::Windows, "windows"),
            (Platform::Linux, "linux"),
            (Platform::Other, "other"),
        ] {
            let mut preferred = Vec::new();
            for fast in [false, true] {
                let map = compile_for_platform(&DOCUMENT, fast, platform).unwrap();
                validate::assert_valid(&map);
                parity::assert_registry(&map, fast, name);
                for &scope in BindingScope::ALL {
                    for mode in [
                        Mode::Normal,
                        Mode::Select,
                        Mode::Insert,
                        Mode::Replace,
                        Mode::List,
                    ] {
                        let legacy = super::super::effective::indices(
                            &map.bindings,
                            &super::super::effective::Overrides::default(),
                            mode,
                            scope,
                        );
                        let named = super::super::effective::indices(
                            &map.bindings,
                            &map.overrides,
                            mode,
                            scope,
                        );
                        let rows = |indices: Vec<usize>| {
                            let mut rows = indices
                                .into_iter()
                                .map(|i| {
                                    let binding = &map.bindings[i];
                                    format!(
                                        "{} {:?} {:?}",
                                        binding.sequence, binding.target, binding.program
                                    )
                                })
                                .collect::<Vec<_>>();
                            rows.sort();
                            rows
                        };
                        assert_eq!(
                            rows(named),
                            rows(legacy),
                            "{name} fast={fast} {scope:?} {mode:?}"
                        );
                    }
                }
                preferred.extend(parity::preferred_rows(&map, fast));
            }
            parity::assert_preferred(preferred);
        }
    }

    #[test]
    fn temporary_mode_entry_is_visible_in_default_program_discovery() {
        let map = keymap_for(false);
        for key in ["x", "X", "m i p", "m a p"] {
            let sequence = KeySequence::parse(key).unwrap();
            let binding = map
                .bindings_for_scope(Mode::Normal, BindingScope::Global)
                .find(|binding| binding.sequence == sequence)
                .unwrap();
            let text = crate::command::discovery::binding_steps(binding, BindingScope::Global);
            assert!(text.contains("temporary: true"), "{key}: {text}");
        }
    }

    #[test]
    fn preset_programs_reject_unknown_fields_conditions_and_bounds() {
        for source in [
            "[{if: mystery, then: [], else: []}]",
            "[{if: {mode: mystery}, then: [], else: []}]",
            "[{if: {mode: normal, extra: true}, then: [], else: []}]",
            "[{if: recording-macro, then: [], else: [], extra: true}]",
            "[{action: mode-enter, arg: list}]",
            "[{action: mode-enter, arg: mystery}]",
            "[{action: unknown, arg: normal}]",
            "[{action: mode-enter, arg: normal, extra: true}]",
            "[{action: mode-enter, arg: select, temporary: nope}]",
            "[{command: yank, temporary: true}]",
            "[{if: recording-macro, then: [], else: [], temporary: true}]",
            "[{steps: [yank], temporary: true}]",
            "[unknown-action]",
            "[mode-enter]",
            "[{command: mode-enter}]",
            "[{}]",
        ] {
            assert!(parsed_program(source).is_err(), "{source}");
        }
        let mut nested = Value::Sequence(vec![]);
        for _ in 0..6 {
            nested = serde_yaml::to_value(
                serde_json::json!([{"if":"recording-macro","then":nested,"else":[]}]),
            )
            .unwrap();
        }
        assert!(parsed_program(&serde_yaml::to_string(&nested).unwrap()).is_err());
        let too_many = Value::Sequence(vec![Value::String("move-left".into()); 17]);
        assert!(parsed_program(&serde_yaml::to_string(&too_many).unwrap()).is_err());
        let invalid_tail: Value = serde_yaml::from_str(
            "[{if: recording-macro, then: [open-command-palette], else: []}, move-left]",
        )
        .unwrap();
        assert!(
            parsed_program(&serde_yaml::to_string(&invalid_tail).unwrap())
                .unwrap()
                .validate(BindingScope::Global)
                .is_err()
        );
        let tree: Value =
            serde_yaml::from_str("[{if: tree-entry-expanded, then: [move-left], else: []}]")
                .unwrap();
        assert!(
            parsed_program(&serde_yaml::to_string(&tree).unwrap())
                .unwrap()
                .validate(BindingScope::Global)
                .is_err()
        );
    }

    #[test]
    fn descriptions_are_bounded_and_preset_metadata_is_not_user_authority() {
        let base = keymap_for(false);
        for label in [
            "x".repeat(1025),
            "line\nbreak".into(),
            "escape\u{1b}".into(),
            "é".repeat(513),
        ] {
            let value=serde_yaml::to_value(serde_json::json!({"bind":{"normal":{"F6":{"steps":["move-left"],"description":label}}}})).unwrap();
            let result = configured::compile(&value, &base);
            assert!(
                result.errors.iter().any(|e| e.contains("description")),
                "{:?}",
                result.errors
            );
            assert_eq!(result.keymap.bindings(), base.bindings());
        }
        let value=serde_yaml::to_value(serde_json::json!({"bind":{"normal":{"F6":{"steps":["move-left"],"description":"é".repeat(512)}}}})).unwrap();
        assert!(configured::compile(&value, &base).errors.is_empty());
        for extra in [
            "role: primary",
            "target: move-left",
            "availability: {unsupported: reason}",
            "alias: h",
            "alias-modes: [normal]",
            "platforms: [macos]",
        ] {
            let value = serde_yaml::from_str(&format!(
                "bind: {{normal: {{F6: {{steps: [move-left], {extra}}}}}}}"
            ))
            .unwrap();
            assert!(
                configured::compile(&value, &base)
                    .errors
                    .iter()
                    .any(|e| e.contains("reserved for embedded presets"))
            );
        }
        for steps in ["null", "move-left", "[]"] {
            let value =
                serde_yaml::from_str(&format!("bind: {{normal: {{F6: {{steps: {steps}}}}}}}"))
                    .unwrap();
            assert!(!configured::compile(&value, &base).errors.is_empty());
        }
    }

    #[test]
    fn preset_schema_rejects_unknown_and_duplicate_authored_fields() {
        for source in [
            "mystery: true",
            "bind: {normal: {x: {steps: [move-left], platforms: []}}}",
            "bind: {normal: {x: {steps: [move-left], platforms: [unknown]}}}",
            "bind: {normal: {x: {command: move-left, platforms: [macos]}}}",
            "bind: {normal: {x: move-left, x: move-right}}",
            "bind: {normal: {x: {steps: [move-left], mystery: true}}}",
            "bind: {normal: {x: {steps: [move-left], role: other}}}",
            "namespaces: [{scope: global, modes: [normal], key: g, description: Goto, capability: Other}]",
        ] {
            assert!(serde_yaml::from_str::<Preset>(source).is_err(), "{source}");
        }
    }

    #[test]
    fn preset_selection_overrides_and_private_step_gates() {
        for fast in [false, true] {
            let base = keymap_for(fast);
            for source in ["{}", "preset: runyte"] {
                let result = configured::compile(&serde_yaml::from_str(source).unwrap(), &base);
                assert!(result.errors.is_empty(), "{:?}", result.errors);
                assert_eq!(result.keymap.bindings(), base.bindings());
            }
            let unknown =
                configured::compile(&serde_yaml::from_str("preset: ../../file").unwrap(), &base);
            assert!(
                unknown
                    .errors
                    .iter()
                    .any(|e| e.contains("unknown embedded preset"))
            );
            assert_eq!(unknown.keymap.bindings(), base.bindings());
            for source in [
                "preset: 12",
                "preset: []",
                "namespaces: []",
                "context-actions: []",
                "fast-pane: {}",
            ] {
                let result = configured::compile(&serde_yaml::from_str(source).unwrap(), &base);
                assert!(!result.errors.is_empty(), "{source}");
                assert_eq!(result.keymap.bindings(), base.bindings());
            }
            for step in [
                "{action: mode-enter, arg: select}",
                "{action: mode-enter, arg: select, temporary: true}",
                "{if: {mode: select}, then: [move-left], else: []}",
                "mode-enter",
                "{command: mode-enter}",
            ] {
                for source in [
                    format!("bind: {{normal: {{F6: {step}}}}}"),
                    format!("bind: {{directory: {{F6: {step}}}}}"),
                ] {
                    let result =
                        configured::compile(&serde_yaml::from_str(&source).unwrap(), &base);
                    assert!(
                        result.errors.iter().any(|e| e.contains("stage 5")),
                        "{source}: {:?}",
                        result.errors
                    );
                    assert_eq!(result.keymap.bindings(), base.bindings());
                }
            }
            let described = configured::compile(&serde_yaml::from_str("preset: runyte\nbind:\n  normal:\n    F6: {steps: [select-all, yank], description: Copy document}").unwrap(), &base);
            assert!(described.errors.is_empty(), "{:?}", described.errors);
            let Lookup::Exact(binding) = described
                .keymap
                .lookup(Mode::Normal, &KeySequence::parse("F6").unwrap())
            else {
                panic!("missing binding")
            };
            assert_eq!(binding.description, "Copy document");
            assert_eq!(binding.program.len(), 2);
        }
    }
}

/// Observation-only counters; ordinary builds contain no clocks or reporting.
#[cfg(feature = "startup-timing")]
static TIMINGS: [std::sync::atomic::AtomicU64; 3] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 3];
/// YAML parsing, program/metadata construction, and pre-existing index build.
#[cfg(feature = "startup-timing")]
pub fn initialization_timings_ns() -> [u64; 3] {
    std::array::from_fn(|i| TIMINGS[i].load(std::sync::atomic::Ordering::Relaxed))
}

#[cfg(test)]
#[path = "../../tests/preset_support/mod.rs"]
mod parity;
