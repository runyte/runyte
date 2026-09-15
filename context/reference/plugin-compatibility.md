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
[the active transition plan](../plans/active/PLAN_STABLE_PLUGINS.md). This register
does not claim that 0.3.0 has already been published or certified.
