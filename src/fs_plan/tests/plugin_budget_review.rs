// SPDX-License-Identifier: MPL-2.0

use super::*;

#[cfg(unix)]
#[test]
fn captured_symlink_targets_consume_the_metadata_budget() {
    let root = TempDir::new();
    // Stay below macOS's own symlink-target ceiling so this exercises
    // Runyte's metadata budget on every supported Unix platform.
    let target = "x".repeat(900);
    std::os::unix::fs::symlink(&target, root.join("link")).unwrap();
    let mut limits = OperationLimits {
        entries: 2,
        depth: 1,
        bytes: 0,
        metadata_bytes: 512 * 2 + "link".len() + target.len(),
    };
    let captured = SourceFingerprint::capture_limited(&root.0, Some(limits)).unwrap();
    assert_eq!(captured.descendants.len(), 1);
    assert_eq!(
        captured.descendants[0].1.symlink_target.as_deref(),
        Some(Path::new(&target))
    );
    limits.metadata_bytes -= 1;
    assert!(
        SourceFingerprint::capture_limited(&root.0, Some(limits))
            .unwrap_err()
            .is::<OperationLimitExceeded>()
    );
}

#[test]
fn deep_wide_capture_stops_when_the_shared_entry_budget_is_consumed() {
    let root = TempDir::new();
    let mut current = root.0.clone();
    for _ in 0..4 {
        fs::create_dir(current.join("nested")).unwrap();
        for sibling in 0..8 {
            fs::write(current.join(format!("sibling-{sibling}")), "").unwrap();
        }
        current = current.join("nested");
    }
    let limits = OperationLimits {
        entries: 12,
        depth: 32,
        bytes: 0,
        metadata_bytes: 64 * 1024,
    };
    let mut budget = Some(TreeBudget::new(limits));
    budget
        .as_mut()
        .unwrap()
        .charge(Path::new(""), EntryKind::Directory, 0)
        .unwrap();
    let mut captured = Vec::new();
    let error =
        capture_descendants(&root.0, Path::new(""), &mut captured, &mut budget).unwrap_err();
    assert!(error.is::<OperationLimitExceeded>());
    assert_eq!(captured.len(), limits.entries - 1);
    assert_eq!(budget.unwrap().entries, limits.entries + 1);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn application_file_copy_rejects_growth_past_its_eight_megabyte_limit() {
    use crate::plugin::filesystem::{Directory, Intent, MAX_DOCUMENT_BYTES, Prepared, Task};

    let root = TempDir::new();
    fs::write(root.join("source"), "original").unwrap();
    let directory = Directory {
        path: root.0.clone(),
        snapshot: DirectorySnapshot::read(&root.0).unwrap(),
        revision: "test".into(),
    };
    let entry = directory.entry_id(Path::new("source"));
    let Prepared::Plan(plan) = Task::Prepare {
        root: root.0.clone(),
        directory,
        intent: Intent::Copy {
            entry,
            destination: "destination".into(),
        },
    }
    .run()
    .unwrap() else {
        panic!("expected prepared plan")
    };

    let error = apply(&plan, |step, source, _| {
        if step == IoStep::CopyEntry {
            fs::OpenOptions::new()
                .write(true)
                .open(source)?
                .set_len(MAX_DOCUMENT_BYTES as u64 + 1)?;
        }
        Ok(())
    })
    .unwrap_err();
    assert!(error.to_string().contains("resource limits"));
    assert!(error.report.applied.is_empty());
    assert!(error.report.recovery.is_empty());
    assert_eq!(
        fs::metadata(root.join("source")).unwrap().len(),
        MAX_DOCUMENT_BYTES as u64 + 1
    );
    assert!(!root.join("destination").exists());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn copied_symlink_target_growth_consumes_the_metadata_budget() {
    let root = TempDir::new();
    fs::create_dir(root.join("source")).unwrap();
    std::os::unix::fs::symlink("short", root.join("source/link")).unwrap();
    let snapshot = DirectorySnapshot::read(&root.0).unwrap();
    let entry = &snapshot.entries()[0];
    let desired = vec![
        DesiredEntry::existing(entry, "source"),
        DesiredEntry::existing(entry, "destination"),
    ];
    let limits = OperationLimits {
        entries: 2,
        depth: 1,
        bytes: 0,
        metadata_bytes: 1500,
    };
    let plan = FsPlan::build_bounded(root.0.clone(), snapshot, desired, limits).unwrap();
    let error = apply(&plan, |step, source, _| {
        if step == IoStep::CopyEntry && source.file_name().is_some_and(|name| name == "link") {
            fs::remove_file(source)?;
            std::os::unix::fs::symlink("x".repeat(900), source)?;
        }
        Ok(())
    })
    .unwrap_err();
    assert!(error.to_string().contains("resource limits"));
    assert!(error.report.applied.is_empty());
    assert!(error.report.recovery.is_empty());
    assert_eq!(
        fs::read_link(root.join("source/link"))
            .unwrap()
            .as_os_str()
            .len(),
        900
    );
    assert!(!root.join("destination").exists());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
}
