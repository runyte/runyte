// SPDX-License-Identifier: MPL-2.0
use super::super::application as api;
use super::*;
use std::borrow::Cow;

#[derive(Deserialize)]
struct Probe<'a> {
    #[serde(rename = "type", borrow)]
    kind: Cow<'a, str>,
    #[serde(default, borrow)]
    method: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    params: Option<&'a RawValue>,
    #[serde(default, borrow)]
    settings_schema: Option<&'a RawValue>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<'a> {
    #[serde(rename = "type")]
    kind: String,
    id: String,
    method: String,
    #[serde(borrow)]
    params: &'a RawValue,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Set {
    expected_revision: String,
    document: Document,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delete {
    expected_revision: String,
}
#[derive(Deserialize)]
struct DocumentProbe<'a> {
    #[serde(borrow)]
    document: &'a RawValue,
}

/// No Value tree is built until the relevant raw subtree passed its own bound.
/// State requests remain raw while queued and parse under a local-work charge.
pub(crate) fn preflight(
    bytes: &[u8],
) -> anyhow::Result<(Option<super::super::ClientMessage>, bool)> {
    let probe: Probe<'_> = serde_json::from_slice(bytes)?;
    let settings = probe.kind == "register" && probe.settings_schema.is_some();
    if settings {
        json::validate(
            probe.settings_schema.unwrap().get(),
            json::Limits {
                max_bytes: 64 * 1024,
                max_depth: 8,
                max_nodes: 4096,
                max_container: 64,
            },
        )
        .map_err(|_| anyhow::anyhow!("Invalid application settings schema"))?;
    }
    if probe.kind != "request"
        || !matches!(
            probe.method.as_deref(),
            Some("state.get" | "state.set" | "state.delete")
        )
    {
        return Ok((None, settings));
    }
    let params = probe
        .params
        .ok_or_else(|| anyhow::anyhow!("Invalid state request"))?;
    if probe.method.as_deref() == Some("state.set") {
        let document: DocumentProbe<'_> = serde_json::from_str(params.get())
            .map_err(|_| anyhow::anyhow!("Invalid state request"))?;
        json::validate(document.document.get(), LIMITS)
            .map_err(|_| anyhow::anyhow!("State document exceeds its bounds"))?;
    }
    let envelope: Envelope<'_> =
        serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("Invalid state request"))?;
    anyhow::ensure!(
        envelope.kind == "request" && envelope.id.len() <= 64 && envelope.method.len() <= 96,
        "Invalid state request"
    );
    let request = match envelope.method.as_str() {
        "state.get" => {
            let _: api::Empty = serde_json::from_str(envelope.params.get())
                .map_err(|_| anyhow::anyhow!("Invalid state request"))?;
            api::Request::StateGet(api::Empty {})
        }
        "state.set" => {
            let value: Set = serde_json::from_str(envelope.params.get())
                .map_err(|_| anyhow::anyhow!("Invalid state request"))?;
            api::Request::StateSet {
                expected_revision: value.expected_revision,
                document: value.document,
            }
        }
        "state.delete" => {
            let value: Delete = serde_json::from_str(envelope.params.get())
                .map_err(|_| anyhow::anyhow!("Invalid state request"))?;
            api::Request::StateDelete {
                expected_revision: value.expected_revision,
            }
        }
        _ => unreachable!(),
    };
    Ok((
        Some(super::super::ClientMessage::Application(
            api::ClientMessage::Request {
                id: envelope.id,
                request,
            },
        )),
        false,
    ))
}
