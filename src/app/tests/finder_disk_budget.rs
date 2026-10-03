// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn content_finder_excludes_open_disk_rows_before_the_scan_budget() {
    for background in [false, true] {
        let fixture = crate::test_support::TestRuntimeRoot::new("finder-disk-budget").unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let stale = root.join("z.txt");
        let fresh = root.join("a.txt");
        fs::write(&stale, "needle\n".repeat(CONTENT_ENTRY_LIMIT)).unwrap();
        fs::write(&fresh, "needle from disk\n").unwrap();
        let mut app = App::new(Config::default(), Some(stale.clone())).unwrap();
        app.project_root = root.clone();
        app.state_root = root.join("runtime");
        let length = app.buffers[0].len_chars();
        app.buffers[0].apply(&Transaction::new(vec![Change::new(
            0,
            length,
            "changed in memory\n",
        )]));
        let mut events = if background {
            let (scanner, events) = crate::file_picker::scanner();
            app.attach_file_scanner(scanner);
            Some(events)
        } else {
            None
        };
        app.open_project_grep().unwrap();
        if let Some(events) = events.as_mut() {
            let until = Instant::now() + std::time::Duration::from_secs(10);
            loop {
                if let Ok(event) = events.try_recv() {
                    let finished = matches!(event, FilePickerEvent::Finished { .. });
                    app.apply_file_picker_event(event);
                    if finished {
                        break;
                    }
                } else {
                    assert!(Instant::now() < until, "content scan did not finish");
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
        let picker = app.picker.as_ref().unwrap();
        let found = picker.views().any(|entry| entry.path == fresh);
        assert!(
            found,
            "unopened file survives discarded saved rows; background={background}"
        );
        assert!(
            !picker.limited,
            "discarded saved rows never use the content budget"
        );
        app.close_file_picker();
    }
}
