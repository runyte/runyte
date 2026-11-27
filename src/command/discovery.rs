// SPDX-License-Identifier: MPL-2.0
//! Read-only action discovery. Palette names and shared presentation live here.
pub mod display;
use super::{ColonCommand, CommandId, EditorCommand, Mode};
use crate::keymap::{Binding, BindingScope, BindingTarget, Keymap, actions};
use serde_yaml::{Mapping, Value};

pub const DESCRIBE_KEY: &str = "describe-key";
pub const ACTION_BROWSER: &str = "browse-actions";
pub const COPY_BINDING: &str = "copy-key-binding";

pub fn identities() -> impl Iterator<Item = CommandId> {
    EditorCommand::ALL
        .iter()
        .copied()
        .map(CommandId::Editor)
        .chain(ColonCommand::ALL.iter().copied().map(CommandId::Colon))
}

/// A bounded projection built once per browser, independent of picker filtering.
#[derive(Default)]
pub struct BindingIndex(std::collections::HashMap<CommandId, BindingSummary>);
#[derive(Default)]
struct BindingSummary {
    keys: Vec<String>,
    key_count: usize,
    programs: Vec<String>,
    program_count: usize,
}
impl BindingIndex {
    pub fn new(map: &Keymap, mode: Mode, scope: BindingScope) -> Self {
        let mut index = Self::default();
        for b in map.bindings_for_scope(mode, scope) {
            if b.is_plain_action() {
                let entry = index.0.entry(b.target.id()).or_default();
                entry.key_count += 1;
                if entry.keys.len() < 32 {
                    entry.keys.push(b.sequence.to_string());
                }
            } else {
                let mut members = Vec::new();
                let mut rendered = None;
                for action in b.program.iter() {
                    let id = action.target().id();
                    if members.contains(&id) {
                        continue;
                    }
                    members.push(id);
                    let entry = index.0.entry(id).or_default();
                    entry.program_count += 1;
                    if entry.programs.len() < 4 {
                        let text = rendered.get_or_insert_with(|| {
                            format!(
                                "Composed/argument binding: {}\n{}",
                                b.sequence,
                                binding_steps(b, scope)
                            )
                        });
                        entry.programs.push(text.clone());
                    }
                }
            }
        }
        index
    }
    pub fn keys(&self, id: CommandId) -> Vec<String> {
        self.0
            .get(&id)
            .map(|entry| {
                let mut keys = entry.keys.clone();
                if entry.key_count > keys.len() {
                    keys.push(format!("… {} more", entry.key_count - keys.len()));
                }
                keys
            })
            .unwrap_or_default()
    }
}

/// Keys are a keymap fact, not a claim that an action's runtime guards pass.
pub fn keys(id: CommandId, map: &Keymap, mode: Mode, scope: BindingScope) -> Vec<String> {
    BindingIndex::new(map, mode, scope).keys(id)
}

pub fn details(id: CommandId, map: &Keymap, mode: Mode, scope: BindingScope) -> String {
    let defaults = crate::keymap::keymap_for(map.fast_pane_keys());
    details_indexed(
        id,
        mode,
        scope,
        &BindingIndex::new(&defaults, mode, scope),
        &BindingIndex::new(map, mode, scope),
    )
}

pub fn details_indexed(
    id: CommandId,
    mode: Mode,
    scope: BindingScope,
    defaults: &BindingIndex,
    effective: &BindingIndex,
) -> String {
    let Some(d) = id.descriptor() else {
        return "Runtime plugin action".into();
    };
    let modes = d
        .policy
        .global_binding_modes
        .iter()
        .map(|m| actions::mode_name(*m))
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = format!(
        "{}\n\n{}\n\nView: {}\nMode: {}\nEffects: {}\nCount: {}\nCharacter operands: {}\nSequence position: {}\nGlobal configurable modes: {}\nDefault keys here: {}\nEffective keys here: {}\n",
        d.metadata.name,
        d.metadata.description,
        scope.name(),
        actions::mode_name(mode),
        display::effects(d.policy.effects),
        display::count(d.policy.count),
        display::operands(d.policy.operands),
        display::continuation(d),
        if modes.is_empty() {
            "scope-only or internal"
        } else {
            &modes
        },
        joined(defaults.keys(id)),
        joined(effective.keys(id))
    );
    if let Some(entry) = effective.0.get(&id) {
        for program in &entry.programs {
            text.push_str(program);
        }
        if entry.program_count > entry.programs.len() {
            text.push_str(&format!(
                "… {} more composed/argument bindings\n",
                entry.program_count - entry.programs.len()
            ));
        }
    }
    if let Some(argument) = d.argument() {
        text.push_str(&format!(
            "Palette argument: {} ({}; {})\n",
            argument.usage,
            display::arguments(argument.syntax),
            display::delivery(argument.delivery)
        ));
    } else {
        text.push_str("Palette argument: none\n");
    }
    if let Some(view) = d.view(scope) {
        text.push_str(&format!("Current view: {}\n", view.description));
    }
    for view in d.view_overrides {
        text.push_str(&format!("{}: {}\n", view.scope.name(), view.description));
    }
    text.push_str("\nRuntime availability still depends on the document, capability and action guards. Inspecting an action does not run it.\n");
    text
}
fn joined(keys: Vec<String>) -> String {
    if keys.is_empty() {
        "none".into()
    } else {
        keys.join(", ")
    }
}

pub fn binding_steps(binding: &Binding, scope: BindingScope) -> String {
    render_program(&binding.program, scope, 0)
}

fn render_program(
    program: &crate::keymap::program::Program,
    scope: BindingScope,
    depth: usize,
) -> String {
    use crate::keymap::program::Step;
    let indent = "  ".repeat(depth);
    program
        .steps()
        .iter()
        .enumerate()
        .map(|(index, step)| match step {
            Step::If {
                condition,
                then,
                otherwise,
            } => format!(
                "{indent}{}. if {}\n{indent}then:\n{}{indent}else:\n{}",
                index + 1,
                condition.label(),
                render_program(then, scope, depth + 1),
                render_program(otherwise, scope, depth + 1)
            ),
            Step::Action(action) => {
                let target = action.target();
                let description = target
                    .id()
                    .descriptor()
                    .and_then(|d| {
                        d.view(scope)
                            .map(|v| v.description)
                            .or(Some(d.metadata.description))
                    })
                    .unwrap_or_else(|| target.description());
                let argument = match action {
                    actions::Action::Invocation(invocation) => format!(
                        " (argument: {})",
                        abbreviated(&argument_text(invocation.parameters()))
                    ),
                    _ => String::new(),
                };
                format!(
                    "{indent}{}. {}{} — {}\n",
                    index + 1,
                    target.name(),
                    argument,
                    description
                )
            }
        })
        .collect()
}

fn abbreviated(text: &str) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(256).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

fn argument_text(value: &super::InvocationParameters) -> String {
    use super::{HelpInvocation, InvocationParameters as P};
    match value {
        P::None | P::OptionalPath(None) | P::OptionalText(None) => "none".into(),
        P::Path(path) | P::OptionalPath(Some(path)) => path.display().to_string(),
        P::OptionalText(Some(text)) => text.clone(),
        P::SessionRename { workspace, name } => format!("{} {name}", workspace.display()),
        P::Grammar(value) => value
            .map(|kind| kind.name())
            .unwrap_or("current grammar")
            .into(),
        P::Help(HelpInvocation::ActiveView) => "current view".into(),
        P::Help(HelpInvocation::Manual(topic)) => topic.as_deref().unwrap_or("manual").into(),
    }
}

/// Copy only forms the current compiler can represent. The original accepted
/// value preserves configured argument strings, step order and authored prose.
pub fn binding_yaml(
    binding: &Binding,
    mode: Mode,
    config: Option<&Value>,
    map: &Keymap,
) -> Result<String, String> {
    if matches!(binding.target, BindingTarget::Plugin(_)) {
        return Err("Plugin bindings use plugins[].bindings".into());
    }
    if binding.program.has_conditions() {
        return Err("This built-in program uses conditional steps, which the configurable binding format does not yet support".into());
    }
    if binding
        .program
        .iter()
        .any(|step| step.target() == BindingTarget::Editor(super::EditorCommand::ModeEnter))
    {
        return Err("This built-in program uses internal mode steps; copying it requires the future mode-argument binding format".into());
    }
    let value = if binding.configured_actions {
        let path = binding
            .source_paths
            .iter()
            .find(|p| p.first().is_some_and(|p| p == "bind"))
            .ok_or("Configured binding source unavailable")?;
        path.iter()
            .try_fold(
                config.ok_or("Configuration source unavailable")?,
                |v, key| v.get(key).ok_or("Binding source unavailable"),
            )?
            .clone()
    } else {
        Value::String(binding.target.name().into())
    };
    let mut entries = Mapping::new();
    entries.insert(Value::String(binding.sequence.to_string()), value);
    let mut modes = Mapping::new();
    modes.insert(
        Value::String(actions::mode_name(mode).into()),
        Value::Mapping(entries),
    );
    let mut section = Mapping::new();
    if binding.scope == BindingScope::Global && mode != Mode::List {
        section.insert(Value::String("bind".into()), Value::Mapping(modes));
    } else {
        let mut scopes = Mapping::new();
        scopes.insert(
            Value::String(binding.scope.name().into()),
            Value::Mapping(modes),
        );
        section.insert(Value::String("scopes".into()), Value::Mapping(scopes));
    }
    let compiled = crate::keymap::configured::compile(&Value::Mapping(section.clone()), map);
    if !compiled.errors.is_empty() {
        return Err(format!(
            "This binding cannot be copied in the current configuration format: {}",
            compiled.errors.join("; ")
        ));
    }
    let mut document = Mapping::new();
    document.insert(Value::String("keys".into()), Value::Mapping(section));
    serde_yaml::to_string(&document).map_err(|e| e.to_string())
}

/// Uses the current scoped compiler's admission inventory, not a Cartesian
/// combination of descriptor views and global modes.
pub fn admitted_here(id: CommandId, mode: Mode, scope: BindingScope) -> bool {
    match id {
        CommandId::Editor(c) => {
            crate::keymap::admission::admits(scope, BindingTarget::Editor(c))
                && crate::keymap::admits_scoped_action(scope, mode, c.metadata().name).is_some()
        }
        CommandId::Colon(c) => c.action_policy().global_binding_modes.contains(&mode),
        CommandId::Plugin(_) => false,
    }
}

pub fn descriptor_reference() -> String {
    use std::fmt::Write;
    let mut out = String::from(
        "\n## Action contracts\n\nGenerated from built-in descriptors. Effects include owned continuations and specialized views; remaining prompt and background effects require the last step. Character operands and palette arguments are separate.\n\n| Action | Effects | Count | Character operands | Palette argument | View differences |\n| --- | --- | --- | --- | --- | --- |\n",
    );
    for id in identities() {
        let d = id.descriptor().unwrap();
        let views = d
            .view_overrides
            .iter()
            .map(|v| format!("{}: {}", v.scope.name(), v.description))
            .collect::<Vec<_>>()
            .join("; ");
        let argument = d.argument().map(|a| a.usage).unwrap_or("none");
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} |",
            d.metadata.name,
            display::effects(d.policy.effects),
            display::count(d.policy.count),
            display::operands(d.policy.operands),
            argument.replace('|', "&#124;"),
            views.replace('|', "&#124;")
        );
    }
    out
}

pub fn guide_reference() -> String {
    crate::keymap::actions::reference()
        .lines()
        .skip(1)
        .map(|line| {
            if line.starts_with('#') {
                format!("##{line}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect()
}
