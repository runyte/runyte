// SPDX-License-Identifier: MPL-2.0
//! Opt-in state witnesses for isolated native-window acceptance, never release builds.
//! No buffer text, selections, terminal output, or preview source is recorded.
use super::*;

#[cfg(unix)]
fn snapshot(app: &App, frame: u64, attachment: u64) -> serde_json::Value {
    let mut panes = app.panes.iter().collect::<Vec<_>>();
    panes.sort_by_key(|(id, _)| **id);
    let panes = panes
        .into_iter()
        .take(64)
        .map(|(id, pane)| {
            serde_json::json!({
                "id": id,
                "buffer": pane.buffer,
                "path": app.buffers.get(pane.buffer).and_then(|buffer| buffer.path.as_ref()),
                "terminal": pane.terminal.is_some(),
                "preview": app.document_previews.get(id).map(|preview| serde_json::json!({
                    "generation": preview.generation,
                    "source": preview.source,
                    "language": preview.language,
                })),
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "role": "host",
        "pid": std::process::id(),
        "attachment": attachment,
        "frame": frame,
        "painted": false,
        "workspace": app.project_root,
        "mode": format!("{:?}", app.mode),
        "active_pane": app.active_pane,
        "panes": panes,
        "status_error": app.status_error,
        "status_present": !app.status.is_empty(),
    })
}

/// Existing explicit paths, including non-UTF-8 paths, retain their identity.
pub(super) fn trace_path(path: std::ffi::OsString, pid: u32) -> PathBuf {
    match path.to_str() {
        Some(text) if text.contains("{pid}") => {
            PathBuf::from(text.replace("{pid}", &pid.to_string()))
        }
        _ => PathBuf::from(path),
    }
}

#[cfg(unix)]
fn write_snapshot(directory: &Path, value: &serde_json::Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    anyhow::ensure!(bytes.len() <= 64 * 1024, "native test state exceeds 64 KiB");
    let name = format!("host-{}", std::process::id());
    let temporary = directory.join(format!("{name}.tmp"));
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, directory.join(format!("{name}.json")))?;
    Ok(())
}

#[cfg(unix)]
fn acknowledge_in(directory: &Path, frame: u64, attachment: u64) -> Result<()> {
    use std::io::Read;
    let path = directory.join(format!("host-{}.json", std::process::id()));
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 64 * 1024, "native test state exceeds 64 KiB");
    let mut state: serde_json::Value = serde_json::from_slice(&bytes)?;
    if state["frame"] == frame && state["attachment"] == attachment {
        state["painted"] = true.into();
        write_snapshot(directory, &state)?;
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn acknowledge(frame: u64, attachment: u64) {
    let Some(directory) = std::env::var_os("RUNYTE_NATIVE_TEST_STATE") else {
        return;
    };
    if let Err(error) = acknowledge_in(Path::new(&directory), frame, attachment) {
        eprintln!("native test paint state unavailable: {error}");
    }
}

#[cfg(unix)]
pub(super) fn record(app: &App, frame: Option<u64>, attachment: Option<u64>) {
    let Some(directory) = std::env::var_os("RUNYTE_NATIVE_TEST_STATE") else {
        return;
    };
    let (Some(frame), Some(attachment)) = (frame, attachment) else {
        return;
    };
    // Diagnostics must never change input admission or frame publication.
    if let Err(error) = write_snapshot(Path::new(&directory), &snapshot(app, frame, attachment)) {
        eprintln!("native test state unavailable: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_template_preserves_explicit_paths() {
        assert_eq!(
            trace_path("input-{pid}.log".into(), 42),
            PathBuf::from("input-42.log")
        );
        assert_eq!(
            trace_path("input.log".into(), 42),
            PathBuf::from("input.log")
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let path = std::ffi::OsString::from_vec(b"input-\xff.log".to_vec());
            assert_eq!(trace_path(path.clone(), 42), PathBuf::from(path));
        }
    }

    #[cfg(unix)]
    #[test]
    fn snapshots_exclude_contents_and_replace_complete_json() {
        let root = crate::test_support::TestRuntimeRoot::new("native-state").unwrap();
        let mut app = App::new(crate::config::Config::default(), None).unwrap();
        app.status = "search: SECRET_FROM_SELECTION".into();
        app.document_previews.insert(
            app.active_pane,
            crate::document_preview::DocumentPreview {
                generation: 7,
                source: app.active().buffer,
                text: "SECRET_PREVIEW".into(),
                selection_only: false,
                language: "html".into(),
                path: None,
            },
        );
        let value = snapshot(&app, 5, 2);
        assert!(!value.to_string().contains("SECRET"));
        assert_eq!(value["frame"], 5);
        assert_eq!(value["attachment"], 2);
        assert_eq!(value["panes"][0]["preview"]["generation"], 7);
        write_snapshot(root.path(), &value).unwrap();
        let next = snapshot(&app, 6, 2);
        write_snapshot(root.path(), &next).unwrap();
        let path = root
            .path()
            .join(format!("host-{}.json", std::process::id()));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(path).unwrap()).unwrap(),
            next
        );
        assert!(
            !root
                .path()
                .join(format!("host-{}.tmp", std::process::id()))
                .exists()
        );
        acknowledge_in(root.path(), 5, 2).unwrap();
        acknowledge_in(root.path(), 6, 1).unwrap();
        let path = root
            .path()
            .join(format!("host-{}.json", std::process::id()));
        let read =
            || serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(read()["painted"], false);
        acknowledge_in(root.path(), 6, 2).unwrap();
        assert_eq!(read()["painted"], true);
        assert!(write_snapshot(root.path(), &serde_json::json!("x".repeat(65536))).is_err());
    }
}
