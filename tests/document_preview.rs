// SPDX-License-Identifier: MPL-2.0
use runyte::{
    buffer::Buffer,
    document_preview::{MAX_BYTES, capture},
};

#[test]
fn whole_buffer_and_selected_capture_allow_one_mib_of_utf8_without_mutation() {
    assert_eq!(MAX_BYTES, 1_048_576);
    let text = "🦀".repeat(MAX_BYTES / 4);
    let mut buffer = Buffer::scratch();
    buffer.apply(&runyte::text::Transaction::insert(0, text.clone()));
    let revision = buffer.revision();
    assert_eq!(capture(&buffer, &[]).unwrap(), (text.clone(), false));
    assert_eq!(
        capture(&buffer, &[(0, MAX_BYTES / 4)]).unwrap(),
        (text.clone(), true)
    );
    buffer.apply(&runyte::text::Transaction::insert(MAX_BYTES / 4, "x"));
    assert_eq!(
        capture(&buffer, &[]).unwrap_err(),
        "preview limit is 1 MiB; select a smaller section"
    );
    assert_eq!(
        capture(&buffer, &[(0, MAX_BYTES / 4 + 1)]).unwrap_err(),
        "selected preview exceeds 1 MiB"
    );
    assert_eq!(capture(&buffer, &[(0, MAX_BYTES / 4)]).unwrap().0, text);
    assert_eq!(buffer.to_string(), format!("{text}x"));
    assert!(buffer.revision() > revision);
}

#[test]
fn disjoint_capture_charges_joining_newlines_and_preserves_range_limit() {
    let mut buffer = Buffer::scratch();
    buffer.apply(&runyte::text::Transaction::insert(
        0,
        "x".repeat(MAX_BYTES + 10),
    ));
    let spans = [(0, MAX_BYTES - 2), (MAX_BYTES, MAX_BYTES + 1)];
    let (text, selected) = capture(&buffer, &spans).unwrap();
    assert!(selected);
    assert_eq!(text.len(), MAX_BYTES);
    assert!(text.ends_with("\nx"));
    assert_eq!(
        capture(&buffer, &[(0, MAX_BYTES - 1), spans[1]]).unwrap_err(),
        "selected preview exceeds 1 MiB"
    );
    assert!(capture(&buffer, &vec![(0, 1); 256]).is_ok());
    assert!(
        capture(&buffer, &vec![(0, 1); 257])
            .unwrap_err()
            .contains("256 selections")
    );
}
