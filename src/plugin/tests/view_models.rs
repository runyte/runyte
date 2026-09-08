// SPDX-License-Identifier: MPL-2.0
use super::*;
use std::sync::atomic::AtomicBool;

fn row(id: &str, text: &str) -> Row {
    Row {
        id: id.into(),
        text: text.into(),
        ..Default::default()
    }
}
fn model(rows: &[(&str, &str)]) -> Model {
    Model {
        title: "Application".into(),
        purpose: Purpose::List,
        rows: rows.iter().map(|(id, text)| row(id, text)).collect(),
        ..Default::default()
    }
}
fn prepare(model: Model) -> PreparedModel {
    PreparedModel::build(model, &AtomicBool::new(false)).unwrap()
}

#[test]
fn old_wire_shape_is_unchanged_and_large_models_have_immutable_encoded_content() {
    let legacy = serde_json::json!({"title":"Application","purpose":"list","rows":[{"id":"one","text":"hello","role":"ordinary"}]});
    let parsed: Model = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), legacy);
    assert_eq!(prepare(parsed).projection.text, "hello\n");
    let prepared = prepare(model(&[(
        "large",
        &"x".repeat(super::super::MAX_BYTES + 1),
    )]));
    assert!(prepared.encoded.len() > super::super::MAX_BYTES);
    assert!(prepared.charge >= prepared.encoded.len() + prepared.projection.text.len());
    let decoded: Model = serde_json::from_str(&prepared.encoded).unwrap();
    assert_eq!(decoded.rows[0].text.len(), super::super::MAX_BYTES + 1);
}

#[test]
fn columns_and_blocks_project_unicode_roles_and_non_actionable_headers() {
    let model = Model {
        columns: vec![
            Column {
                id: "name".into(),
                label: "Name".into(),
            },
            Column {
                id: "state".into(),
                label: "State".into(),
            },
        ],
        rows: vec![Row {
            id: "stable".into(),
            cells: vec![
                Cell {
                    text: "猫e\u{301}".into(),
                    role: Role::Heading,
                },
                Cell {
                    text: "ready".into(),
                    role: Role::Warning,
                },
            ],
            ..Default::default()
        }],
        status: Some(Block {
            text: "Online".into(),
            role: Role::Muted,
        }),
        detail: Some(Block {
            text: "first\nsecond".into(),
            role: Role::Ordinary,
        }),
        preview: Some(Block {
            text: "preview".into(),
            role: Role::Error,
        }),
        actions: vec!["open".into()],
        ..model(&[])
    };
    let prepared = prepare(model);
    let projection = prepared.projection;
    assert!(
        projection
            .text
            .starts_with("Online\nName  State\n猫e\u{301}")
    );
    assert_eq!(projection.rows[0].line, 2);
    assert_eq!(projection.line_rows[2], Some(0));
    assert_eq!(
        projection
            .line_rows
            .iter()
            .filter(|row| row.is_some())
            .count(),
        1
    );
    assert_eq!(projection.row_by_id["stable"], 0);
    assert_eq!(
        projection
            .text
            .chars()
            .skip(projection.rows[0].from)
            .take(projection.rows[0].to - projection.rows[0].from)
            .collect::<String>(),
        "猫e\u{301}   ready"
    );
    assert!(
        projection
            .spans
            .iter()
            .all(|span| span.from < span.to && span.to <= projection.text.chars().count())
    );
    assert!(
        projection
            .spans
            .windows(2)
            .all(|spans| spans[0].to <= spans[1].from)
    );
}

#[test]
fn column_truncation_retains_graphemes_and_full_model_values() {
    let content = format!("{}👩‍🔬after", "x".repeat(30));
    let model = Model {
        columns: vec![Column {
            id: "one".into(),
            label: "Value".into(),
        }],
        rows: vec![Row {
            id: "row".into(),
            cells: vec![Cell {
                text: content.clone(),
                role: Role::Ordinary,
            }],
            ..Default::default()
        }],
        ..model(&[])
    };
    let prepared = prepare(model);
    assert_eq!(prepared.model.rows[0].cells[0].text, content);
    assert!(prepared.projection.text.contains('…'));
    assert!(!prepared.projection.text.contains('👩'));
    assert!(!prepared.projection.text.contains('\u{200d}'));
}

#[test]
fn strict_model_constraints_cover_columns_blocks_actions_and_encoding() {
    let base = model(&[("one", "hello")]);
    let mut duplicate = base.clone();
    duplicate.rows.push(row("one", "other"));
    assert!(duplicate.validate().is_err());
    let mut invalid = base.clone();
    invalid.actions = vec!["open".into(), "open".into()];
    assert!(invalid.validate().is_err());
    invalid.actions = vec!["bad command".into()];
    assert!(invalid.validate().is_err());
    let mut invalid = base.clone();
    invalid.columns = vec![Column {
        id: "x".into(),
        label: "X".into(),
    }];
    assert!(invalid.validate().is_err());
    let mut invalid = base.clone();
    invalid.preview = Some(Block {
        text: "x\n".repeat(MAX_BLOCK_LINES),
        role: Role::Ordinary,
    });
    assert_eq!(
        invalid.validate().unwrap_err().code,
        ErrorCode::LimitExceeded
    );
    invalid.preview = Some(Block {
        text: "\u{85}".into(),
        role: Role::Ordinary,
    });
    assert!(invalid.validate().is_err());
    invalid.preview = None;
    invalid.status = Some(Block {
        text: "bad\nstatus".into(),
        role: Role::Warning,
    });
    assert!(invalid.validate().is_err());
    invalid.status = Some(Block {
        text: "x".repeat(1025),
        role: Role::Ordinary,
    });
    assert!(invalid.validate().is_err());
    assert_eq!(
        model(&[("huge", &"x".repeat(MAX_MODEL_BYTES))])
            .validate()
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded
    );
}

#[test]
fn projection_has_an_independent_byte_limit_even_for_a_valid_encoded_model() {
    let mut model = Model {
        columns: (0..MAX_COLUMNS)
            .map(|index| Column {
                id: index.to_string(),
                label: "x".repeat(32),
            })
            .collect(),
        rows: (0..MAX_ROWS)
            .map(|index| Row {
                id: index.to_string(),
                role: Role::Error,
                cells: (0..MAX_COLUMNS)
                    .map(|_| Cell {
                        text: String::new(),
                        role: Role::Error,
                    })
                    .collect(),
                ..Default::default()
            })
            .collect(),
        ..model(&[])
    };
    let free = MAX_MODEL_BYTES - serde_json::to_vec(&model).unwrap().len() - 64;
    model.rows[0].cells[0].text = "\u{301}".repeat(free / 2);
    model.validate().unwrap();
    let error = PreparedModel::build(model, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(error.code, ErrorCode::LimitExceeded);
    assert!(error.message.contains("Generated view text"));
}

#[test]
fn patches_are_ordered_atomic_and_reorder_exact_stable_id_sets() {
    let base = model(&[("a", "old"), ("b", "keep"), ("c", "last")]);
    let patch = Patch {
        header: Some(Header {
            title: "Dashboard".into(),
            purpose: Purpose::Dashboard,
            status: Some(Block {
                text: "Updated".into(),
                role: Role::Muted,
            }),
            ..Default::default()
        }),
        operations: vec![
            Operation::Update {
                row: row("a", "new"),
            },
            Operation::Remove {
                ids: vec!["b".into()],
            },
            Operation::Insert {
                before: Some("a".into()),
                row: row("d", "inserted"),
            },
            Operation::Reorder {
                ids: vec!["c".into(), "d".into(), "a".into()],
            },
        ],
    };
    let updated = base.patched(patch, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        updated
            .rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["c", "d", "a"]
    );
    assert_eq!(updated.rows[2].text, "new");
    assert_eq!(updated.title, "Dashboard");
    assert_eq!(base.rows[0].text, "old");
    for operation in [
        Operation::Reorder {
            ids: vec!["a".into(), "a".into(), "c".into()],
        },
        Operation::Remove {
            ids: vec!["missing".into()],
        },
        Operation::Update {
            row: row("missing", "no"),
        },
        Operation::Insert {
            before: Some("missing".into()),
            row: row("d", "no"),
        },
    ] {
        let before = serde_json::to_value(&base).unwrap();
        let patch = Patch {
            header: None,
            operations: vec![
                Operation::Update {
                    row: row("a", "tentative"),
                },
                operation,
            ],
        };
        assert!(base.patched(patch, &AtomicBool::new(false)).is_err());
        assert_eq!(serde_json::to_value(&base).unwrap(), before);
    }
}

#[test]
fn prepared_remap_uses_nearest_survivors_in_old_order_after_reordering() {
    let old = prepare(model(&[
        ("a", "a"),
        ("b", "b"),
        ("c", "c"),
        ("d", "d"),
        ("e", "e"),
    ]));
    let new = prepare(model(&[("d", "d"), ("e", "e"), ("c", "c"), ("a", "a")]));
    assert_eq!(
        new.projection.remap_from(&old.projection),
        vec![3, 3, 2, 0, 1]
    );
    assert_eq!(
        prepare(model(&[])).projection.remap_from(&old.projection),
        vec![0; 5]
    );
}

#[test]
fn staging_is_contiguous_bounded_and_keeps_failed_appends_atomic() {
    let mut stage = Stage::new("v:1".into(), "m:1".into(), None, StageKind::Model, 5).unwrap();
    assert_eq!(stage.append(0, "é").unwrap(), 2);
    assert!(!stage.is_complete());
    assert!(stage.append(1, "x").is_err());
    assert!(stage.append(2, "toolong").is_err());
    assert!(stage.append(2, "").is_err());
    assert_eq!(stage.append(2, "猫").unwrap(), 5);
    assert!(stage.is_complete());
    assert_eq!(stage.into_text().unwrap(), "é猫");
    assert!(Stage::new("v:1".into(), "m:1".into(), None, StageKind::Patch, 0).is_err());
    assert!(
        Stage::new(
            "v:1".into(),
            "m:1".into(),
            None,
            StageKind::Patch,
            MAX_MODEL_BYTES + 1
        )
        .is_err()
    );
    assert!(
        Stage::new("v:1".into(), "m:1".into(), None, StageKind::Patch, 1)
            .unwrap()
            .into_text()
            .is_err()
    );
}

#[test]
fn immutable_snapshot_chunks_preserve_byte_offsets_and_unicode_boundaries() {
    let snapshot = ReadSnapshot {
        revision: "m:2".into(),
        encoded: Arc::from("é猫{}"),
    };
    assert_eq!(snapshot.chunk(0, 4).unwrap(), ("é".into(), Some(2)));
    assert_eq!(snapshot.chunk(2, 4).unwrap(), ("猫{".into(), Some(6)));
    assert_eq!(snapshot.chunk(6, 4).unwrap(), ("}".into(), None));
    assert_eq!(snapshot.chunk(7, 1).unwrap(), ("".into(), None));
    for (offset, limit) in [(1, 4), (2, 1), (8, 4), (0, 0), (0, MAX_CHUNK_BYTES + 1)] {
        assert!(snapshot.chunk(offset, limit).is_err());
    }
}

#[test]
fn cancelled_model_or_patch_work_never_returns_a_publishable_candidate() {
    let model = model(&[("a", "one")]);
    assert_eq!(
        PreparedModel::build(model.clone(), &AtomicBool::new(true))
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert_eq!(
        model
            .patched(
                Patch {
                    header: None,
                    operations: vec![]
                },
                &AtomicBool::new(true)
            )
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
}

#[test]
fn retained_storage_counts_spare_string_capacity() {
    let mut value = model(&[("row", "text")]);
    let original = value.payload_bytes();
    let old_capacity = value.rows[0].text.capacity();
    value.rows[0].text.reserve_exact(4096);
    assert_eq!(
        value.payload_bytes() - original,
        value.rows[0].text.capacity() - old_capacity
    );
    value.rows[0].text.reserve_exact(MAX_RETAINED_BYTES);
    assert_eq!(value.validate().unwrap_err().code, ErrorCode::LimitExceeded);
}

#[test]
fn staged_patch_decode_stops_at_array_and_aggregate_reference_limits() {
    let ids = std::iter::repeat_n("\"\"", MAX_ROWS)
        .collect::<Vec<_>>()
        .join(",");
    // The malformed tail must not be visited after the first excess element.
    let excessive =
        format!("{{\"operations\":[{{\"kind\":\"remove\",\"ids\":[{ids},\"\",INVALID]}}]}}");
    let error = serde_json::from_str::<Patch>(&excessive).unwrap_err();
    assert!(error.to_string().contains("View array limit exceeded"));
    let operation = format!("{{\"kind\":\"remove\",\"ids\":[{ids}]}}");
    let aggregate = format!("{{\"operations\":[{operation},{operation},{operation}]}}");
    let error = serde_json::from_str::<Patch>(&aggregate).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("View patch reference limit exceeded")
    );
    let envelope = format!(
        "{{\"type\":\"request\",\"id\":\"p:1\",\"method\":\"view.patch\",\"params\":{{\"view\":\"v:1\",\"expected_revision\":\"m:1\",\"operations\":[{operation},{operation},{operation}]}}}}"
    );
    assert!(super::super::application::decode(envelope.as_bytes()).is_err());
}

#[test]
fn model_and_header_arrays_are_bounded_during_deserialization() {
    let column = serde_json::json!({"id":"one","label":"One"});
    let columns = vec![column; MAX_COLUMNS + 1];
    let header = serde_json::json!({"title":"List","purpose":"list","columns":columns});
    assert!(serde_json::from_value::<Header>(header.clone()).is_err());
    let mut value = header;
    value["rows"] = serde_json::json!([]);
    assert!(serde_json::from_value::<Model>(value).is_err());
    let cell = serde_json::json!({"text":"","role":"ordinary"});
    let value = serde_json::json!({"id":"one","text":"","role":"ordinary","cells":vec![cell;MAX_COLUMNS+1]});
    assert!(serde_json::from_value::<Row>(value).is_err());
    let operation = "{\"kind\":\"remove\",\"ids\":[]}";
    let value = format!(
        "{{\"operations\":[{}]}}",
        std::iter::repeat_n(operation, 1025)
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(
        serde_json::from_str::<Patch>(&value)
            .unwrap_err()
            .to_string()
            .contains("View array limit exceeded")
    );
}

#[test]
fn operation_decoder_streams_id_bounds_before_consuming_a_large_tail() {
    let head = "\"\",".repeat(MAX_ROWS + 1);
    let tail = "\"\",".repeat(1_000_000);
    // Put kind after ids: the bound cannot depend on discriminator ordering.
    let payload =
        format!("{{\"operations\":[{{\"ids\":[{head}{tail}INVALID],\"kind\":\"remove\"}}]}}");
    let mut reader = std::io::Cursor::new(payload.as_bytes());
    let error = serde_json::from_reader::<_, Patch>(&mut reader).unwrap_err();
    assert!(error.to_string().contains("View array limit exceeded"));
    assert!(
        reader.position() < 40_000,
        "consumed {} bytes",
        reader.position()
    );
}

#[test]
fn streaming_operation_decoder_preserves_strict_field_shapes_and_any_key_order() {
    let operation: Operation =
        serde_json::from_str("{\"ids\":[\"a\"],\"kind\":\"remove\"}").unwrap();
    assert!(matches!(operation, Operation::Remove { ids } if ids == ["a"]));
    for encoded in [
        "{\"kind\":\"remove\",\"kind\":\"remove\",\"ids\":[]}",
        "{\"kind\":\"remove\",\"ids\":[],\"ids\":[]}",
        "{\"kind\":\"remove\",\"ids\":[],\"before\":null}",
        "{\"kind\":\"insert\",\"before\":null,\"before\":null}",
        "{\"kind\":\"remove\",\"ids\":[],\"unknown\":0}",
        "{\"kind\":\"update\",\"row\":null}",
        "{\"kind\":\"insert\",\"row\":{\"id\":\"a\",\"text\":\"\",\"role\":\"ordinary\"}}",
        "{\"kind\":\"reorder\"}",
        "{\"ids\":[]}",
    ] {
        assert!(
            serde_json::from_str::<Operation>(encoded).is_err(),
            "{encoded}"
        );
    }
}
