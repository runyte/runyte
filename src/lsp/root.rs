// SPDX-License-Identifier: MPL-2.0

//! Where each language server is started.
//!
//! A workspace may hold more than one project for a language: a repository
//! with a Rust crate in one subdirectory and a Python package in another, or
//! two independent crates side by side. A server rooted at the workspace sees
//! neither project's manifest, so each document is given the root of the
//! project it belongs to, found from marker files such as `Cargo.toml`, and
//! one server runs per language and root.
//!
//! The search follows Helix's `find_lsp_workspace` (helix-lsp, MPL-2.0): walk
//! up from the document to the workspace root and keep the top-most directory
//! holding a marker, so a member of a Cargo workspace resolves to the Cargo
//! workspace rather than to its own crate. Unlike Helix, a document with no
//! marker, or one outside the workspace, is rooted at the workspace itself:
//! Runyte always has one, and that is where every server started before
//! roots existed. The default markers are taken from Helix's `languages.toml`
//! for the languages Runyte highlights, leaving out its glob patterns.
//!
//! A lookup costs one `metadata` call per marker for each directory between
//! the document and the workspace root, so the editor keeps the answer for
//! each directory and language; see `App::lsp_server_of`.

use std::path::{Path, PathBuf};

use crate::config::LanguageServerConfig;

/// One language server process: a language's configured command, started at
/// one root. Every root lies inside the workspace whose permission covers it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ServerId {
    pub language: String,
    pub root: PathBuf,
}

impl ServerId {
    pub fn new(language: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        Self {
            language: language.into(),
            root: root.into(),
        }
    }

    /// How the server is named to a reader: its language, followed by its
    /// root relative to `workspace` unless it is the workspace itself.
    pub fn label(&self, workspace: &Path) -> String {
        match self.root.strip_prefix(workspace) {
            Ok(relative) if relative.as_os_str().is_empty() => self.language.clone(),
            Ok(relative) => format!("{} ({})", self.language, relative.display()),
            Err(_) => format!("{} ({})", self.language, self.root.display()),
        }
    }
}

impl ServerId {
    /// Whether this server's root is `workspace` or a directory below it,
    /// judged from the spelling alone: a root that steps up with `..` is
    /// outside, whatever it resolves to.
    pub fn is_within(&self, workspace: &Path) -> bool {
        self.root.strip_prefix(workspace).is_ok_and(|relative| {
            relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
        })
    }
}

/// The marker file names that identify `language`'s project root when its
/// configuration names none.
pub fn default_markers(language: &str) -> &'static [&'static str] {
    match language {
        "rust" => &["Cargo.toml", "Cargo.lock"],
        "python" => &[
            "pyproject.toml",
            "setup.py",
            "poetry.lock",
            "pyrightconfig.json",
        ],
        "swift" => &["Package.swift"],
        "javascript" => &["package.json", "jsconfig.json"],
        "typescript" | "tsx" => &["package.json", "tsconfig.json"],
        "go" => &["go.work", "go.mod"],
        "java" => &["pom.xml", "build.gradle", "build.gradle.kts"],
        "kotlin" => &["settings.gradle", "settings.gradle.kts"],
        "lua" => &[
            ".luarc.json",
            ".luacheckrc",
            ".stylua.toml",
            "selene.toml",
            ".git",
        ],
        "zig" => &["build.zig"],
        "markdown" => &[".marksman.toml"],
        "dockerfile" => &["Dockerfile", "Containerfile"],
        "php" => &["composer.json", "index.php"],
        "elixir" => &["mix.exs", "mix.lock"],
        _ => &[],
    }
}

/// The root `language`'s server for the document at `path` starts in, spelled
/// below `workspace` as the caller spells it.
///
/// The document's directory is resolved on disk before it is compared with
/// the workspace, so a `..` or a symbolic link in a buffer's path can neither
/// carry a server root outside the workspace whose permission covers it nor
/// make a file inside it look like it is outside.
pub fn server_root(
    language: &str,
    settings: Option<&LanguageServerConfig>,
    path: &Path,
    workspace: &Path,
) -> PathBuf {
    let configured = settings.and_then(|settings| settings.roots.as_deref());
    let found = match configured {
        Some(markers) => top_marker(path, markers.iter().map(String::as_str), workspace),
        None => top_marker(path, default_markers(language).iter().copied(), workspace),
    };
    found.unwrap_or_else(|| workspace.to_path_buf())
}

/// The top-most directory from `path`'s parent up to and including
/// `workspace` that holds one of `markers`, or `None` when none does, `path`
/// is not inside `workspace`, or either cannot be resolved.
fn top_marker<'a>(
    path: &Path,
    markers: impl Iterator<Item = &'a str> + Clone,
    workspace: &Path,
) -> Option<PathBuf> {
    let directory = path.parent()?.canonicalize().ok()?;
    let resolved = workspace.canonicalize().ok()?;
    directory.strip_prefix(&resolved).ok()?;
    let mut top = None;
    for ancestor in directory.ancestors() {
        if markers.clone().any(|marker| {
            is_marker_name(marker) && ancestor.join(marker).symlink_metadata().is_ok()
        }) {
            top = Some(ancestor);
        }
        if ancestor == resolved {
            break;
        }
    }
    // Reported in the caller's spelling of the workspace, so a server root
    // and the workspace it belongs to compare as they are written.
    Some(workspace.join(top?.strip_prefix(&resolved).ok()?))
}

/// Markers are single file names. A configured entry that would leave the
/// directory it is tested in, such as `../Cargo.toml` or an absolute path,
/// is ignored rather than allowed to look outside the workspace.
fn is_marker_name(marker: &str) -> bool {
    let mut components = Path::new(marker).components();
    matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{ServerId, server_root};
    use crate::config::LanguageServerConfig;

    fn workspace() -> crate::test_support::TestRuntimeRoot {
        crate::test_support::TestRuntimeRoot::new("lsp-root").unwrap()
    }

    fn touch(path: &std::path::Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }

    #[test]
    fn a_document_without_a_marker_is_rooted_at_the_workspace() {
        let workspace = workspace();
        let root: &std::path::Path = &workspace;
        let file = root.join("scripts/tool.rs");
        touch(&file);

        assert_eq!(server_root("rust", None, &file, root), root);
    }

    #[test]
    fn the_top_most_marker_inside_the_workspace_wins() {
        let workspace = workspace();
        let root: &std::path::Path = &workspace;
        touch(&root.join("tools/Cargo.toml"));
        touch(&root.join("tools/crates/member/Cargo.toml"));
        let file = root.join("tools/crates/member/src/lib.rs");
        touch(&file);

        assert_eq!(server_root("rust", None, &file, root), root.join("tools"));
    }

    #[test]
    fn a_marker_above_the_workspace_is_not_seen() {
        let outer = workspace();
        touch(&outer.join("Cargo.toml"));
        let root = outer.join("project");
        let file = root.join("src/main.rs");
        touch(&file);

        assert_eq!(server_root("rust", None, &file, &root), root);
    }

    #[test]
    fn a_marker_at_the_workspace_root_roots_the_server_there() {
        let workspace = workspace();
        let root: &std::path::Path = &workspace;
        touch(&root.join("Cargo.toml"));
        touch(&root.join("crates/member/Cargo.toml"));
        let file = root.join("crates/member/src/lib.rs");
        touch(&file);

        assert_eq!(server_root("rust", None, &file, root), root);
    }

    #[test]
    fn a_document_outside_the_workspace_is_rooted_at_the_workspace() {
        let workspace = workspace();
        let elsewhere = self::workspace();
        touch(&elsewhere.join("Cargo.toml"));
        let file = elsewhere.join("main.rs");
        touch(&file);

        assert_eq!(
            server_root("rust", None, &file, &workspace),
            workspace.to_path_buf()
        );
    }

    #[test]
    fn configured_markers_replace_the_defaults() {
        let workspace = workspace();
        let root: &std::path::Path = &workspace;
        touch(&root.join("python/pyproject.toml"));
        touch(&root.join("python/app/.project"));
        let file = root.join("python/app/main.py");
        touch(&file);
        let configured = LanguageServerConfig {
            roots: Some(vec![".project".to_owned()]),
            ..LanguageServerConfig::default()
        };
        let none = LanguageServerConfig {
            roots: Some(Vec::new()),
            ..LanguageServerConfig::default()
        };

        assert_eq!(
            server_root("python", None, &file, root),
            root.join("python")
        );
        assert_eq!(
            server_root("python", Some(&configured), &file, root),
            root.join("python/app")
        );
        assert_eq!(server_root("python", Some(&none), &file, root), root);
    }

    #[test]
    fn a_marker_that_names_another_directory_is_ignored() {
        let workspace = workspace();
        let root: &std::path::Path = &workspace;
        touch(&root.join("Cargo.toml"));
        let file = root.join("app/src/main.rs");
        touch(&file);
        let configured = LanguageServerConfig {
            roots: Some(vec!["../Cargo.toml".to_owned(), "/".to_owned()]),
            ..LanguageServerConfig::default()
        };

        assert_eq!(
            server_root("rust", Some(&configured), &file, &root.join("app")),
            root.join("app")
        );
    }

    #[test]
    fn a_parent_component_cannot_carry_a_root_outside_the_workspace() {
        let outer = workspace();
        let root = outer.join("project");
        fs::create_dir_all(&root).unwrap();
        touch(&outer.join("sibling/Cargo.toml"));
        let file = root.join("../sibling/src/main.rs");
        touch(&file);

        assert_eq!(server_root("rust", None, &file, &root), root);
    }

    #[test]
    fn a_marker_reached_through_a_parent_component_inside_the_workspace_counts() {
        let workspace = workspace();
        let root: &std::path::Path = &workspace;
        touch(&root.join("tools/Cargo.toml"));
        let file = root.join("other/../tools/src/main.rs");
        fs::create_dir_all(root.join("other")).unwrap();
        touch(&file);

        assert_eq!(server_root("rust", None, &file, root), root.join("tools"));
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_into_the_workspace_finds_the_same_root() {
        let outer = workspace();
        let root = outer.join("project");
        touch(&root.join("tools/Cargo.toml"));
        let file = root.join("tools/src/main.rs");
        touch(&file);
        std::os::unix::fs::symlink(&root, outer.join("alias")).unwrap();

        assert_eq!(
            server_root("rust", None, &outer.join("alias/tools/src/main.rs"), &root),
            root.join("tools")
        );
    }

    #[test]
    fn a_marker_directory_counts_like_a_marker_file() {
        let workspace = workspace();
        let root: &std::path::Path = &workspace;
        fs::create_dir_all(root.join("game/.git")).unwrap();
        let file = root.join("game/init.lua");
        touch(&file);

        assert_eq!(server_root("lua", None, &file, root), root.join("game"));
    }

    #[test]
    fn a_root_that_steps_out_of_the_workspace_is_not_within_it() {
        let workspace = std::path::Path::new("/work");

        assert!(ServerId::new("rust", "/work").is_within(workspace));
        assert!(ServerId::new("rust", "/work/tools").is_within(workspace));
        assert!(!ServerId::new("rust", "/work/../elsewhere").is_within(workspace));
        assert!(!ServerId::new("rust", "/elsewhere").is_within(workspace));
    }

    #[test]
    fn a_label_names_the_root_only_when_it_is_not_the_workspace() {
        let workspace = std::path::Path::new("/work");

        assert_eq!(ServerId::new("rust", "/work").label(workspace), "rust");
        assert_eq!(
            ServerId::new("rust", "/work/tools").label(workspace),
            "rust (tools)"
        );
    }
}
