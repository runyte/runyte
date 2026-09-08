// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn bounded_copy_rechecks_tree_growth_during_copy_and_cleans_owned_staging() {
    for growth in ["entries", "depth", "bytes", "metadata"] {
        let root = TempDir::new();
        fs::create_dir(root.join("source")).unwrap();
        fs::write(root.join("source/child"), "data").unwrap();
        let snapshot = DirectorySnapshot::read(&root.0).unwrap();
        let entry = &snapshot.entries()[0];
        let desired = vec![
            DesiredEntry::existing(entry, "source"),
            DesiredEntry::existing(entry, "destination"),
        ];
        let limits = OperationLimits {
            entries: if growth == "entries" { 4 } else { 64 },
            depth: 2,
            bytes: 8,
            metadata_bytes: if growth == "metadata" { 1600 } else { 4096 },
        };
        let plan = FsPlan::build_bounded(root.0.clone(), snapshot, desired, limits).unwrap();
        let error = apply(&plan, move |step, source, _| {
            if step == IoStep::CopyEntry && source.file_name().is_some_and(|name| name == "source")
            {
                match growth {
                    "entries" => {
                        for n in 0..4 {
                            fs::write(source.join(format!("new-{n}")), "")?;
                        }
                    }
                    "depth" => fs::create_dir_all(source.join("a/b/c"))?,
                    "bytes" => fs::write(source.join("child"), "over eight bytes")?,
                    "metadata" => {
                        fs::write(source.join("x".repeat(100)), "")?;
                    }
                    _ => unreachable!(),
                }
            }
            Ok(())
        })
        .unwrap_err();
        assert!(
            error.to_string().contains("resource limits"),
            "{growth}: {error}"
        );
        assert!(error.report.applied.is_empty(), "{growth}");
        assert!(error.report.recovery.is_empty(), "{growth}");
        assert!(root.join("source/child").is_file());
        assert!(!root.join("destination").exists());
        assert_eq!(
            fs::read_dir(&root.0).unwrap().count(),
            1,
            "{growth}: staging leaked"
        );
    }
}
