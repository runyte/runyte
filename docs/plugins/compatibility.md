# Stable compatibility

Stable plugin support begins at Runyte **0.3.0**, using **`runyte-1`**.
Experimental epochs are removed in this cutover. Regenerate configuration and
update the plugin/client together; there is no legacy mode or automatic fallback.
Restart existing persistent hosts to load the new executable and configuration.

The separately authenticated [workspace context profile](context.md) negotiates
`runyte.context.v1` on its own Unix transport. Its read/edit/proposal scopes do
not broaden the meanings of ordinary process-plugin capabilities. The current
schema references a separate context schema, and its standalone Python client
has independent conformance tests. The retained `compatibility/v1/` schema,
client and fixtures are unchanged; context messages are never sent to clients
that did not enter and negotiate this transport.

## The promise

Before 1.0, later patches in `0.X.Y` preserve the stable plugin contract;
a new `X` may break it after an explicit compatibility decision. After 1.0,
compatible features and deprecations use minor releases, compatible fixes use
patch releases, and incompatible changes require a major release.

Compatibility preserves messages and their meanings: commands/arguments, native
foreground authority, transactional edits and undo, document saves and dirty
state, event baselines/order/resynchronization, capability meanings, connection
ownership, cancellation, shutdown, limits and uncertain outcomes. It does not
freeze private frontend DTOs, exact diagnostic prose, rendering pixels or timing
within documented bounds. A successful registration alone is insufficient.

An older stable plugin and its pinned client must run unchanged on a later host
inside its supported range. A newer plugin using a new feature either raises its
minimum host release or negotiates an optional fallback. No promise applies
retroactively to experimental plugins or modified builds reusing release numbers.

## Authored metadata and configuration

Keep the supported range in plugin-owned source/package metadata. For Python,
pass it explicitly to the maintained client:

```python
app = Application('Example', commands, ['views'],
                  runyte='>=0.3.0, <0.4.0')
```

Copy the same value into generated configuration:

```yaml
api: runyte-1
runyte: ">=0.3.0, <0.4.0"
```

Runyte has no plugin manifest loader, bundle scan or metadata subprocess probe.
The user runs a plugin's configuration generator during installation. Before
launch or restart, the owning host compares its own compiled package version
with the configured interval. Missing/invalid metadata or a mismatch leaves a
failed entry without starting a process. Other valid entries can still start.

Registration contains the authoritative plugin-authored range. The host checks
it before installing commands. Configuration may narrow the authored interval,
but must be a subset of it. Wider or stale configuration is rejected even if
the current host belongs to both ranges; regenerate or narrow the entry.
No configuration value bypasses the process's declaration.

The attached frontend, a newer executable on disk and a program found on PATH
cannot substitute their version for the running persistent host's version.
Unsupported release, protocol, capability, feature and configuration grant are
distinct failures. Registration refusals use bounded `registration_error` frames
when delivery is possible and retain a manager diagnostic through cleanup.

## Range syntax

Use one lower (`>=` or `>`) and one upper (`<` or `<=`) comparator separated by
a comma, in either order; or one exact `=version`. Core versions have exactly
three unsigned 64-bit components, without leading zeros. Spaces/tabs around
tokens are allowed. Declarations are at most 256 UTF-8 bytes.

| Range | Accepted releases |
| --- | --- |
| `>=0.5.2, <0.6.0` | 0.5.2 and later 0.5 patches |
| `>0.5.2, <=0.5.4` | 0.5.3 and 0.5.4 |
| `=0.5.2` | Exactly 0.5.2, ignoring build metadata |
| `>=1.2.3, <2.0.0` | Compatible 1.x releases starting at 1.2.3 |
| `=0.5.3-rc.1` | Only that prerelease, ignoring build metadata |

Empty/missing ranges, empty intervals, duplicate bounds, extra terms, wildcards,
bare versions, `^`, `~`, unions and unbounded intervals are rejected. For example
`>0.5.2, <0.5.3` contains no final release and is invalid.

Intervals exclude every prerelease. Prereleases are supported only as exact
declarations; prerelease interval endpoints are rejected. An exact prerelease
does not include the eventual final release. Build metadata in host versions
is validated but ignored for matching; authored ranges reject `+build` suffixes
because they cannot pin an artifact. Use full source SHAs and checksums for
artifact identity. Development builds report their actual package version;
there is no runtime version override or range bypass.

Crossing 1.0 requires an explicitly widened authored interval and acceptance on
both sides. Range grammar does not change at 1.0; recommended upper bounds do.

## Protocol, capabilities and features

Hello advertises `version` (wire protocol), `host_version` (Runyte release),
supported `capabilities`, `features`, and `limits`. Registration supplies
`runyte`, commands, `required_capabilities`, `optional_capabilities`,
`required_features` and `optional_features`. The acknowledged capability set
is the intersection with host support and user grants; all required grants must
be present. Features select understood wire behavior and cannot grant authority.

The host supports `view-row-actions` for row-specific action lists,
`view-action-presentation` for readable labels and grouped discovery,
`view-default-bindings` for configurable view-scoped default keys,
`view-metadata` for labelled values above content, and `view-document` for
complete bounded read-only documents. `job-feedback` adds bounded completion
messages for owned jobs without another capability. See the [application contract](applications.md#row-dependent-actions).
These remain additive features under `runyte-1`; the bundled frontend DTO
version 54 separately carries non-selectable heading rows. Supported features are selected
from the plugin's required/optional declarations; unavailable optional features
are omitted, unavailable required features fail. Each negotiation family is
bounded at 32 entries with no duplicates or overlap between required/optional
sets. Capability names use the command-ID spelling rules (48 bytes); feature
names use ASCII letters, digits, dots, underscores and hyphens (64 bytes).

The Python SDK exposes `host_version`, `granted_capabilities`, `features` and
`limits` after checking the acknowledgement. Protocol equality, release matching
and permissions/features are independent requirements.

## Additive changes and schemas

New hosts accept valid older stable messages with their original meanings and
defaults. Plugin envelopes and parameters remain strict. New clients omit
unsupported fields; adding a field to an old request is not safe simply because
the new host calls it optional. Never probe a mutation with speculative fields
or automatically retry a refused/uncertain mutation.

Host objects allow ignorable extra properties, including nested outputs; plugin
input retains strict validation. Empty `{}` acknowledgements remain exact in the
generic schema because results have no shared discriminator: clients correlate
each result with the method of its request. Extending an empty result therefore
requires a negotiated feature or a new method. Closed enum values, new host requests/events
and incompatible result shapes require explicit feature selection or a new
method that opts into the new response. They are withheld from old clients.
Unknown advertised feature names are ordinary bounded strings, not authority.
The stable fixture capacities and deadline allowances are minimum guarantees;
the SDK checks them in both hello and acknowledgement. Optional resource inventory,
when present, must supply its known fields with valid values. Clients retain
their own fixed framing bounds even when a host advertises larger capacities.
Limits and existing error meanings cannot silently change under an old profile.

The current [schema](runyte-1.schema.json), [fixtures](stable-fixtures.json) and
[shared range vectors](compatibility/ranges.json) are checked together. Frozen
stable clients, schemas and fixtures remain immutable compatibility evidence.
Schema validation establishes structure; host/worker and real-editor tests
establish ownership, effects, ordering, cancellation and cleanup.

## Client distribution

Python is the first maintained reusable SDK. Vendor `application.py` unchanged,
with full upstream source revision and file digests; copy matching schema and
fixtures for tests. Runtime and default tests need only the standard library.
Update deliberately after upstream review. Compatible host upgrades do not
require client updates. Newer SDK source revisions can support older stable
hosts, so vendor provenance and minimum host release are separate values.

Node, Rust and C have maintained independent examples and conformance checks.
They are not full packaged SDKs. Any language can implement the public protocol;
additional SDKs follow concrete plugin needs rather than being a prerequisite
for stable support.

## Release evidence

The [conformance guide](conformance.md) describes required checks. Runyte owns
within-range host regressions against frozen clients/plugins; plugin authors own
their minimum host declarations and plugin changes. A gate is not repaired by
widening an old fixture, replacing its client or making a required lane advisory.
Moving development branches supply separate advisory coverage.

During the initial transition, 0.3.0 candidate builds may be constructed from a
temporary source tree while the development package still says 0.2.4. Record
that construction explicitly. The actual release still requires green CI for
its exact version-only commit before publishing; candidate results cannot
substitute for that final check.
