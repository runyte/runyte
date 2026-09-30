// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn non_file_conflict_is_preserved_without_unmerged_stages() {
    let oid = "a".repeat(40);
    let bytes = format!(
        "{oid}\x00\x001\x00directory\x00CONFLICT (directory rename)\x00review directory placement\x00"
    );
    let MergePreviewOutcome::Merge {
        has_conflicts,
        conflicts,
        messages,
        ..
    } = parse_preview(bytes.as_bytes(), false).unwrap()
    else {
        panic!()
    };
    assert!(has_conflicts);
    assert!(conflicts.is_empty());
    assert_eq!(messages[0].paths, [PathBuf::from("directory")]);
    assert_eq!(messages[0].kind, "CONFLICT (directory rename)");
}

#[test]
fn malformed_machine_output_and_duplicate_stages_fail_closed() {
    let oid = "a".repeat(40);
    for bytes in [
        b"invalid\x00\x00".to_vec(),
        format!("{oid}\x00no-tab\x00\x00").into_bytes(),
        format!("{oid}\x00100644 invalid 2\tfile\x00\x00").into_bytes(),
        format!("{oid}\x00100644 {oid} 9\tfile\x00\x00").into_bytes(),
        format!("{oid}\x00999999 {oid} 2\tfile\x00\x00").into_bytes(),
        format!("{oid}\x00100644 {oid} 2\t../escape\x00\x00").into_bytes(),
        format!("{oid}\x00100644 {oid} 2\tfile\x00100644 {oid} 2\tfile\x00\x00").into_bytes(),
        format!("{oid}\x00\x00not-count\x00").into_bytes(),
        format!("{oid}\x00\x004097\x00").into_bytes(),
        format!("{oid}\x00\x001\x00file\x00").into_bytes(),
        format!("{oid}\x00\x001\x00file\x00CONFLICT").into_bytes(),
    ] {
        assert!(parse_preview(&bytes, false).is_err(), "{bytes:?}");
    }
    let staged = format!("{oid}\x00100644 {oid} 2\tfile\x00\x00");
    assert!(parse_preview(staged.as_bytes(), true).is_err());
}

#[test]
fn delimiter_and_unusual_path_bytes_are_kept_without_quoting() {
    let oid = "b".repeat(64);
    let bytes = format!(
        "{oid}\x00100644 {oid} 1\tspace and\t tab\nname\x00100644 {oid} 2\tspace and\t tab\nname\x00120000 {oid} 3\tspace and\t tab\nname\x00\x00"
    );
    let MergePreviewOutcome::Merge { conflicts, .. } =
        parse_preview(bytes.as_bytes(), false).unwrap()
    else {
        panic!()
    };
    assert_eq!(conflicts[0].path, PathBuf::from("space and\t tab\nname"));
    assert_eq!(conflicts[0].other.as_ref().unwrap().mode, "120000");
    assert_eq!(conflicts[0].base.as_ref().unwrap().oid.len(), 64);
}
