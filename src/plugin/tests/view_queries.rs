// SPDX-License-Identifier: MPL-2.0
use super::*;

#[test]
fn query_preconditions_prevent_legacy_or_old_results_after_activation() {
    check_query(None, None).unwrap();
    assert!(check_query(None, Some("qv:1")).is_err());
    let first = QueryState::set(None, None, "猫 query".into()).unwrap();
    assert_eq!(
        first.wire(),
        Query {
            revision: "qv:1".into(),
            text: "猫 query".into(),
            pending: true
        }
    );
    assert_eq!(
        check_query(Some(&first), None).unwrap_err().code,
        ErrorCode::Stale
    );
    check_query(Some(&first), Some("qv:1")).unwrap();
    let retry = QueryState::set(Some(&first), Some("qv:1"), first.text.clone()).unwrap();
    assert_eq!(retry.revision, 2);
    assert!(retry.pending);
    assert_eq!(
        check_query(Some(&retry), Some("qv:1")).unwrap_err().code,
        ErrorCode::Stale
    );
    assert_eq!(first.revision, 1);
    let cleared = QueryState::set(Some(&retry), Some("qv:2"), String::new()).unwrap();
    assert_eq!(cleared.revision, 3);
    assert!(cleared.pending);
    assert!(cleared.text.is_empty());
}

#[test]
fn query_failure_is_bounded_and_leaves_previous_pending_state_untouched() {
    let mut settled = QueryState::set(None, None, "old".into()).unwrap();
    settled.pending = false;
    for text in [
        "x".repeat(MAX_QUERY_BYTES + 1),
        "one\ntwo".into(),
        "secret\u{85}".into(),
    ] {
        assert!(QueryState::set(Some(&settled), Some("qv:1"), text).is_err());
        assert!(!settled.pending);
        assert_eq!(settled.text, "old");
    }
    let boundary = QueryState::set(None, None, "é".repeat(MAX_QUERY_BYTES / 2)).unwrap();
    assert_eq!(boundary.text.len(), MAX_QUERY_BYTES);
    settled.revision = u64::MAX;
    assert_eq!(
        QueryState::set(
            Some(&settled),
            Some(&format!("qv:{}", u64::MAX)),
            String::new()
        )
        .unwrap_err()
        .code,
        ErrorCode::Internal
    );
}

#[test]
fn stages_retain_the_exact_query_captured_before_chunk_assembly() {
    let mut stage = Stage::new(
        "v:1".into(),
        "m:2".into(),
        Some("qv:3".into()),
        StageKind::Model,
        2,
    )
    .unwrap();
    stage.append(0, "{}").unwrap();
    assert_eq!(stage.expected_query_revision.as_deref(), Some("qv:3"));
    assert_eq!(stage.into_text().unwrap(), "{}");
}

#[test]
fn accepted_query_does_not_retain_an_oversized_input_allocation() {
    let mut text = String::with_capacity(QUERY_CHARGE * 4);
    text.push_str("query");
    let query = QueryState::set(None, None, text).unwrap();
    assert_eq!(query.text, "query");
    assert_eq!(query.text.capacity(), query.text.len());
}
