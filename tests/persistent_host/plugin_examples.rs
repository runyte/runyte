// SPDX-License-Identifier: MPL-2.0

//! Every example under `docs/plugins/` started by a real persistent host.
//!
//! The Python conformance checks play the host themselves, so an example can
//! pass all of them and still be refused by the editor. This test configures
//! the examples in a real host and requires `:plugins` to report each one
//! running. It needs Python with Paramiko, Node.js and the built todo showcase,
//! so it is ignored by default and run by the plugin conformance CI job:
//!
//! ```sh
//! python3 docs/plugins/todo/build.py
//! RUNYTE_EXAMPLE_PYTHON=/path/to/venv/bin/python \
//!   cargo test --locked --test persistent_host plugin_examples -- --ignored
//! ```

use super::*;
use std::collections::BTreeSet;

/// Enabled plugin instances one workspace host accepts, per `docs/plugins.md`.
const HOST_PLUGIN_LIMIT: usize = 8;

/// A registered plugin that exits right after registration would still read
/// `Running` once. Looking again after this pause catches it.
const SETTLE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
enum Runtime {
    Python,
    Node,
    /// A binary written by `docs/plugins/todo/build.py`, below its output.
    Built,
}

struct Example {
    id: &'static str,
    runtime: Runtime,
    path: &'static str,
    capabilities: &'static [&'static str],
}

const fn python(
    id: &'static str,
    path: &'static str,
    capabilities: &'static [&'static str],
) -> Example {
    Example {
        id,
        runtime: Runtime::Python,
        path,
        capabilities,
    }
}

const EXAMPLES: &[Example] = &[
    Example {
        id: "case",
        runtime: Runtime::Python,
        path: "uppercase.py",
        capabilities: &["text", "selections"],
    },
    python("catalog", "catalog.py", &["views", "interaction"]),
    python("dashboard", "dashboard.py", &["views"]),
    python("documents", "documents.py", &["documents", "jobs"]),
    python(
        "files",
        "files.py",
        &["views", "filesystem", "documents", "interaction", "jobs"],
    ),
    python(
        "handoffs",
        "handoffs.py",
        &["notifications", "terminals", "external"],
    ),
    python(
        "helper",
        "helper.py",
        &["views", "interaction", "processes"],
    ),
    python("jobs", "jobs.py", &["jobs"]),
    python(
        "media",
        "media.py",
        &["views", "processes", "activity", "jobs"],
    ),
    python("memory", "memory.py", &["providers", "documents", "jobs"]),
    python(
        "preferences",
        "preferences.py",
        &["settings", "state", "views"],
    ),
    python("tasks", "tasks.py", &["views"]),
    python("validation", "validation.py", &["interaction"]),
    python(
        "ftp",
        "ftp.py",
        &[
            "views",
            "providers",
            "documents",
            "jobs",
            "filesystem",
            "interaction",
        ],
    ),
    python(
        "sftp",
        "sftp.py",
        &[
            "views",
            "providers",
            "documents",
            "jobs",
            "filesystem",
            "interaction",
        ],
    ),
    Example {
        id: "tasks-node",
        runtime: Runtime::Node,
        path: "tasks.mjs",
        capabilities: &["views"],
    },
    python("todo-python", "todo/python/todo.py", &["views"]),
    Example {
        id: "todo-rust",
        runtime: Runtime::Built,
        path: "rust/release/runyte-todo-example",
        capabilities: &["views"],
    },
    Example {
        id: "todo-c",
        runtime: Runtime::Built,
        path: "todo-c",
        capabilities: &["views"],
    },
];

/// Top-level example programs, recognised by how they register, so a new
/// example cannot be added without also being started here.
fn examples_on_disk(directory: &Path) -> BTreeSet<String> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_str().unwrap();
            (name.ends_with(".py") || name.ends_with(".mjs"))
                && !name.starts_with("check_")
                && name != "application.py"
        })
        .filter(|path| {
            let source = fs::read_to_string(path).unwrap();
            ["app.run()", "\"register\"", "'register'"]
                .iter()
                .any(|marker| source.contains(marker))
        })
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_owned())
        .collect()
}

/// An absolute program path from `variable`, or else from `PATH`. Plugin
/// configuration refuses relative executables.
fn program(variable: &str, name: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH")?)
                .map(|directory| directory.join(name))
                .find(|candidate| candidate.is_file())
        })
        .unwrap_or_else(|| panic!("{name} is not on PATH; set {variable}"));
    assert!(path.is_absolute(), "{variable} must be absolute: {path:?}");
    path
}

struct Programs {
    examples: PathBuf,
    python: PathBuf,
    node: PathBuf,
    showcase: PathBuf,
}

impl Programs {
    fn entry(&self, example: &Example, profiles: &Path) -> serde_json::Value {
        let script = self.examples.join(example.path);
        let (executable, mut args) = match example.runtime {
            Runtime::Python => (self.python.clone(), vec![script]),
            Runtime::Node => (self.node.clone(), vec![script]),
            Runtime::Built => {
                let binary = self.showcase.join(example.path);
                assert!(
                    binary.is_file(),
                    "{binary:?} is missing; run python3 docs/plugins/todo/build.py"
                );
                (binary, Vec::new())
            }
        };
        if matches!(example.id, "ftp" | "sftp") {
            args.push("--config".into());
            args.push(profiles.join(format!("{}.json", example.id)));
        }
        let mut entry = serde_json::json!({
            "id": example.id,
            "enabled": true,
            "executable": executable,
            "args": args,
        });
        entry["api"] = "runyte-1".into();
        entry["runyte"] = ">=0.3.0, <0.4.0".into();
        entry["capabilities"] = example.capabilities.into();
        entry
    }
}

/// Connection profiles that pass the remote examples' startup validation. They
/// point at a closed local port; registering never connects.
fn write_profiles(directory: &Path) {
    fs::create_dir_all(directory).unwrap();
    for file in ["password", "known_hosts", "identity"] {
        fs::write(directory.join(file), "").unwrap();
    }
    let path = |file: &str| directory.join(file).to_str().unwrap().to_owned();
    let ftp = serde_json::json!({
        "transport": "ftp", "alias": "Smoke", "host": "127.0.0.1", "port": 9,
        "username": "smoke", "root": "/", "password_file": path("password"),
    });
    let sftp = serde_json::json!({
        "alias": "Smoke", "host": "127.0.0.1", "port": 9, "username": "smoke",
        "root": "/", "known_hosts": path("known_hosts"),
        "identity_files": [path("identity")],
    });
    fs::write(directory.join("ftp.json"), ftp.to_string()).unwrap();
    fs::write(directory.join("sftp.json"), sftp.to_string()).unwrap();
}

/// The `:plugins` rows as (configured ID, phase) once no plugin is still
/// starting, with the list's selected row.
async fn settled_plugin_list(client: &mut LocalClient, count: usize) -> PluginList {
    let deadline = Instant::now() + EDITOR_STATE_TIMEOUT;
    let mut rows = Vec::new();
    loop {
        client.send(&ClientRequest::Resynchronize).await.unwrap();
        let frame = next_complete_frame(client).await;
        if let Some((selected, listed)) = plugin_list(&frame) {
            rows = listed;
            if rows.len() == count
                && rows
                    .iter()
                    .all(|(_, phase)| !matches!(phase.as_str(), "Starting" | "Restart pending"))
            {
                return (selected, rows);
            }
        }
        assert!(
            Instant::now() < deadline,
            "plugins were still starting after {EDITOR_STATE_TIMEOUT:?}: {rows:?}"
        );
        tokio::time::sleep(EDITOR_STATE_POLL).await;
    }
}

/// The `:plugins` list's selected row, and each row's configured ID and phase.
type PluginList = (Option<usize>, Vec<(String, String)>);

/// The `:plugins` list shown in `frame`, if it is open.
fn plugin_list(frame: &HostFrame) -> Option<PluginList> {
    let overlay = frame
        .overlays
        .iter()
        .find(|overlay| overlay.title == "Plugins")?;
    let rows = overlay
        .rows
        .iter()
        .map(|row| (row.label.clone(), row.detail.clone()))
        .collect();
    Some((overlay.selected, rows))
}

async fn register_in_one_host(programs: &Programs, batch: &[Example]) {
    let sandbox = TestSandbox::new();
    let root = project();
    let profiles = sandbox.runtime.join("profiles");
    write_profiles(&profiles);
    let plugins: Vec<_> = batch
        .iter()
        .map(|example| programs.entry(example, &profiles))
        .collect();
    let config = sandbox.runtime.join("plugin-examples.yaml");
    // JSON is valid YAML, and serde_json quotes every path correctly.
    fs::write(
        &config,
        serde_json::json!({ "lsp": { "enable": false }, "plugins": plugins }).to_string(),
    )
    .unwrap();
    let process = sandbox
        .bundled_runyte()
        .args(["--serve", "--config"])
        .arg(&config)
        .arg("note.txt")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", sandbox.runtime_dir())
        .env("XDG_CACHE_HOME", sandbox.cache_dir())
        .env("XDG_DATA_HOME", sandbox.runtime.join("data"))
        .env("XDG_STATE_HOME", sandbox.runtime.join("state"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(Some(process));
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(sandbox.runtime_dir()),
    )
    .unwrap();
    assert!(wait_for_endpoint(&mut child, &endpoint).await);
    let (mut client, _) = connect_interactive_when_available(&endpoint, geometry()).await;
    let frame = next_idle_frame(&mut client).await;
    let result = invoke_when_current(&mut client, "plugins", frame).await;
    assert!(
        matches!(result, HostResponse::CommandResult { .. }),
        "expected :plugins to run, got {result:?}"
    );

    let (selected, first) = settled_plugin_list(&mut client, batch.len()).await;
    assert_eq!(selected, Some(0), "the plugin list opens on its first row");
    tokio::time::sleep(SETTLE).await;
    // A frame read now can predate the pause: frames already on the socket are
    // still unread, and the host's replaceable frame slot is delivered behind
    // command replies, so neither Resynchronize nor a request/response pair
    // is a barrier. Moving the selection after the pause is: only a frame
    // prepared after that input shows the second row selected.
    let moved = send_input(&mut client, KeyStroke::plain(KeyCode::Down)).await;
    let current = wait_for_editor_frame(
        &mut client,
        moved,
        "the plugin list after the settling pause",
        |frame| plugin_list(frame).is_some_and(|(selected, _)| selected == Some(1)),
    )
    .await;
    let (_, settled) = plugin_list(&current).unwrap();
    let expected: Vec<_> = batch
        .iter()
        .map(|example| (example.id.to_owned(), "Running".to_owned()))
        .collect();
    assert_eq!(
        settled, expected,
        "every example should stay running after registration (first observed: {first:?})"
    );

    shutdown(&mut client, ClientRequest::Shutdown).await;
    let status = tokio::task::spawn_blocking(move || child.0.take().unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[ignore = "needs Python with Paramiko, Node.js and the built todo showcase; run by plugin conformance CI"]
async fn every_example_plugin_registers_and_stays_running_in_a_real_host() {
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"));
    let examples = checkout.join("docs/plugins");
    let listed: BTreeSet<_> = EXAMPLES
        .iter()
        .filter(|example| !matches!(example.runtime, Runtime::Built) && !example.path.contains('/'))
        .map(|example| example.path.to_owned())
        .collect();
    assert_eq!(
        listed,
        examples_on_disk(&examples),
        "EXAMPLES must start every example program in docs/plugins"
    );
    let programs = Programs {
        python: program("RUNYTE_EXAMPLE_PYTHON", "python3"),
        node: program("RUNYTE_EXAMPLE_NODE", "node"),
        showcase: std::env::var_os("RUNYTE_TODO_SHOWCASE")
            .map_or_else(|| checkout.join("target/todo-showcase"), PathBuf::from),
        examples,
    };
    for batch in EXAMPLES.chunks(HOST_PLUGIN_LIMIT) {
        register_in_one_host(&programs, batch).await;
    }
}
