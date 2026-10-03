// SPDX-License-Identifier: MPL-2.0

use runyte::file_picker::{
    CONTENT_ENTRY_LIMIT, FilePicker, FilePickerEvent, ScanScope, line_hits, scan_content, scanner,
};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let root = Self(std::env::temp_dir().join(format!(
                "runyte-dense-finder-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
        fs::create_dir(&root.0).unwrap();
        root
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_dense_file_reports_overflow_before_narrowed_query_rescans() {
    let root = Root::new();
    let file = root.0.join("dense.txt");
    let exact = "needle\n".repeat(CONTENT_ENTRY_LIMIT);
    fs::write(&file, &exact).unwrap();
    let scope = ScanScope::Everything;
    let (_, _, limited) =
        scan_content(&root.0, &scope, &root.0.join(".runyte"), false, "needle").unwrap();
    assert!(!limited, "exactly the budget is a complete scan");
    let overflowing = format!("{exact}needle rare\n");
    assert_eq!(line_hits(&overflowing, "needle").len(), CONTENT_ENTRY_LIMIT);
    fs::write(&file, overflowing).unwrap();
    let (hits, skipped, limited) =
        scan_content(&root.0, &scope, &root.0.join(".runyte"), false, "needle").unwrap();
    assert!(limited, "the per-file cap concealed an extra matching line");
    assert_eq!(
        hits.iter().map(|hit| hit.len()).sum::<usize>(),
        CONTENT_ENTRY_LIMIT
    );
    let mut picker = FilePicker::grep(1, root.0.clone(), scope.clone());
    picker.query = "needle".to_owned();
    picker.query_cursor = picker.query.len();
    picker.add_content(hits);
    picker.finish(skipped, limited);
    picker.insert_query_text(" rare");
    assert!(picker.content_rescan_needed());
    let (hits, _, limited) = scan_content(
        &root.0,
        &scope,
        &root.0.join(".runyte"),
        false,
        &picker.query,
    )
    .unwrap();
    assert!(!limited);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].lines[0].row, CONTENT_ENTRY_LIMIT);

    let (scanner, mut events) = scanner();
    scanner.scan_content(
        9,
        root.0.clone(),
        scope,
        root.0.join(".runyte"),
        false,
        "needle".into(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut rows = 0;
    loop {
        match events.try_recv() {
            Ok(FilePickerEvent::Content { entries, .. }) => {
                rows += entries.iter().map(|hit| hit.len()).sum::<usize>()
            }
            Ok(FilePickerEvent::Finished { limited, .. }) => {
                assert!(limited);
                assert_eq!(rows, CONTENT_ENTRY_LIMIT);
                break;
            }
            Ok(FilePickerEvent::Failed { message, .. }) => panic!("{message}"),
            Ok(_) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "content scan did not finish");
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("content scan disconnected: {error}"),
        }
    }
}
