// SPDX-License-Identifier: MPL-2.0

//! Cancellable background traversal for retained workspace-search results.
//!
//! The editor owns the prompt and the generated result buffer. This module
//! owns only the work that may wait on the filesystem or scan substantial
//! text, returning one bounded, request-identified result to the host thread.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use anyhow::Result;
use regex::Regex;
use tokio::sync::mpsc::{self, Receiver, Sender};

use crate::text::Text;

pub(crate) const GLOBAL_SEARCH_FILE_LIMIT: u64 = 4 * 1024 * 1024;
pub(crate) const GLOBAL_SEARCH_RESULT_LIMIT: usize = 10_000;
const EVENT_CAPACITY: usize = 4;

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceSearchSnapshot {
    pub path: PathBuf,
    pub text: Text,
}

#[derive(Debug)]
pub(crate) struct WorkspaceSearchRequest {
    pub id: u64,
    pub root: PathBuf,
    pub matcher: Regex,
    pub show_hidden: bool,
    pub open_buffers: Vec<WorkspaceSearchSnapshot>,
    /// Whether the walk is bounded by [`crate::scan_boundary`], as a search
    /// with no workspace behind it is. Unreadable directories below the root
    /// are then passed over rather than failing the search, because such a
    /// search starts in system directories the reader only partly owns.
    pub contained: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceMatch {
    pub(crate) path: PathBuf,
    pub(crate) row: usize,
    pub(crate) column: usize,
    pub(crate) length: usize,
    pub(crate) preview: String,
}

#[derive(Debug)]
pub enum WorkspaceSearchEvent {
    Completed {
        id: u64,
        matches: Vec<WorkspaceMatch>,
        limited: bool,
    },
    Failed {
        id: u64,
        message: String,
    },
}

enum ServiceBackend {
    Threaded(Sender<WorkspaceSearchEvent>),
    #[cfg(test)]
    Controlled(std::sync::mpsc::Sender<WorkspaceSearchRequest>),
}

pub struct WorkspaceSearchService {
    active: Arc<AtomicU64>,
    backend: ServiceBackend,
}

impl WorkspaceSearchService {
    pub(crate) fn search(&self, request: WorkspaceSearchRequest) -> Result<(), String> {
        self.active.store(request.id, Ordering::Release);
        match &self.backend {
            ServiceBackend::Threaded(events) => {
                let id = request.id;
                let active = Arc::clone(&self.active);
                let events = events.clone();
                thread::Builder::new()
                    .name("runyte-workspace-search".to_owned())
                    .spawn(move || {
                        let result = perform(request, || active.load(Ordering::Acquire) != id);
                        if active.load(Ordering::Acquire) != id {
                            return;
                        }
                        let event = match result {
                            Ok(Some((matches, limited))) => WorkspaceSearchEvent::Completed {
                                id,
                                matches,
                                limited,
                            },
                            Ok(None) => return,
                            Err(error) => WorkspaceSearchEvent::Failed {
                                id,
                                message: error.to_string(),
                            },
                        };
                        let _ = events.blocking_send(event);
                    })
                    .map(|_| ())
                    .map_err(|error| format!("failed to start workspace search: {error}"))
            }
            #[cfg(test)]
            ServiceBackend::Controlled(requests) => requests
                .send(request)
                .map_err(|_| "workspace search test service stopped".to_owned()),
        }
    }

    #[cfg(test)]
    pub(crate) fn controlled() -> (Self, std::sync::mpsc::Receiver<WorkspaceSearchRequest>) {
        let (requests, receiver) = std::sync::mpsc::channel();
        (
            Self {
                active: Arc::new(AtomicU64::new(0)),
                backend: ServiceBackend::Controlled(requests),
            },
            receiver,
        )
    }
}

impl Drop for WorkspaceSearchService {
    fn drop(&mut self) {
        self.active.store(0, Ordering::Release);
    }
}

pub fn spawn() -> (WorkspaceSearchService, Receiver<WorkspaceSearchEvent>) {
    let (events, receiver) = mpsc::channel(EVENT_CAPACITY);
    (
        WorkspaceSearchService {
            active: Arc::new(AtomicU64::new(0)),
            backend: ServiceBackend::Threaded(events),
        },
        receiver,
    )
}

pub(crate) fn perform(
    request: WorkspaceSearchRequest,
    cancelled: impl Fn() -> bool,
) -> Result<Option<(Vec<WorkspaceMatch>, bool)>> {
    let open_paths = request
        .open_buffers
        .iter()
        .map(|snapshot| snapshot.path.as_path())
        .collect::<HashSet<_>>();
    let Some((mut matches, mut limited)) = workspace_matches(
        &request.root,
        &request.matcher,
        request.show_hidden,
        request.contained,
        &open_paths,
        &cancelled,
    )?
    else {
        return Ok(None);
    };

    // Search the immutable rope snapshots on the worker too. Cloning a Text
    // shares Ropey's chunks, so taking these snapshots on the host does not
    // copy each complete open document or hand mutable editor state away.
    for snapshot in request.open_buffers {
        if cancelled() {
            return Ok(None);
        }
        let Some((live, live_limited)) =
            matches_in_rope(&snapshot.path, &snapshot.text, &request.matcher, &cancelled)
        else {
            return Ok(None);
        };
        limited |= live_limited;
        matches.extend(live);
        matches.sort_by(|left, right| {
            (&left.path, left.row, left.column).cmp(&(&right.path, right.row, right.column))
        });
        limited |= matches.len() > GLOBAL_SEARCH_RESULT_LIMIT;
        matches.truncate(GLOBAL_SEARCH_RESULT_LIMIT);
    }
    matches.sort_by(|left, right| {
        (&left.path, left.row, left.column).cmp(&(&right.path, right.row, right.column))
    });
    Ok(Some((matches, limited)))
}

fn workspace_matches(
    root: &Path,
    matcher: &Regex,
    show_hidden: bool,
    contained: bool,
    open_paths: &HashSet<&Path>,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<(Vec<WorkspaceMatch>, bool)>> {
    let boundary = if contained {
        let root = root.canonicalize()?;
        if let Some(reason) = crate::scan_boundary::refusal(&root) {
            anyhow::bail!(reason);
        }
        Some(crate::scan_boundary::Boundary::of(&root)?)
    } else {
        None
    };
    let mut visited = 0usize;
    let mut pending = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    while let Some(directory) = pending.pop() {
        if cancelled() {
            return Ok(None);
        }
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) if contained && directory != root => continue,
            Err(error) => return Err(error.into()),
        };
        // A contained walk spends its budget as entries are read, hidden ones
        // included, so a huge directory is never collected or sorted whole.
        let mut truncated = false;
        let mut collected = Vec::new();
        for entry in entries {
            if contained {
                if visited == crate::scan_boundary::CONTAINED_SCAN_ENTRY_LIMIT {
                    truncated = true;
                    break;
                }
                visited += 1;
            }
            match entry {
                Ok(entry) => collected.push(entry),
                Err(_) if contained => {}
                Err(error) => return Err(error.into()),
            }
        }
        let mut entries = collected;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries.into_iter().rev() {
            if cancelled() {
                return Ok(None);
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) if contained => continue,
                Err(error) => return Err(error.into()),
            };
            if file_type.is_symlink() {
                continue;
            }
            // A contained walk needs every entry's device for its boundary.
            // An uncontained one reads metadata only for the files that pass
            // the cheaper checks below, as it always has.
            let metadata = if contained {
                match entry.metadata() {
                    Ok(metadata) => Some(metadata),
                    Err(_) => continue,
                }
            } else {
                None
            };
            if let Some(boundary) = boundary.as_ref()
                && file_type.is_dir()
                && !metadata.as_ref().is_some_and(|meta| boundary.admits(meta))
            {
                continue;
            }
            if file_type.is_dir() {
                if matches!(name.as_ref(), ".git" | ".runyte" | "target")
                    || (!show_hidden && name.starts_with('.'))
                {
                    continue;
                }
                pending.push(entry.path());
                continue;
            }
            if !file_type.is_file() || (!show_hidden && name.starts_with('.')) {
                continue;
            }
            let length = match metadata {
                Some(metadata) => metadata.len(),
                None => entry.metadata()?.len(),
            };
            if length > GLOBAL_SEARCH_FILE_LIMIT {
                continue;
            }
            let path = entry.path();
            // The captured rope is authoritative. Its stale disk contents
            // must not consume the result budget and hide unopened files.
            if open_paths.contains(path.as_path()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if !extend_matches(
                &mut matches,
                &path,
                text.lines().enumerate(),
                matcher,
                cancelled,
            ) {
                return Ok(None);
            }
            if matches.len() >= GLOBAL_SEARCH_RESULT_LIMIT {
                matches.truncate(GLOBAL_SEARCH_RESULT_LIMIT);
                return Ok(Some((matches, true)));
            }
        }
        if truncated {
            return Ok(Some((matches, true)));
        }
    }
    Ok(Some((matches, false)))
}

#[cfg(test)]
#[path = "workspace_search/tests/mod.rs"]
mod regression_tests;

fn matches_in_rope(
    path: &Path,
    text: &Text,
    matcher: &Regex,
    cancelled: &impl Fn() -> bool,
) -> Option<(Vec<WorkspaceMatch>, bool)> {
    let rows = if text.len_chars() == 0 {
        0
    } else if text.char_at(text.len_chars() - 1) == Some('\n') {
        text.len_lines().saturating_sub(1)
    } else {
        text.len_lines()
    };
    let mut matches = Vec::new();
    let completed = extend_matches(
        &mut matches,
        path,
        (0..rows).map(|row| (row, text.line_string(row))),
        matcher,
        cancelled,
    );
    completed.then(|| {
        let limited = matches.len() > GLOBAL_SEARCH_RESULT_LIMIT;
        matches.truncate(GLOBAL_SEARCH_RESULT_LIMIT);
        (matches, limited)
    })
}

fn extend_matches(
    matches: &mut Vec<WorkspaceMatch>,
    path: &Path,
    lines: impl Iterator<Item = (usize, impl AsRef<str>)>,
    matcher: &Regex,
    cancelled: &impl Fn() -> bool,
) -> bool {
    for (row, line) in lines {
        if cancelled() {
            return false;
        }
        let line = line.as_ref();
        let mut previous_byte = 0;
        let mut previous_column = 0;
        let mut preview = None;
        for found in matcher.find_iter(line) {
            if cancelled() {
                return false;
            }
            // Matches never overlap. Count each prefix fragment once rather
            // than scanning from column zero again for every result.
            let column = previous_column + line[previous_byte..found.start()].chars().count();
            let length = found.as_str().chars().count();
            previous_byte = found.end();
            previous_column = column + length;
            let preview =
                preview.get_or_insert_with(|| line.trim().chars().take(240).collect::<String>());
            matches.push(WorkspaceMatch {
                path: path.to_path_buf(),
                row,
                column,
                length,
                preview: preview.clone(),
            });
            if matches.len() > GLOBAL_SEARCH_RESULT_LIMIT {
                return true;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "runyte-workspace-search-{}-{}-{label}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn rope_matching_has_string_lines_semantics() {
        let matcher = Regex::new("^$").unwrap();
        let snapshot = Text::from_str("full\n\n");

        let (matches, limited) =
            matches_in_rope(Path::new("file"), &snapshot, &matcher, &|| false).unwrap();

        assert_eq!(
            matches.iter().map(|found| found.row).collect::<Vec<_>>(),
            [1]
        );
        assert!(!limited);
    }

    #[test]
    fn cancellation_produces_no_result_or_failure() {
        let request = WorkspaceSearchRequest {
            id: 1,
            root: PathBuf::from("not-read-when-cancelled"),
            matcher: Regex::new("needle").unwrap(),
            show_hidden: false,
            open_buffers: Vec::new(),
            contained: false,
        };

        assert!(perform(request, || true).unwrap().is_none());
    }

    #[test]
    fn traversal_failures_remain_failures() {
        let request = WorkspaceSearchRequest {
            id: 1,
            root: PathBuf::from("missing-workspace-search-root"),
            matcher: Regex::new("needle").unwrap(),
            show_hidden: false,
            open_buffers: Vec::new(),
            contained: false,
        };

        assert!(perform(request, || false).is_err());
    }

    #[test]
    fn threaded_service_returns_a_bounded_identified_event() {
        let root = temporary("threaded");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("file.txt"), "needle\n").unwrap();
        let (service, mut events) = spawn();
        service
            .search(WorkspaceSearchRequest {
                id: 7,
                root: root.clone(),
                matcher: Regex::new("needle").unwrap(),
                show_hidden: false,
                open_buffers: Vec::new(),
                contained: false,
            })
            .unwrap();

        let WorkspaceSearchEvent::Completed {
            id,
            matches,
            limited,
        } = events.blocking_recv().unwrap()
        else {
            panic!("a valid scan completes");
        };
        assert_eq!(id, 7);
        assert_eq!(matches.len(), 1);
        assert!(!limited);
        std::fs::remove_dir_all(root).unwrap();
    }
}
