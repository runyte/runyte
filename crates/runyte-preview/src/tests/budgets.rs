// SPDX-License-Identifier: MPL-2.0
use std::io::Cursor;

#[test]
fn helper_readers_reject_oversized_wire_and_missing_newline() {
    let oversized = vec![b' '; super::limits::MAX_HELPER_MESSAGE_BYTES + 1];
    let once = super::run_once(std::iter::empty(), Cursor::new(&oversized), Vec::new(), &[]);
    assert!(once.unwrap_err().to_string().contains("wire budget"));
    let retained = super::serve(Cursor::new(&oversized), Vec::new(), &[]);
    assert!(retained.unwrap_err().to_string().contains("wire budget"));
    let missing_newline = super::serve(Cursor::new(b"{}"), Vec::new(), &[]);
    assert!(missing_newline.unwrap_err().to_string().contains("framing"));
}
