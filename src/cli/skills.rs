// SPDX-License-Identifier: MPL-2.0

//! Offline installation of the bundled MCP skill. No editor or agent is started.

mod locking;
#[cfg(test)]
mod tests;

use crate::{hash::sha256_hex, private_storage::Directory};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    io::{self, Read},
    path::Path,
};

const SKILL: &[u8] = include_bytes!("../../skills/runyte-mcp/SKILL.md");
const VERSION: u32 = 1;
const PAYLOAD: &str = "SKILL.md";
const METADATA: &str = ".runyte-skill.json";
const HELP: &str = "Usage: runyte skills <install|update|status> <--claude|--codex|--all> <--user|--project>\n\nManage the bundled runyte-mcp skill offline. Scope is required.\n--user uses your operating-system account home; --project uses the current\nGit or Runyte project root. Modified files are preserved. Use update for\nolder managed installations. No agent configuration or grants are changed.";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Install,
    Update,
    Status,
}
#[derive(Debug, PartialEq)]
struct Options {
    action: Action,
    user: bool,
    claude: bool,
    codex: bool,
}

fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Option<Options>> {
    let args: Vec<_> = args.into_iter().collect();
    if args == ["--help"] || args == ["-h"] {
        return Ok(None);
    }
    let action = match args.first().and_then(|a| a.to_str()) {
        Some("install") => Action::Install,
        Some("update") => Action::Update,
        Some("status") => Action::Status,
        _ => bail!("{HELP}"),
    };
    if args.len() == 2 && matches!(args[1].to_str(), Some("--help" | "-h")) {
        return Ok(None);
    }
    let (mut scope, mut claude, mut codex) = (None, false, false);
    for arg in &args[1..] {
        match arg.to_str() {
            Some("--user" | "--project") => {
                ensure!(
                    scope.is_none(),
                    "choose exactly one scope: --user or --project"
                );
                scope = Some(arg == "--user");
            }
            Some("--claude") => claude = true,
            Some("--codex") => codex = true,
            Some("--all") => {
                claude = true;
                codex = true;
            }
            _ => bail!("unknown skills argument: {}\n{HELP}", arg.to_string_lossy()),
        }
    }
    ensure!(claude || codex, "choose --claude, --codex or --all");
    Ok(Some(Options {
        action,
        user: scope.context("choose --user or --project")?,
        claude,
        codex,
    }))
}

pub(super) fn run(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let Some(options) = parse(args)? else {
        println!("{HELP}");
        return Ok(());
    };
    let root = if options.user {
        crate::user_paths::system_home_directory()
            .context("cannot resolve account home directory")?
    } else {
        crate::project_root::discover(&std::env::current_dir()?, Path::new(".runyte"))?.context(
            "no project root found; run inside a Git repository or Runyte-initialized directory",
        )?
    };
    execute(&options, &root, &mut io::stdout())
}

fn execute(options: &Options, root: &Path, output: &mut impl io::Write) -> Result<()> {
    let mut failed = false;
    for (selected, agent, subdir) in [
        (options.claude, "Claude Code", ".claude"),
        (options.codex, "Codex", ".agents"),
    ] {
        if !selected {
            continue;
        }
        let destination = root.join(subdir).join("skills/runyte-mcp");
        match manage(root, &destination, options.action) {
            Ok(status) => writeln!(output, "{agent}: {}: {status}", destination.display())?,
            Err(error) => {
                failed = true;
                writeln!(output, "{agent}: {}: {error:#}", destination.display())?;
            }
        }
    }
    ensure!(
        !failed,
        "one or more skill targets failed; existing conflicting files were preserved"
    );
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Release {
    skill_version: u32,
    runyte_version: String,
    sha256: String,
}
impl Release {
    fn bundled() -> Self {
        Self {
            skill_version: VERSION,
            runyte_version: env!("CARGO_PKG_VERSION").into(),
            sha256: sha256_hex(SKILL),
        }
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.skill_version > 0
                && !self.runyte_version.is_empty()
                && self.sha256.len() == 64
                && self.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid skill metadata"
        );
        Ok(())
    }
}

// The pending release is written before the payload. Either old or new bytes
// remain recognizable after a crash; any other bytes are a user modification.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    schema: u32,
    installed: Option<Release>,
    pending: Option<Release>,
}

fn read_optional(directory: &Directory, name: &str) -> Result<Option<Vec<u8>>> {
    match directory.open_owned_read(OsStr::new(name)) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 1024 * 1024, "skill file exceeds 1 MiB");
            Ok(Some(bytes))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| {
            format!("cannot read {name}; expected an owned regular file, not a link")
        }),
    }
}

fn read_metadata(bytes: Option<&[u8]>) -> Result<Option<Metadata>> {
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let metadata: Metadata = serde_json::from_slice(bytes)
        .context("invalid skill metadata; preserve files and resolve manually")?;
    ensure!(
        metadata.schema == 1,
        "unsupported skill metadata schema {}",
        metadata.schema
    );
    ensure!(
        metadata.installed.is_some() || metadata.pending.is_some(),
        "empty skill metadata"
    );
    for release in metadata.installed.iter().chain(metadata.pending.iter()) {
        release.validate()?;
    }
    Ok(Some(metadata))
}

fn manage(root: &Path, destination: &Path, action: Action) -> Result<String> {
    let directory = match Directory::open_existing(destination, false) {
        Ok(directory) => directory,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if action == Action::Status {
                return Ok("not installed".into());
            }
            if action == Action::Update {
                bail!("not installed; use skills install");
            }
            Directory::open(destination, false)?
        }
        Err(e) => return Err(e).context("cannot open skill destination; links are not supported"),
    };
    if action != Action::Status {
        // Commit only the subtree below the independently selected project/home
        // root, without flushing drive roots or changing shared agent-dir ACLs.
        let relative = destination.strip_prefix(root)?;
        let mut ancestor = root.to_path_buf();
        Directory::open_existing(&ancestor, false)?.sync()?;
        for component in relative.components() {
            ensure!(
                matches!(component, std::path::Component::Normal(_)),
                "invalid skill destination"
            );
            ancestor.push(component);
            Directory::open_existing(&ancestor, false)?.sync()?;
        }
    }
    // A persistent lock inode prevents overlapping installers. Kernel locks are
    // released even if the process crashes; never delete or replace this file.
    let _lock = if action != Action::Status {
        Some(locking::acquire(&directory)?)
    } else {
        None
    };
    let before = read_optional(&directory, PAYLOAD)?;
    let before_metadata = read_optional(&directory, METADATA)?;
    let metadata = read_metadata(before_metadata.as_deref())?;
    let desired = Release::bundled();
    let digest = before.as_deref().map(sha256_hex);
    let mut installed = None;
    if let Some(metadata) = &metadata {
        ensure!(
            !metadata
                .installed
                .iter()
                .chain(metadata.pending.iter())
                .any(|r| r.skill_version > VERSION),
            "newer skill version recorded; use a newer Runyte (downgrade refused)"
        );
        installed = if metadata
            .pending
            .as_ref()
            .is_some_and(|p| Some(&p.sha256) == digest.as_ref())
        {
            metadata.pending.clone()
        } else {
            ensure!(
                metadata.installed.as_ref().map(|r| &r.sha256) == digest.as_ref(),
                "skill modified or missing; preserve it and resolve manually"
            );
            metadata.installed.clone()
        };
    } else {
        ensure!(
            before.is_none() || before.as_deref() == Some(SKILL),
            "unmanaged skill differs from bundled content; preserve it and resolve manually"
        );
    }
    if action == Action::Status {
        return Ok(if metadata.as_ref().is_some_and(|m| m.pending.is_some()) {
            "interrupted update; run install/update to recover".into()
        } else if before.as_deref() == Some(SKILL) {
            format!(
                "current (skill {VERSION}){}",
                if metadata.is_none() {
                    "; unmanaged identical content"
                } else {
                    ""
                }
            )
        } else if installed.is_some() {
            "update available".into()
        } else {
            "not installed".into()
        });
    }
    if before.as_deref() != Some(SKILL) && installed.is_some() && action == Action::Install {
        bail!("older managed skill installed; use skills update");
    }
    if before.is_none() && metadata.is_none() && action == Action::Update {
        bail!("not installed; use skills install");
    }
    if before.as_deref() == Some(SKILL)
        && metadata
            .as_ref()
            .is_some_and(|m| m.pending.is_none() && m.installed.as_ref() == Some(&desired))
    {
        return Ok(format!("current (skill {VERSION})"));
    }
    ensure!(
        read_optional(&directory, PAYLOAD)? == before
            && read_optional(&directory, METADATA)? == before_metadata,
        "skill changed during installation; retry after reviewing it"
    );
    let journal = Metadata {
        schema: 1,
        installed,
        pending: Some(desired.clone()),
    };
    directory.atomic_write(OsStr::new(METADATA), &serde_json::to_vec_pretty(&journal)?)?;
    // Recheck after journaling too: a concurrent editor may have saved the skill.
    ensure!(
        read_optional(&directory, PAYLOAD)? == before,
        "skill changed during installation; files preserved for manual review"
    );
    if before.as_deref() != Some(SKILL) {
        directory.atomic_write(OsStr::new(PAYLOAD), SKILL)?;
    }
    let final_metadata = Metadata {
        schema: 1,
        installed: Some(desired),
        pending: None,
    };
    directory.atomic_write(
        OsStr::new(METADATA),
        &serde_json::to_vec_pretty(&final_metadata)?,
    )?;
    Ok(format!(
        "{} (skill {VERSION})",
        if before.is_none() {
            "installed"
        } else {
            "updated"
        }
    ))
}
