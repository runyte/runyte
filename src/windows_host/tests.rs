// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn cleanup_failures_retain_primary_native_error_and_report_both_services() {
    let result = finish_cleanup(
        Err(std::io::Error::from_raw_os_error(5).into()),
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
    assert!(message.contains("plugin fixture failure"));
    assert!(message.contains("catalog fixture failure"));
    assert!(message.contains("transport fixture failure"));
    let result = finish_cleanup(
        Ok(()),
        Ok(()),
        Ok(()),
        Err(anyhow::anyhow!("retirement failed")),
    )
    .unwrap_err();
    assert!(format!("{result:#}").contains("retirement failed"));
}
