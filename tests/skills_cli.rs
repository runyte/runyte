// SPDX-License-Identifier: MPL-2.0

#![cfg(any(unix, windows))]

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("runyte-skills-cli-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        fs::create_dir_all(path.join("project/.runyte")).unwrap();
        fs::create_dir_all(path.join("project/nested dir")).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_runyte"))
            .args(args)
            .current_dir(self.0.join("project/nested dir"))
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn skills_cli_discovers_project_and_reports_partial_conflicts_without_an_editor() {
    let f = Fixture::new();
    let help = f.run(&["skills", "--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--project"));
    assert!(!f.run(&["skills", "install", "--all"]).status.success());
    let installed = f.run(&["skills", "install", "--all", "--project"]);
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    for agent in [".claude", ".agents"] {
        assert_eq!(
            fs::read(
                f.0.join("project")
                    .join(agent)
                    .join("skills/runyte-mcp/SKILL.md")
            )
            .unwrap(),
            include_bytes!("../skills/runyte-mcp/SKILL.md")
        );
    }
    assert!(!f.0.join("project/nested dir/.agents").exists());
    let path = f.0.join("project/.claude/skills/runyte-mcp/SKILL.md");
    fs::write(&path, "user edits").unwrap();
    let update = f.run(&["skills", "update", "--all", "--project"]);
    assert!(!update.status.success());
    let output = String::from_utf8_lossy(&update.stdout);
    assert!(
        output.contains("Claude Code:") && output.contains("Codex:") && output.contains("current")
    );
    assert_eq!(fs::read_to_string(path).unwrap(), "user edits");
    fs::remove_dir(f.0.join("project/.runyte")).unwrap();
    assert!(
        !f.run(&["skills", "status", "--all", "--project"])
            .status
            .success()
    );
}
