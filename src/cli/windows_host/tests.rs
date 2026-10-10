// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn cleanup_failures_retain_primary_native_error_and_report_both_services() {
    let result = finish_cleanup(
        Err(std::io::Error::from_raw_os_error(5).into()),
        Err(anyhow::anyhow!("context fixture failure")),
        Err(anyhow::anyhow!("plugin fixture failure")),
        Err(anyhow::anyhow!("catalog fixture failure")),
        Err(anyhow::anyhow!("transport fixture failure")),
    )
    .unwrap_err();
    assert_eq!(
        result
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .raw_os_error(),
        Some(5)
    );
    let message = format!("{result:#}");
    assert!(message.contains("context fixture failure"));
    assert!(message.contains("plugin fixture failure"));
    assert!(message.contains("catalog fixture failure"));
    assert!(message.contains("transport fixture failure"));
    let result = finish_cleanup(
        Ok(()),
        Ok(()),
        Ok(()),
        Ok(()),
        Err(anyhow::anyhow!("retirement failed")),
    )
    .unwrap_err();
    assert!(format!("{result:#}").contains("retirement failed"));
}

#[test]
fn console_close_keeps_its_type_through_host_cleanup() {
    let result = finish_cleanup(
        Err(super::super::terminated(super::super::ConsoleEvent::Close)),
        Err(anyhow::anyhow!("context was joined")),
        Err(anyhow::anyhow!("plugins were joined")),
        Err(anyhow::anyhow!("catalog was joined")),
        Err(anyhow::anyhow!("transport was joined")),
    )
    .unwrap_err();
    assert!(super::super::console_closed(&result));
    let detail = format!("{result:#}");
    assert!(detail.contains("context was joined"));
    assert!(detail.contains("plugins were joined"));
    assert!(detail.contains("catalog was joined"));
    assert!(detail.contains("transport was joined"));
}
