// SPDX-License-Identifier: MPL-2.0
use super::*;

/// All retained captures at their maximum size and worst JSON expansion.
pub(crate) fn large_preview_response() -> HostResponse {
    let root = crate::test_support::TestRuntimeRoot::new("preview-wire-frame").unwrap();
    let app =
        crate::app::App::new_in_project(crate::config::Config::default(), None, &*root).unwrap();
    let mut host = crate::workspace::WorkspaceHost::new(app);
    let mut frame = host.prepare_frame(FrameGeometry::default());
    let pane = frame.editor.panes[0].clone();
    frame.editor.panes = (0..crate::document_preview::MAX_CAPTURES)
        .map(|id| {
            let mut pane = pane.clone();
            pane.pane_id += id;
            pane.active = id == 0;
            pane.preview = Some(crate::document_preview::DocumentPreview {
                generation: id as u64 + 1,
                source: 0,
                selection_only: false,
                text: "\0".repeat(crate::document_preview::MAX_BYTES),
                language: "text".into(),
                path: None,
            });
            pane
        })
        .collect();
    HostResponse::Frame {
        frame: Box::new(frame.into()),
    }
}

pub(crate) fn assert_large_previews(response: HostResponse) {
    let HostResponse::Frame { frame } = response else {
        panic!("expected frame")
    };
    let frame = crate::workspace::HostFrame::try_from(*frame).unwrap();
    assert_eq!(
        frame.editor.panes.len(),
        crate::document_preview::MAX_CAPTURES
    );
    for pane in frame.editor.panes {
        let capture = pane.preview.unwrap();
        assert_eq!(capture.text.len(), crate::document_preview::MAX_BYTES);
        assert!(capture.text.bytes().all(|byte| byte == 0));
    }
}

#[tokio::test]
async fn host_frames_carry_eight_maximum_escaped_previews_without_widening_requests() {
    let response = large_preview_response();
    assert!(
        encode_message(&response).is_err(),
        "request budget must stay 8 MiB"
    );
    let (mut writer, reader) = tokio::io::duplex(64 * 1024);
    let write = write_host_message(&mut writer, &response);
    let mut reader = MessageReader::host_responses(reader);
    let (written, received) = tokio::join!(write, reader.read::<HostResponse>());
    written.unwrap();
    assert_large_previews(received.unwrap().unwrap());
}

#[tokio::test]
async fn host_reader_and_encoder_enforce_their_wire_bound() {
    let too_large = "x".repeat(MAX_HOST_MESSAGE_BYTES - 2);
    assert!(encode_message_with_limit(&too_large, MAX_HOST_MESSAGE_BYTES).is_err());
    drop(too_large);
    // Stream a chunk repeatedly so the fixture itself does not allocate a
    // second maximum-sized input. No newline means rejection at the byte bound.
    let (mut writer, reader) = tokio::io::duplex(64 * 1024);
    let write = async move {
        let chunk = vec![b'x'; 64 * 1024];
        for _ in 0..=MAX_HOST_MESSAGE_BYTES / chunk.len() {
            if writer.write_all(&chunk).await.is_err() {
                break;
            }
        }
    };
    let read = async move {
        MessageReader::host_responses(reader)
            .read::<HostResponse>()
            .await
    };
    let (_, result) = tokio::join!(write, read);
    assert!(result.unwrap_err().to_string().contains("exceeds"));
}
