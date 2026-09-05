// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::{fs, os::unix::fs::symlink, time::Duration};

struct DiscoveryFixture {
    root: PathBuf,
    program: PathBuf,
}

impl DiscoveryFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "runyte-discovery-{}-{}",
            std::process::id(),
            crate::hash::sha256_hex(format!("{:?}", std::time::Instant::now()).as_bytes())
        ));
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::create_dir_all(root.join(".git/objects")).unwrap();
        fs::create_dir_all(root.join(".git/refs")).unwrap();
        let root = root.canonicalize().unwrap();
        Self {
            program: root.join("git-provider"),
            root,
        }
    }

    fn install(&self, behavior: &str) {
        symlink(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
            &self.program,
        )
        .unwrap();
        fs::write(self.program.with_extension("behavior"), behavior).unwrap();
    }
}

impl Drop for DiscoveryFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn discovery_distinguishes_failure_to_launch_from_a_git_exit_or_signal() {
    let fixture = DiscoveryFixture::new();
    let provider = GitCliProvider::new(&fixture.program);
    assert!(matches!(
        provider.discover(&fixture.root),
        Err(GitError::Unavailable { .. })
    ));
    fixture.install("printf 'cannot start Git\\n' >&2\nexit 128\n");
    assert!(matches!(provider.discover(&fixture.root),
        Err(GitError::Failed { code: Some(128), signal: None, stderr, .. })
            if stderr.contains("cannot start Git")));
    fs::write(
        fixture.program.with_extension("behavior"),
        "kill -TERM $$\n",
    )
    .unwrap();
    assert!(matches!(
        provider.discover(&fixture.root),
        Err(GitError::Failed {
            code: None,
            signal: Some(15),
            ..
        })
    ));
    // The same provider can recover after its launch environment is repaired.
    fs::write(
        fixture.program.with_extension("behavior"),
        "case \"$3\" in\n--show-toplevel) pwd ;;\n*) printf '.git\\n' ;;\nesac\n",
    )
    .unwrap();
    assert_eq!(
        provider.discover(&fixture.root).unwrap().unwrap().workdir(),
        fixture.root
    );
}

#[test]
fn every_repository_discovery_read_has_a_deadline() {
    for argument in ["--show-toplevel", "--git-dir", "--git-common-dir"] {
        let fixture = DiscoveryFixture::new();
        fixture.install(&format!(
            "if [ \"$3\" = '{argument}' ]; then sleep 1; fi\n\
             case \"$3\" in\n--show-toplevel) pwd ;;\n*) printf '.git\\n' ;;\nesac\n"
        ));
        let provider = GitCliProvider::new(&fixture.program)
            .with_local_read_timeout(Duration::from_millis(200));
        let error = provider.discover(&fixture.root).unwrap_err();
        assert!(
            matches!(&error, GitError::TimedOut { command, .. } if command.contains(argument)),
            "{error:?}"
        );
    }
}
