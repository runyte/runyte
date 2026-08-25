# Review local protocol and framing

Conduct a focused hardening review of the private versioned protocol used by
bundled local clients. This is a proactive review rather than evidence of a
known defect; make changes only for confirmed problems and do not turn the
protocol into a public compatibility contract. Fix every confirmed problem
that is safely within this category; do not stop after reporting findings.

The primary scope is `src/protocol/`, its use by `src/workspace/transport.rs`,
and `tests/local_protocol.rs`. Check frame-size and collection bounds, partial
reads and writes, malformed and truncated payloads, unknown fields and message
kinds, version negotiation, request identity, timeout behavior, slow or
disconnected peers, serialization failures, input-event validation, memory
growth, and the distinction between interactive and control connections.
Confirm that no malformed local message can panic the host or leave a request
in a permanently ambiguous state.

Add boundary and adversarial regression tests for every confirmed defect while
preserving the bundled-client-only contract.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
