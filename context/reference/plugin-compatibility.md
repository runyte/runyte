# Plugin compatibility register

Stable support starts at Runyte **0.3.0**, with the public protocol `runyte-1`.
The normative public policy and range/negotiation semantics live in
[docs/plugins/compatibility.md](../../docs/plugins/compatibility.md).

Before 1.0, each `0.X` line preserves plugin compatibility across patches.
A breaking change requires an explicit new minor boundary. After 1.0, use
SemVer: compatible additions/deprecation increment minor, fixes increment patch,
and incompatible changes increment major. The release runbook remains the
authority for branch, version-only commits, publish flags and push/tag order.

The initial transition removes both experimental protocols and regenerates the
maintainer's configuration. Experimental clients are not compatibility targets.
Freeze the first stable client/schema/fixtures and the migrated ru-time artifact;
from that baseline forward, retain older stable profiles as regression evidence.

Required release evidence includes native Linux/macOS conformance, immutable
external-plugin pins, base-profile old-schema/client checks, behavior beyond
registration, all ordinary gates and the existing coverage floor. Moving-head
external checks are advisory. No skip, mutable reference, previous-commit CI
result or temporary candidate build authorizes publication of the release commit.

Compatibility includes wire and semantic behavior: messages, commands, explicit
targets and revisions, transactional edits/undo, save outcomes, event ordering,
permissions, lifecycle, cancellation and resource limits. Schema-only validation
does not establish these guarantees. A change must assess the affected contract
and add meaningful regression coverage at its behavior boundary.

Implementation and initial validation are tracked in
[the completed transition plan](../plans/completed/PLAN_STABLE_PLUGINS.md).
Its lifecycle closure records delivered implementation, not certification of
unverified release gates. This register does not establish registry publication.

The external agent context transport adds the negotiated
`runyte.context.v1` profile on a separate, natively granted Unix socket.
Its schema and independently vendorable Python client live in
`docs/plugins/runyte-context-1.schema.json` and `docs/plugins/context_client.py`.
`docs/plugins/check_context.py` verifies closed request shapes, old-transport
denial, scoped admission, response identity and uncertain mutation outcomes.
No frozen `compatibility/v1/` artifact or immutable inventory pin changes for
this addition; those continue to test ordinary stable process applications.

The explicit **0.4.0** boundary changes editor launch modes: `--persistent` is
removed, and `--init DIRECTORY` creates a workspace and exits. The retained
ru-time 0.3 revision (`4e586d9`) remains pinned with its `<0.4.0` range; the
0.4 gate verifies that it rejects the new host before registration. The new
ru-time revision (`3e33230`) initializes its test workspace separately and
launches with `--ide` or `--mux`; its native suite is required for 0.4. Both
revisions vendor the same unchanged `runyte-1` Python client and schema, which
are recorded as separate release-line profiles in the immutable inventory.
Development CI stages a 0.4.0 candidate from the current source without
editing its checkout. Release CI must use the exact 0.4.0 package version and
source commit under the release runbook.

The bundled examples and MCP adapter admit both 0.3 and 0.4 hosts with
`>=0.3.0, <0.5.0`; their wire contract is unchanged. The documented
configuration examples, the admission error's suggestion (`RECOMMENDED_RANGE`),
the benchmark harness and the in-crate context fixtures use the same range.
`STABLE_RANGE` stays `>=0.3.0, <0.4.0` because it is the acknowledgement in the
shared wire fixtures. Native example registration
and the two-client MCP suite run against the staged 0.4 host in CI. The MCP
fixture initializes its temporary workspaces before launching ide or mux.

The optional `input-path-completion` feature adds `completion: "local-path"`
to text fields in native prompts and forms. Hosts require negotiation before
accepting the property; plugins omit it for older hosts. This changes neither
the `runyte-1` epoch nor the frozen base schema/client fixtures. Native path
selection edits the field only; submission and validation retain their existing
authority and revision checks.
