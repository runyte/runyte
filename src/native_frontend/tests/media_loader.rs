// SPDX-License-Identifier: MPL-2.0
use super::*;

fn key(page: usize) -> Key {
    Key {
        path: "document.pdf".into(),
        page,
        detail: None,
        modified: None,
        length: 100,
    }
}
fn loaded(key: Key, pages: usize) -> Loaded {
    Loaded {
        result: Ok((
            Arc::new(Page {
                source: (key.modified, key.length),
                animation: None,
                image: Arc::new(RenderImage::new(vec![image::Frame::new(
                    image::RgbaImage::new(1, 1),
                )])),
                width: 1.0,
                height: 1.0,
                words: Vec::new(),
                text_error: None,
            }),
            pages,
        )),
        key,
    }
}
fn finish(schedule: &mut Schedule, pages: usize) -> (bool, Option<(PathBuf, usize)>) {
    let request = schedule.next().unwrap();
    schedule.complete(loaded(request.key.clone(), pages), &request.key)
}

#[test]
fn adjacent_prefetch_is_bounded_and_does_not_cascade() {
    let mut schedule = Schedule::default();
    assert!(schedule.get(key(5)).is_none());
    assert_eq!(
        finish(&mut schedule, 100),
        (true, Some(("document.pdf".into(), 100)))
    );
    let mut prefetched = Vec::new();
    while let Some(request) = schedule.next() {
        assert!(request.speculative);
        prefetched.push(request.key.page);
        assert_eq!(
            schedule.complete(loaded(request.key.clone(), 100), &request.key),
            (false, None)
        );
    }
    assert_eq!(prefetched, [6, 4, 7, 3]);
    assert_eq!(schedule.cache.len(), 5);
    assert!(schedule.get(key(6)).unwrap().is_ok());
    assert!(schedule.get(key(5)).unwrap().is_ok());
    assert!(schedule.demand.is_empty());
}

#[test]
fn requested_page_cancels_speculation_and_runs_before_other_neighbors() {
    let mut schedule = Schedule::default();
    schedule.get(key(5));
    finish(&mut schedule, 100);
    let speculative = schedule.next().unwrap();
    assert_eq!(speculative.key.page, 6);
    schedule.get(key(40));
    assert!(speculative.cancel.load(Ordering::Acquire));
    assert_eq!(
        schedule.complete(loaded(speculative.key.clone(), 100), &speculative.key),
        (false, None)
    );
    assert!(!schedule.cache.iter().any(|entry| entry.key.page == 6));
    let demand = schedule.next().unwrap();
    assert_eq!(demand.key.page, 40);
    assert!(!demand.speculative);
    assert_eq!(
        schedule.complete(loaded(demand.key.clone(), 100), &demand.key),
        (true, None)
    );
    assert_eq!(
        schedule
            .speculative
            .iter()
            .map(|key| key.page)
            .collect::<Vec<_>>(),
        [41, 39, 42, 38]
    );
}

#[test]
fn requesting_inflight_neighbor_promotes_it_without_canceling_or_duplicate_work() {
    let mut schedule = Schedule::default();
    schedule.get(key(5));
    finish(&mut schedule, 10);
    let speculative = schedule.next().unwrap();
    schedule.get(key(6));
    assert!(!speculative.cancel.load(Ordering::Acquire));
    assert!(schedule.demand.is_empty());
    assert_eq!(
        schedule.complete(loaded(speculative.key.clone(), 10), &speculative.key),
        (true, None)
    );
    assert!(schedule.get(key(6)).unwrap().is_ok());
}

#[test]
fn cache_evicts_oldest_page_without_flushing_recent_back_navigation() {
    let mut schedule = Schedule::default();
    for page in 1..=8 {
        schedule.get(key(page));
        finish(&mut schedule, 20);
    }
    assert!(schedule.get(key(1)).unwrap().is_ok());
    schedule.get(key(9));
    finish(&mut schedule, 20);
    assert_eq!(schedule.cache.len(), CACHE_PAGES);
    assert!(schedule.get(key(8)).unwrap().is_ok());
    assert!(schedule.get(key(1)).unwrap().is_ok());
    assert!(!schedule.cache.iter().any(|entry| entry.key.page == 2));
}

#[test]
fn changed_source_cancels_old_work_and_republishes_count_once() {
    let mut schedule = Schedule::default();
    schedule.get(key(5));
    finish(&mut schedule, 20);
    let stale = schedule.next().unwrap();
    let revised = Key {
        length: 101,
        ..key(5)
    };
    schedule.get(revised.clone());
    assert!(stale.cancel.load(Ordering::Acquire));
    assert!(schedule.cache.is_empty());
    assert_eq!(
        schedule.complete(loaded(stale.key.clone(), 20), &revised),
        (false, None)
    );
    assert_eq!(
        finish(&mut schedule, 30),
        (true, Some(("document.pdf".into(), 30)))
    );
    assert_eq!(finish(&mut schedule, 30), (false, None));
    assert!(schedule.cache.iter().all(|entry| entry.key.length == 101));
}

#[test]
fn completion_revalidates_source_even_without_another_get() {
    let mut schedule = Schedule::default();
    schedule.get(key(1));
    let active = schedule.next().unwrap();
    let revised = Key {
        length: 200,
        ..key(1)
    };
    assert_eq!(
        schedule.complete(loaded(active.key, 20), &revised),
        (true, None)
    );
    assert!(schedule.cache.is_empty());
}

#[test]
fn prefetch_respects_document_boundaries_and_does_not_apply_to_images() {
    for (page, pages, expected) in [(1, 1, vec![]), (1, 2, vec![2]), (10, 10, vec![9, 8])] {
        let mut schedule = Schedule::default();
        schedule.get(key(page));
        finish(&mut schedule, pages);
        assert_eq!(
            schedule
                .speculative
                .iter()
                .map(|key| key.page)
                .collect::<Vec<_>>(),
            expected
        );
    }
    let mut schedule = Schedule::default();
    schedule.get(Key {
        path: "image.png".into(),
        ..key(1)
    });
    finish(&mut schedule, 1);
    assert!(schedule.next().is_none());
}

#[test]
fn demand_queues_and_source_tracking_are_bounded_across_many_panes() {
    let mut schedule = Schedule::default();
    for i in 0..100 {
        schedule.get(Key {
            path: format!("document-{i}.pdf").into(),
            ..key(1)
        });
    }
    assert_eq!(schedule.demand.len(), QUEUED_PAGES);
    assert_eq!(schedule.sources.len(), CACHE_PAGES);
    assert_eq!(
        schedule.next().unwrap().key.path,
        PathBuf::from("document-99.pdf")
    );
}

#[test]
fn retained_counts_survive_reopen_without_repeated_publications() {
    let mut counts = Vec::new();
    assert!(retain_count(&mut counts, &key(1), 20));
    // Reading another cached page, including after reopening, leaves the same
    // fingerprinted count available to the host without another resize wake.
    assert!(!retain_count(&mut counts, &key(5), 20));
    assert_eq!(counts, vec![(PathBuf::from("document.pdf"), None, 100, 20)]);
    let replacement = Key {
        length: 101,
        ..key(1)
    };
    assert!(retain_count(&mut counts, &replacement, 30));
    assert_eq!(counts, vec![(PathBuf::from("document.pdf"), None, 101, 30)]);
}

#[test]
fn cached_count_access_refreshes_bounded_retention_and_can_restore_evicted_count() {
    let mut counts = Vec::new();
    retain_count(&mut counts, &key(1), 20);
    for i in 0..7 {
        retain_count(
            &mut counts,
            &Key {
                path: format!("other-{i}.pdf").into(),
                ..key(1)
            },
            10,
        );
    }
    assert!(!retain_count(&mut counts, &key(5), 20));
    retain_count(
        &mut counts,
        &Key {
            path: "new.pdf".into(),
            ..key(1)
        },
        10,
    );
    assert_eq!(counts.len(), CACHE_PAGES);
    assert!(
        counts
            .iter()
            .any(|entry| entry.0 == Path::new("document.pdf"))
    );
    assert!(
        !counts
            .iter()
            .any(|entry| entry.0 == Path::new("other-0.pdf"))
    );
    let evicted = Key {
        path: "other-0.pdf".into(),
        ..key(1)
    };
    assert!(retain_count(&mut counts, &evicted, 10));
    assert!(!retain_count(&mut counts, &evicted, 10));
    assert_eq!(counts.len(), CACHE_PAGES);
}

fn detail_key(page: usize, x: u32) -> Key {
    Key {
        detail: Some(Detail {
            full: [6400, 4800],
            origin: [x, 100],
            size: [800, 600],
        }),
        ..key(page)
    }
}

#[test]
fn detail_changes_cancel_obsolete_work_but_preserve_other_visible_panes() {
    let mut schedule = Schedule::default();
    schedule.get(key(1));
    finish(&mut schedule, 10);
    let base = schedule.get(key(1)).unwrap().unwrap();
    let mut viewport = super::super::viewport::Viewport::new(1);
    viewport.show_source(&base);
    viewport.zoom = 4.;
    viewport.center = [0.3, 0.4];
    viewport.selection = Some(super::super::viewport::Selection::Region(
        [0.1, 0.2],
        [0.4, 0.5],
    ));
    let geometry = viewport.geometry([base.width, base.height], [800., 600.]);
    let first = detail_key(1, 100);
    let other_pane = detail_key(1, 2000);
    schedule.get(first.clone());
    let active = schedule.next().unwrap();
    schedule.get(other_pane.clone());
    schedule.retain_details(&[first.clone(), other_pane.clone()]);
    assert!(!active.cancel.load(Ordering::Acquire));
    let latest = detail_key(1, 200);
    schedule.get(latest.clone());
    schedule.retain_details(&[latest.clone(), other_pane.clone()]);
    assert!(active.cancel.load(Ordering::Acquire));
    assert_eq!(
        schedule.complete(loaded(first.clone(), 10), &first),
        (true, None)
    );
    assert!(schedule.details.is_empty());
    assert_eq!(finish(&mut schedule, 10), (true, None));
    assert_eq!(finish(&mut schedule, 10), (true, None));
    assert!(schedule.get(latest).unwrap().is_ok());
    let same_base = schedule.get(key(1)).unwrap().unwrap();
    assert!(Arc::ptr_eq(&base, &same_base));
    viewport.show_source(&same_base);
    assert_eq!(viewport.zoom, 4.);
    assert_eq!(viewport.center, [0.3, 0.4]);
    assert!(viewport.selection.is_some());
    assert_eq!(
        viewport.geometry([same_base.width, same_base.height], [800., 600.]),
        geometry
    );
    assert!(schedule.get(other_pane).unwrap().is_ok());
    assert!(schedule.get(key(1)).unwrap().is_ok());
    assert!(schedule.speculative.iter().all(|key| key.detail.is_none()));
    schedule.get(detail_key(1, 500));
    let pending = schedule.next().unwrap();
    schedule.retain_details(&[]);
    assert!(pending.cancel.load(Ordering::Acquire));
    assert!(schedule.demand.is_empty());
}

#[test]
fn detail_cache_is_bounded_separate_from_base_pages_and_invalidated_with_source() {
    let mut schedule = Schedule::default();
    schedule.get(key(1));
    finish(&mut schedule, 10);
    for x in 0..12 {
        schedule.get(detail_key(1, x));
        finish(&mut schedule, 10);
    }
    assert_eq!(schedule.details.len(), CACHE_PAGES);
    assert!(schedule.get(key(1)).unwrap().is_ok());
    assert!(schedule.get(detail_key(1, 0)).is_none());
    let active = schedule.next().unwrap();
    let revised = Key {
        length: 101,
        ..key(1)
    };
    schedule.get(revised);
    assert!(active.cancel.load(Ordering::Acquire));
    assert!(schedule.details.is_empty());
    assert!(schedule.cache.is_empty());
}

#[test]
fn detail_failures_keep_base_page_available_and_do_not_retry_each_frame() {
    let mut schedule = Schedule::default();
    schedule.get(key(1));
    finish(&mut schedule, 10);
    let sharp = detail_key(1, 100);
    schedule.get(sharp.clone());
    let request = schedule.next().unwrap();
    assert_eq!(
        schedule.complete(
            Loaded {
                key: request.key,
                result: Err("helper failed".into())
            },
            &sharp
        ),
        (true, None)
    );
    assert!(schedule.get(sharp).unwrap().is_err());
    assert!(schedule.get(key(1)).unwrap().is_ok());
    assert!(schedule.demand.is_empty());
}
