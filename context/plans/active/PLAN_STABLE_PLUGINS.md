# Stable plugin compatibility and Runyte release ranges

## Status and intended outcome

Active, 2026-09-15. Implementation authorized, including subagent code reviews.
The first stable target is 0.3.0. Release publication remains a separate action.
Follow [the plan lifecycle](../README.md).

## Implementation progress

- Host cutover implemented: bounded release parser, per-entry pre-launch admission,
  stable negotiation, required/optional features and grants, atomic registration,
  structured rejection and retained manager compatibility diagnostics.
- Removed the experimental wire routing. Ported detached edit/undo, worker,
  command, observer and persistent view/job tests to the stable boundary.
- Added native selection spans to `selection.get`, so the migrated uppercase
  example preserves Unicode, reverse, half-open and empty selection semantics.
- Migrated schemas, Python SDK, independent Node/Rust/C examples and benchmarks.
  Host output permits harmless additional fields; events remain discriminated,
  plugin input strict, and empty acknowledgements exact.
- ru-time now authors metadata once, emits it in config and registration, vendors
  the stable client/schema/vectors, and tests release boundaries and native
  cancellation/restart/rebind/detach behavior with isolated temporary storage.
- Added immutable inventories, retained artifact checks, required external/native
  CI cells and a separately advisory moving-head lane. Versioned candidate
  construction leaves the checkout at 0.2.4; first stable target remains 0.3.0.
- Exact-release CI, remote fetchability of the final local source pins, macOS
  execution and publication evidence remain release readiness work.
- Python is the first maintained reusable SDK, distributed as a vendorable
  standard-library file. Other languages receive protocol documentation,
  schemas, runnable examples and conformance checks; additional packaged SDKs
  are outside this transition.
- Independent host, SDK/schema, CI and external acceptance reviews are running;
  validation results and immutable source identities are recorded below before handoff.


Migration decision: the only current user is the maintainer, so the transition
does not need to preserve experimental plugins, clients or configuration.
Replace them in one coordinated cutover. Compatibility commitments and frozen
older-client coverage begin with the first stable release.

Inspected sources:

- Runyte `bb38b12254035a03e8e6bb53f0968950ff2f865b` (`Run ru-time's tests
  against each commit in CI`), package version **0.2.4**.
- ru-time `af60f201b9f73f601cd957598846294507bc4aa6` (`Run the real-editor
  test in CI against the pinned Runyte`).
- ru-time's `VENDOR.md` pins Runyte
  `a8273f65a15cd95bcda245eb80d333beb6b61570`; its Python client and schema
  are byte-identical to the inspected Runyte copies. The handshake fixture is
  adapted to ru-time's capabilities and command identity.

These are source and workflow inspections, not new CI execution results.
The repository instructions, README, plugin section of the user guide, context
index, release register, conformance and authoring guides, and completed minimal
plugin/application plans inform this proposal. Current implementation and public
documentation take precedence over those historical plans.

The outcome is an enforceable promise: a released plugin using the stable
contract continues to work on later compatible Runyte releases without replacing
its pinned client. An unsupported installation receives a specific diagnostic
before commands or resources become available, preferably before process launch.
The editor retains its responsiveness, bounded resource ownership and native
approval rules.

Scope excludes package installation, discovery, marketplaces, sandboxing, new
plugin runtimes, general editor automation, and a redesign of plugin methods or
ru-time storage. No deferred implementation issue is resumed by this plan.

## 1. Inspected pre-cutover behavior and the remaining gap

| Area | Inspected behavior and source |
| --- | --- |
| Configuration | `PluginConfig` in [src/plugin/epoch1.rs at the inspected revision](https://github.com/runyte/runyte/blob/bb38b12254035a03e8e6bb53f0968950ff2f865b/src/plugin/epoch1.rs) has an ID, explicit enablement, API, absolute executable, argument vector, grants, bindings and settings. It denies unknown fields. There is no release range, bundle manifest or discovery mechanism. `Api` in [application.rs](../../../src/plugin/application.rs) defaults to epoch 1; epoch 2 is explicit. |
| Launch | [plugin_manager.rs](../../../src/workspace/host/plugin_manager.rs), `initialize_plugin_manager`, `valid_config` and `launch_managed_plugin`, validate entries and spawn enabled programs. Hello is selected from the configured API. No host-release comparison occurs before spawning. Disabled entries launch nothing. |
| Handshake | Epoch 2 hello contains `version: runyte-experimental-2`, capabilities and limits. Registration repeats that exact epoch, commands and required/optional capabilities. Neither message declares a Runyte release. `application_message` in [plugin_applications.rs](../../../src/workspace/host/plugin_applications.rs) intersects requested capabilities with host support and configuration, requires all mandatory grants, and reuses atomic command/keymap installation. |
| Parsing | `application::decode` explicitly checks registration envelope field count. Known plugin parameters are strict; unknown methods return `unsupported`, while malformed envelopes terminate the connection. Simply adding registration metadata to the existing client would fail on older hosts. |
| Host schema evolution | [The epoch 2 schema](https://github.com/runyte/runyte/blob/bb38b12254035a03e8e6bb53f0968950ff2f865b/docs/plugins/runyte-experimental-2.schema.json) allows additions in many host objects, but still has closed message variants and enums; its shared error object disallows extra properties. Unknown-field tolerance alone cannot make arbitrary new host messages or enum values compatible with a pinned validator. |
| Client | [application.py](../../../docs/plugins/application.py), `Application.run`, checks the exact epoch, emits all supplied capabilities as required, and sends an empty optional set. It reads the registered acknowledgement's type but does not expose comprehensive negotiated release/feature state. Its separate reader, bounded output and cancellation workers are existing behavior to preserve. |
| Ownership | [plugins.rs](../../../src/workspace/host/plugins.rs), [worker.rs](../../../src/plugin/worker.rs), and the feature-specific host modules own cleanup, generations and results. Persistent-session detach retains workers; stop/restart retires handles and waits for cleanup. Private bundled frontend versions in `src/protocol/` are independent of plugin epochs. |
| ru-time | `ru_time/__main__.py::configuration` generates JSON/YAML configuration; `ru_time/plugin.py::TimePlugin` constructs the vendored client. Required grants are `views`, `interaction`, `activity`, `providers`, `documents`, and `jobs`. `VENDOR.md` explicitly warns that older epoch 2 hosts reject command aliases. Epoch equality therefore does not currently identify the minimum usable host. |

### Landed CI baseline

Build on these jobs; do not plan them as new work:

- Runyte's [CI workflow](../../../.github/workflows/ci.yml) has required Linux
  and macOS `plugin-conformance` jobs. They build/lint/test native todo variants,
  run `check_all.py --require-backends` with schema and local backend dependencies,
  and launch the examples in a real persistent host.
- [plugin_examples.rs](../../../tests/persistent_host/plugin_examples.rs),
  `every_example_plugin_registers_and_stays_running_in_a_real_host`, requires
  all 19 listed examples to report `Running` and still be running after a second.
  It includes an inventory check and host shutdown, but invokes no commands.
- Runyte's new `ru-time` job runs its full unittest suite with `RUNYTE_BIN` on
  Linux/macOS against the host built from the current commit. It has
  `continue-on-error: true` and checks out ru-time with **no pinned ref**.
- ru-time's `.github/workflows/tests.yml` retains its standard-library
  Linux/macOS × Python 3.10/3.14 matrix. Its new native job extracts the upstream
  SHA from `VENDOR.md`, builds that exact Runyte with `--locked`, and enables
  `RUNYTE_BIN` tests on both platforms using Python 3.14.
- ru-time's `tests/test_native.py::test_short_commands_and_space_pause_with_native_actions`
  already exercises commands, Enter/Tab actions, timer pause, durable multiline
  and empty note saves, deletion cancellation/acceptance and plugin stop. Its
  wire tests also cover stale commands, forced-stop checkpoint recovery, idle
  silence and note-provider IO. Controller tests cover cancellation without
  waiting on the UI lock. These are useful behavioral coverage, not merely
  registration checks.
- Runyte already tests revision fences, documents, observations, jobs, leases,
  provider writes and owner cleanup. The [conformance matrix](../../../docs/plugins/conformance.md#conformance-matrix)
  names those suites. `tests/persistent_host/plugin_stable.rs` also covers a
  view and finite job across repeated attachments using a checked-in stand-in.

The gap is **cross-version enforcement**: most fixtures and examples move with
the host; ru-time's pinned native host is a source revision rather than a release
range boundary; the host-side external job is advisory and mutable. Existing
behavioral tests need to participate in a compatibility matrix with frozen
clients and real host versions, with additional lifecycle/race scenarios where
that matrix currently has no coverage.

## 2. Recommended compatibility policy

### Release boundaries

Adopt the proposed pre-1.0 promise prospectively: within `0.X.Y`, increasing
`Y` preserves the stable plugin contract; increasing `X` may break it after an
explicit compatibility decision and migration notice. A plugin can therefore
declare `>=0.5.2, <0.6.0`. This is a Runyte-specific promise stronger than
SemVer's major-zero guarantee. After 1.0, adopt SemVer: new compatible API
functionality/deprecation requires a minor release, compatible fixes a patch,
and incompatible API changes a major release. SemVer also defines prerelease
ordering and excludes build metadata from precedence; it does not define this
plan's range language. See [SemVer 2.0.0](https://semver.org/spec/v2.0.0.html).

This fits [releasing.md](../../reference/releasing.md): routine releases
currently increment `0.2.Y`, and a minor change already requires an explicit
compatibility or scope decision. It adds a constraint on patch releases that
the experimental API has not previously promised. Do not retroactively label
all existing `0.2.Y` releases compatible.

Practical default: introduce stable support at the next explicitly selected
minor boundary, tentatively **0.3.0** if that is still the next line when this
work lands. The `0.5.2` examples below illustrate semantics, not a release
assignment. No existing-user compatibility obligation requires a new minor,
but a new minor makes the start of the promise easier to communicate and test.

The promise is backward compatibility: an older plugin runs on a newer host
inside its range. It does not promise that a plugin using a newly introduced
feature runs on hosts predating that feature. The plugin raises its lower
bound or implements an explicitly negotiated fallback.

### What compatible releases preserve

Freeze a public behavioral baseline in addition to wire shapes:

| Surface | Required preservation |
| --- | --- |
| Wire | UTF-8 NDJSON framing and bounds, message direction, required fields/defaults, integer and Unicode-scalar meanings, opaque handle/revision semantics, ID correlation and send ordering, refusal classification and no automatic replay. |
| Commands | Registration atomicity, local/scoped identity, typed arguments and contexts, alias collision behavior, configured bindings, native foreground grants and refusal after context changes. Optional new built-ins must not confiscate previously valid plugin names/bindings within the promise. |
| Documents and views | Explicit targets, revision fences, atomic edits and undo, stable row identity, no focus theft, clean/dirty and accepted-save semantics, conditional provider writes, retention on stop, conservative rebind and uncertain outcomes. |
| Events | Subscription scope, baseline-before-change, response-before-resulting-event ordering, reliable lifecycle transitions, permitted observation coalescing, explicit resynchronization and unsubscribe barriers. No new unrequested event stream. |
| Capabilities | Existing grant meanings and method requirements. A compatible host cannot start requiring a new permission for an already authorized operation or silently expand the meaning of an existing grant. |
| Lifecycle | One process per enabled owner, persistent-session detach/reattach behavior, connection-generation fencing, bounded cleanup, explicit restart and no replay, readable unavailable views and retained dirty documents. |
| Cancellation | Separate command/job lifetimes; idempotent requests; cooperative cleanup deadlines; terminal-state and late-success rejection; prompt response despite blocked normal IO; no claimed success for an uncertain mutation. |
| Limits and failures | Documented minimum capacities and deadline allowances, maximum framing rules, deterministic refusal versus disconnect, backpressure and isolation from the editor loop. Advertising a reduced capacity does not by itself make a regression compatible. |

All implemented epoch 2 operations should enter stable v1 only after their
current guarantees and deliberate limitations are audited. The default scope
is that whole public application surface, including provider guarantees, rather
than silently stabilizing only ru-time's subset. A method that cannot meet the
promise must be explicitly left out before v1 is frozen; experimental-only
operations must not leak into stable connections later.

Internal refactors, exact diagnostic prose, rendering pixels, and timing within
documented bounds are not frozen. Neither are private frontend DTOs or the
headless testing facade. Fixing behavior that violated the documented contract
is allowed, but requires an impact check against released plugins; an accidental
dependency should be diagnosed rather than erased by changing its fixture.
Security fixes require explicit compatibility review too. A necessary breaking
restriction uses the breaking release boundary, with a migration/advisory and
any justified backport decision recorded separately.

## 3. Metadata, configuration and admission

### One authored value, two checkpoints

Keep the plugin's supported range in ordinary plugin source/package metadata.
For ru-time add a small `ru_time/compatibility.py`, containing `API`,
`RUNYTE_RANGE`, and `CAPABILITIES`. Both configuration generation and application
construction import it. It must have no storage or process side effects.

No manifest loader is needed. Runyte cannot infer a Python script's identity
from the interpreter executable, and running `--print-config` is itself process
execution. The user runs that command during installation; Runyte never invokes
it for discovery or preflight. A future package format can carry the same value
without being a prerequisite for stable support.

Proposed generated configuration (paths are placeholders):

```yaml
plugins:
  - id: time
    enabled: true
    api: runyte-1
    runyte: ">=0.5.2, <0.6.0"
    executable: /absolute/path/to/python3
    args: [/absolute/path/to/ru-time/time_plugin.py]
    capabilities: [views, interaction, activity, providers, documents, jobs]
    bindings:
      open: Space = =
      add: Space = a
      pause: Space = p
      delete: Space = d
      note: Space = n
```

`runyte` is the required stable configuration admission range. Normally it is
an exact copy of the author's range. A user may narrow it, for example to
`>=0.5.3, <0.6.0`, without changing plugin code. It is neither an install request
nor proof that the executable still contains the version that generated it.

Before each launch, including restart, the owning workspace host checks its own
compiled release against the configured range, and validates the selected API
and ordinary configuration bounds. A mismatch leaves a failed manager entry
without spawning a worker. Do not compare the attached TUI version, a `runyte`
found on PATH, or the currently installed binary when an older persistent host
is still running. No new scan, subprocess probe or refresh timer is required.

After launch, registration supplies the **authoritative plugin-authored range**.
The host checks it before installing any command, binding, subscription or
resource. The SDK checks hello before registration as a helpful client-side
diagnostic; it cannot replace the host's enforcement.

### Precedence and disagreement

Let `C` be the configured range, `P` the process's registration range, and `H`
the running host version. Stable admission requires `H ∈ C`, `H ∈ P`, and
`C ⊆ P`, plus exact protocol and required capability/feature agreement.

| Situation | Result |
| --- | --- |
| Same normalized range | Admit if all other checks pass. |
| Configuration is a proper subset of the author's range | Admit within that subset; manager details identify the user restriction. |
| Configuration includes versions outside the author's range, even if this host is in both | Reject registration as stale/conflicting configuration; regenerate the entry or deliberately narrow it. Never let user configuration widen author support. |
| Either range excludes the host | Reject at the earliest available checkpoint. |
| Stable range missing, empty or malformed | Configuration fails before spawn, or registration fails atomically if the missing value is in the process message. No implicit wildcard. |
| Experimental API or old configuration without stable metadata | Reject; regenerate configuration and update the plugin as part of the cutover. No legacy admission path. |
| Configured API and registration version disagree | Reject; never silently switch protocols or relaunch to try another epoch. |

Compare normalized interval semantics, not whitespace. Requiring the subset
relation is stricter than merely intersecting declarations: it catches an old
generated entry after a plugin raises its minimum. The cost is an intentional
configuration refresh even when the current host happens to satisfy both.

Use bounded structured reasons and retained manager diagnostics, for example:

```text
Plugin time not started: host 0.6.0 is outside configured Runyte range >=0.5.2, <0.6.0.
Plugin time registration refused: configured range >=0.5.2, <0.6.0 exceeds plugin range >=0.5.3, <0.6.0. Regenerate the plugin entry or narrow its range.
Plugin time registration refused: required capability documents is not granted in configuration.
```

Distinguish malformed metadata, unsupported release, protocol mismatch,
unsupported host capability/feature and missing user grant. Show host version,
protocol and bounded normalized declarations, not arguments/settings or raw
stdout/stderr. Stable v1 should define a bounded `registration_error` reply
with a fixed reason code and message before cleanup when delivery is possible;
the manager retains the reason even if delivery fails. Existing generic cleanup
feedback must not overwrite that reason. Refusal still uses the normal bounded
worker stop/reap path.

## 4. Minimal range language

Support exactly one interval or one exact version:

- One lower comparator (`>=` or `>`) and one upper comparator (`<` or `<=`),
  separated by a comma, in either order; conjunction, never union.
- Or a single `=MAJOR.MINOR.PATCH`, including an explicit prerelease suffix
  for testing. Three numeric components are mandatory; no leading zeros.
- ASCII spaces/tabs around tokens are insignificant. Reject embedded line
  breaks, empty terms, duplicate lower/upper bounds, extra terms and empty
  intervals. Normalize to lower bound then upper bound.
- Require both bounds for an interval. Reject `^`, `~`, wildcards, bare versions,
  `||`, hyphen ranges, implicit comparator lists and unbounded `>=0.5.2`.
  Keep explicit bounds when crossing release-policy boundaries.
- Limit declarations to 256 UTF-8 bytes; use checked numeric parsing with the
  same documented bound in Rust and Python (recommend unsigned 64-bit core
  components). Reject overflow. Validate nonempty intervals over the supported
  discrete release domain, not just lexicographic endpoint order.

| Declaration | Meaning/result |
| --- | --- |
| `>=0.5.2, <0.6.0` | Includes 0.5.2 and later 0.5 patches; rejects 0.5.1 and 0.6.0. |
| `>0.5.2, <=0.5.4` | Includes 0.5.3 and 0.5.4, excludes 0.5.2 and 0.5.5. |
| `=0.5.2` | Exact release precedence; useful for diagnosis, discouraged as the normal compatibility promise. |
| `>=1.2.3, <2.0.0` | Includes later 1.x minor and patch releases, excludes 2.0.0. |
| `>=0.9.4, <2.0.0` | Explicitly claims compatibility across 1.0; accept syntax, but author must test both sides. Never infer this widening. |
| `>=0.5.4, <0.5.4` or `>0.5.2, <0.5.3` | Empty interval; reject. |
| `0.5`, `^0.5.2`, `>=0.5.2,`, or absent stable value | Reject with a syntax/missing-value diagnostic. |

**Prereleases:** ordinary intervals contain only final releases. They exclude
`0.5.3-rc.1` even though its ordering falls inside some interval. Initially allow
prereleases only with exact `=0.5.3-rc.1`; reject prerelease endpoints in two-bound
intervals. That provides an explicit test opt-in without copying npm/Cargo/Python
range conventions. It does not opt into the eventual `0.5.3` final release.
Use separate temporary test configurations and test-plugin metadata for candidate
versions; never widen released plugin artifacts during compatibility testing.

**Development builds:** expose `env!("CARGO_PKG_VERSION")` as the host release.
A build labeled `0.5.3-dev.1` follows the exact-prerelease rule. Build metadata
such as `+git.abc123` does not affect matching; `=0.5.3` therefore also matches
`0.5.3+git.abc123`. Do not accept `+build` suffixes in authored comparators, since
they misleadingly suggest artifact pinning. Use full Git SHAs/checksums for
that separate purpose. A checkout still labeled `0.5.2` is indistinguishable
from that release by version alone and receives only development test evidence,
not release certification. Do not infer provenance from debug/release mode.

Range matching needs no network or Python package. Share a bounded JSON vector
corpus between the Rust host and standard-library Python client. Keep it separate
from JSON Schema: schema validation cannot establish interval containment or
whether a running host matches. At 1.0 the parser stays unchanged; documented
authoring defaults change from `<0.(X+1).0` to `<(MAJOR+1).0.0`.

## 5. Protocol, capabilities and compatible additions

### Use a distinct stable identifier

Introduce **`runyte-1`** as the stable application wire protocol, derived from
the audited epoch 2 behavior. Keep `version` as the protocol field to minimize
gratuitous naming changes; use a separate `host_version` for the Runyte release.
Do not call it `runyte-experimental-2`, alias that name to stable v1, or require
the wire version to change with every Runyte minor/patch release.

A new identifier marks the start of the compatibility promise and its mandatory
range metadata, structured handshake errors and extension negotiation. Breaking
experimental clients is intentional; a distinct name prevents confusing their
contract with the new stable one. Stable v1 can span multiple compatible release
lines, including 1.0. After that initial cutover, a breaking wire/semantic change
needs a new protocol identifier and an allowed breaking Runyte release boundary.
There is no experimental adapter or fallback in the stable implementation.

Three independent conditions answer different questions:

1. **Release range:** has the plugin author committed to this Runyte release?
2. **Protocol:** can both sides understand the same messages and semantics?
3. **Capabilities/features:** does the host implement the needed functionality,
   and has the user granted the permissions it requires?

Keep current required/optional capability negotiation. Add bounded
`features` advertisement plus `required_features`/`optional_features` registration
and an acknowledged selected `features` set to stable v1 from its first release.
Features identify optional wire/behavior extensions, not new authority. They
cannot grant a capability. Existing v1 functionality requires no feature flags;
initial feature sets can be empty. Limit each negotiation set and token length
explicitly (default 32 tokens, 64 ASCII bytes each), reject duplicates, and
never make a new feature mandatory for an old registration.

The following are **proposed handshake excerpts**; hello and registered also
carry the complete existing limits inventory. Registration below illustrates one
ru-time command; the actual plugin sends its complete command list atomically.

```json
{"type":"hello","version":"runyte-1","host_version":"0.5.3","capabilities":["views","interaction","activity","providers","documents","jobs"],"features":[]}
{"type":"register","version":"runyte-1","runyte":">=0.5.2, <0.6.0","name":"Time","commands":[{"name":"open","alias":"time","description":"Open time tracker","context":"workspace"}],"required_capabilities":["views","interaction","activity","providers","documents","jobs"],"optional_capabilities":[],"required_features":[],"optional_features":[]}
{"type":"registered","commands":["plugin.time.open"],"capabilities":["views","interaction","activity","providers","documents","jobs"],"features":[],"runyte":">=0.5.2, <0.6.0"}
```

The registered `runyte` value is the effective configured restriction after
containment validation. Host capability advertisement lists support, while the
registered set lists actual grants. Stable SDKs retain hello/registered data,
verify required grants/features and expose the selected sets and limits.
An unknown required feature fails registration; an unknown optional feature is
omitted. Extra fields, methods or behavior belonging to a feature are enabled
only when selected. Existing capabilities alone are too coarse to establish
that a newer optional parameter is understood.

### Rules for additions and old schemas

- New hosts continue accepting all valid old client messages. New plugin fields
  need omission defaults that preserve old behavior; plugin envelopes/known
  parameters remain strict so misspellings do not become silent no-ops.
- New clients omit extension fields unless supported and selected, or raise
  their minimum release. A new parameter under an existing method is not safe
  merely because it is called optional: an older strict host may disconnect.
  Do not probe mutations by sending speculative fields or retrying after errors.
- Make stable host objects extensible for ignorable properties, including nested
  result/error objects, in both the published schema and reference decoders.
  Preserve required members, defaults and old value constraints. Explicitly test
  older independent language clients, not just Python dictionary access.
- A new event type, host request, required response shape or value in a closed
  enum must be withheld from old unopted clients. Use a selected feature (or a
  new method whose request opts into its result) and a matching feature schema.
  Frozen base schemas must still validate the traffic sent to base-only clients.
  New failure codes for old operations require the same care: map to existing
  meanings or negotiate them; do not turn every old error into `internal`.
- Feature advertisements and selection use open bounded string sets so old
  schemas can accept unknown advertised names without claiming to understand
  their semantics. Never emit unknown mandatory operations to those clients.
- Keep immutable schemas/fixtures at released commits, with a stable v1 schema
  identity and documented release-specific source URLs. Compatible changes to
  the current schema do not replace archived snapshots used by CI. Differential
  checks compare accepted old plugin messages and new host output against old
  schemas for each negotiated profile, rather than comparing schema text alone.

This feature set adds a small negotiation surface. Relying only on release
minimums is simpler but forces plugins to abandon older hosts even for optional
enhancements; permissive unknown-field decoding alone cannot protect old enum
validators or prevent unsolicited host requests.

## 6. Coordinated cutover

1. Replace both experimental protocols with stable v1. Require explicit
   `api: runyte-1` and `runyte` in plugin configuration; remove the implicit
   epoch 1 default. Old entries must be regenerated. No coexistence period,
   deprecation window, automatic conversion or legacy maintenance branch is
   needed.
2. Remove experimental-only decoders, routing and SDK modes. Extract shared
   configuration/worker values currently housed in `src/plugin/epoch1.rs` before
   retiring that module; retain editor behavior and ownership machinery needed
   by the stable API. Replace active schemas/fixtures and checks with stable
   versions. Git history preserves historical contracts; completed plans remain
   historical records rather than maintained legacy API documentation.
3. Update the public Python client directly for stable registration, range
   enforcement and optional capability/feature handling. Its constructor may
   change during this cutover; it needs no legacy mode. Never hardcode ru-time's
   range into the generic client. Preserve the standard-library-only runtime.
   Align independent Node, C and Rust implementations and their fixtures.
4. Migrate every runnable example and configuration generator to stable v1,
   including rewriting the uppercase example around stable buffer reads/edits.
   Port useful experimental tests to stable messages instead of discarding
   behavioral coverage. Old-epoch refusal tests need only prove rejection;
   no old client needs to remain operational.
5. In ru-time, copy the upstream client and stable schema together, unmodified,
   refresh `tests/host_handshake.json`, and update `VENDOR.md` with the full source
   SHA, file digests, license and migration note. Record the vendor source SHA
   separately from the supported host lower bound and native test host pins:
   a newer SDK can intentionally support an older stable host.
6. ru-time's `--print-config` and registration use the same authored constants.
   Document copying the generated entry while preserving user paths, settings,
   bindings, configured ID and database arguments. A persistent host must restart
   to load changed configuration; `:plugin-restart time` alone uses the previously
   loaded entry. No task database/export migration is needed for this transition.
7. Coordinate the host, example, ru-time and configuration updates before release.
   Pin ru-time's native CI to the new stable-capable host revision as part of
   that update. The current experimental pin is source evidence, not a host that
   the migrated plugin must support. Mixed old/new installations are unsupported.
8. Freeze and publish the first stable ru-time artifact with its minimum stable
   host. From this baseline forward, later compatible host releases must run
   its unchanged client. This prospective obligation does not apply to any
   existing experimental artifact.

Hosts predating this work do not understand `api: runyte-1` or the new config
field and cannot provide its improved diagnostics. Installation instructions
must name the minimum Runyte release and require upgrading/restarting the host
before installing a stable entry. An old running persistent host does not gain
stable support merely because its executable on disk has been updated.

## 7. Compatibility CI and release gates

### Required versus advisory coverage

| Lane | Required acceptance |
| --- | --- |
| Existing Runyte gates | Retain formatting, Clippy, Rust tests, Linux/macOS canonical coverage at the current 89% floor, lifecycle/performance jobs, and complete plugin-conformance including real-host example startup. |
| Stable contract gate in Runyte | Every applicable PR and the exact release commit run range/negotiation vectors, old-schema/decoder compatibility, and current real-host behavioral tests on Linux/macOS. No silent skips. |
| Frozen external gate in Runyte | Run immutable released ru-time/plugin artifacts and their pinned clients against the new host on Linux/macOS. Remove advisory status from this new stable lane. A stable candidate whose version falls inside their declaration must pass. |
| ru-time default tests | Keep Linux/macOS × Python 3.10/3.14 standard-library tests unchanged in dependency policy. Test generated metadata and handshake consistency here. |
| ru-time stable host matrix | Required native checks against the exact oldest supported release and a pinned newest compatible released host on Linux/macOS. Use Python 3.10 at the floor and 3.14 at the newest host; retain broader interpreter coverage in the default matrix. Run both interpreters at the floor when stabilizing the first release. |
| Boundary/refusal gate | Test below-minimum, included minimum, a later compatible patch, excluded next minor/major, and all syntax/prerelease/config disagreement cases. Cover pre-spawn rejection, registration rejection and successful cleanup. A correct unsupported-version rejection is a pass in an explicitly named rejection scenario. |
| Moving ecosystem coverage | Preserve Runyte → ru-time default-branch checks as advisory early warning, now clearly separate from the frozen required lane. Add scheduled/dispatch ru-time → Runyte dev checks and automated proposals to advance release pins. |
| Optional expansion | Scheduled intermediate releases, wider architectures/interpreter cross-products and long stress runs supplement required native Linux/macOS gates. Live remote services/accounts remain optional; existing isolated local backend checks remain required. |

Freeze at least the first stable client from each supported release line and
every later released client profile introducing a method/feature/schema
distinction; include independent Node/C/Rust decoders. Execute all retained
profiles on the candidate host. A registry row records why it is retained and
which versions are expected to accept it. Updating the current SDK or ru-time
must **add** a profile when needed, not replace the old evidence.

Experimental plugins and clients are not retained as compatibility targets.
The inspected ru-time revision supplies behavior to preserve during its port;
its wire messages and client may change freely before the stable baseline.
The first stable release must freeze a reviewed stable candidate fixture, run it before
publishing Runyte, then publish the matching ru-time release and retain the same
bytes. Resolve this bootstrap pin before declaring later patch guarantees.

At that first release, the oldest and newest supported stable host are the same
candidate. Exercise range boundaries with injected-version host tests and stable
protocol fixtures; do not require nonexistent earlier stable releases. Expand
the real released-host matrix as compatible releases become available. Migrate
the landed CI jobs' fixtures and pins to stable support while retaining their
platform coverage and behavioral checks.

On a breaking host boundary, old ranges are expected to reject; label those
matrix cells as rejection tests and also require a migrated plugin to succeed
on the new host. Never turn an unexpected registration failure into a passing
test just because the process exits. Within a supported range, neither widening
the fixture's range nor upgrading its client can repair the gate.

### Behavioral coverage to promote or add

Use existing tests as the base, then exercise these scenarios with an actual
host and a pinned client. Schema transcripts alone cannot certify them.

| Boundary | Evidence required beyond successful registration |
| --- | --- |
| Commands | Invoke registered full names and aliases, typed arguments and native view actions; assert editor state/durable effect and correlation. Verify collision handling, stale context, invalid arguments, refusal atomicity and no focus theft. Reuse ru-time's existing native flow. |
| Text/documents | Unicode scalar reads/edits, undo, stale/foreign/closed handles, view revisions, local open/save/close jobs and provider note round trips. Extend real-host ru-time coverage for edit-during-save, conflict, deleted-task save refusal, restart/rebind and uncertain completion without replay. Reuse Runyte's document/provider suites for broad behavior outside ru-time's scope. |
| Events | Assert subscription baseline/order, response before causal event, coalescing, overflow/resync, unsubscribe barrier and no events after owner retirement. Add a public-client probe over real host subscriptions; also verify ru-time visible/hidden viewport refresh behavior without relying on exact redraw counts. |
| Cancellation | Cancel a finite job and a running ru-time activity via native host input; assert terminal cancellation, persisted paused timer, released shutdown protection and no subsequent success. Exercise early cancellation, blocked ordinary request IO, expiry and unresponsive-owner cleanup using deterministic host/worker fixtures. |
| Persistent lifecycle | Add real ru-time detach/reattach with a running timer and editable note, unchanged process ownership, monotonic elapsed time, explicit stop/restart, retired handles and no replay. Keep the existing stand-in attachment test as fast focused coverage. |
| Shutdown/failure | Normal idle shutdown and child reaping; protected quit while work is live; cancellation before shutdown; force-stop checkpoint preservation and explicit recovery; retained unavailable views and unsaved note text. EOF must not imply a final response or durable success for a killed process. |
| Evolution | Old plugin frames accepted by new host; new host frames accepted by frozen schemas/decoders when no extensions selected; new client omits unsupported optional fields on the oldest host; required unavailable feature fails before commands install. |

Keep required real-editor tests semantic and bounded: await observable state
changes and deadlines rather than adding fixed sleeps for each step. Cross-repo
tests use the public plugin wire and PTY input. Runyte's own persistent-host tests
may keep their existing bundled-client harness; do not export that private
transport to ru-time as a new dependency.

Use temporary workspace/configuration/database/runtime/cache/data/state roots,
temporary local server credentials and process-group cleanup on failures. Keep
ru-time's default tests independent of `jsonschema`; optional schema validation
runs in an isolated development venv/CI job. Native checks remain opt-in locally
through `RUNYTE_BIN`, but required CI must assert that they actually ran.
Rust launch fixtures use the checked-in `src/fixtures/stand-in` and behavior data,
never a newly written executable. Compiled example programs remain ordinary
build artifacts.

### Reproducible references and ownership

Add a small **test inventory**, not a plugin installation manifest, at
`docs/plugins/compatibility/hosts-and-clients.json` in Runyte and
`tests/runyte-hosts.json` in ru-time. Entries record repository URL, full commit
SHA, human release tag/version, client/schema digest, declared host range,
protocol/features, scenario and expected accept/reject result. Host binaries
come from verified release archive digests or exact-SHA `cargo build --locked`.
Commit all matrix pins before the version-only release commit.

Require that fetching a tag resolves to the recorded SHA, that vendored files
match their recorded upstream bytes, and that reported host version agrees with
the inventory. Cache keys include these identities. CI reports both SHAs,
platform, interpreter and expected outcome; retain bounded sanitized diagnostics,
not unrestricted protocol bodies. Unavailable downloads or skipped native tests
are infrastructure failures, not compatibility success.

Runyte maintainers own a new-host regression against frozen supported plugins;
ru-time maintainers own a plugin change failing its oldest supported host or
incorrect generated metadata. Client/schema defects belong upstream in Runyte
and are re-vendored deliberately. Pin updates require ordinary reviewed changes
and explanations. Keep optional moving-head jobs independent so a mutable
external branch cannot authorize or unexpectedly block an immutable release.

Update [the release runbook](../../reference/releasing.md) in a normal
implementation commit: require the compatibility matrix in exact-release-commit
CI before crates.io publish, and confirm candidate package version is what the
gate exercised. Preserve the existing two-file version commit, main/dev flow,
`--locked`, main-push-before-publish, and tag order. Retain exact-tag binary smoke
checks; do not defer the only compatibility check until after irreversible
publication. Scheduled green runs or an earlier commit never substitute for
the exact release gate.

## 8. Stages, affected files and acceptance

Each stage is independently reviewable. Stage completion records actual results
and unresolved failures in this plan; only the final acceptance moves it to
`completed/`.

### Stage A — approve and freeze the promise

**Runyte:** this plan; new `context/reference/plugin-compatibility.md`;
`context/reference/releasing.md`; `docs/plugins/conformance.md`; frozen baseline
inventory under `docs/plugins/compatibility/`.
**ru-time:** `VENDOR.md`, README and proposed `tests/runyte-hosts.json`.

- Decide first stable release, protocol name, scope and range grammar. The
  clean experimental cutover is already decided. Audit method/event/error/limit guarantees against
  their host implementations and existing test matrix.
- Identify the first stable candidate profiles to freeze;
  record hashes and ownership. Preserve the newly landed CI baseline.

**Acceptance:** a release-by-release accept/reject table and gap-to-test mapping
exist; no current behavior is called stable yet. Validate all source references,
hashes and inventory parsing; document any operation withheld from stable v1.

### Stage B — range parser and pre-launch configuration admission

**Runyte:** new `src/plugin/compatibility.rs` and standalone test module;
`src/plugin.rs`, `src/plugin/epoch1.rs` (move `PluginConfig` into an appropriate
shared configuration module as the experimental module is retired),
`src/workspace/host/plugin_manager.rs`, manager tests, config tests and
`config.example.yaml`. Add shared vectors in `docs/plugins/compatibility/`.
**ru-time:** no runtime change required; reuse vectors during Stage D.

- Implement bounded parse/normalize/contains/subset checks; host identity comes
  from its package version. Define required stable config metadata, including
  explicit API selection, and remove implicit epoch 1 admission with Stage C.
- Retain specific entry-local diagnostics; invalid plugin ranges must not reset
  unrelated editor configuration. Perform semantic validation in manager
  admission rather than making a range error fatal to the whole config file.

**Acceptance:** vectors cover all operators, inclusivity, empty/discrete ranges,
overflow, prereleases/build metadata, normalization and containment. Worker-spawn
instrumentation proves mismatch/disabled entries create no child; restart uses
the same gate. Other valid plugins still start. No scan/timer is introduced.

### Stage C — stable handshake and evolution rules

**Runyte:** `src/plugin/application.rs`, `worker.rs`, `state/wire.rs` as needed
for strict preflight, `src/workspace/host/plugin_applications.rs`,
`plugin_manager.rs`, `plugins.rs`, `src/app/plugin_workflows.rs` and existing
API-dispatch sites; new stable schema/fixtures; host/worker/manager tests.
**ru-time:** no vendored edits until the upstream interface is coherent.

- Replace experimental routing and `Api::Epoch2` conditionals in queue sizing,
  decoding, invocation and manager paths with the stable application path.
  Remove obsolete epoch 1 wire values after extracting shared worker machinery.
- Implement both range checkpoints, feature selection, structured rejection and
  persistent diagnostic retention. Make registration atomic on every failure.
- Audit stable host decoders/schema extension points; test the initial stable
  profile and negotiated additions with a synthetic feature.

**Acceptance:** valid stable handshake succeeds; missing/conflicting fields,
unsupported releases/protocols/features and absent grants refuse deterministically
with no installed commands or leaked owner. Experimental API selections and
missing stable metadata are refused. Initial stable profiles accept all
unextended host traffic, and timeout/EOF/reaping
checks show no editor stall. Registration-error delivery is bounded best effort.

### Stage D — public clients, examples and ru-time migration

**Runyte:** `docs/plugins/application.py`, new stable schema/fixtures, existing
`check_*.py` suites and migrated `uppercase.py`; `tasks.mjs`, todo Python/Rust/C
sources/build helper; `docs/plugins.md`, `applications.md`, `authoring.md`,
`conformance.md`, README, `docs/user-guide.md`, `config.example.yaml`.
**ru-time:** new `ru_time/compatibility.py`; `ru_time/__main__.py`,
`ru_time/plugin.py`; `vendor/application.py`; stable schema and handshake/vector
fixtures; `VENDOR.md`, README, `tests/test_cli.py`, `test_wire.py`, `test_native.py`.

- Expose negotiated client state, replace the experimental constructor, and make
  range parsing dependency-free. Add optional feature/capability handling.
- Vendor from an immutable upstream commit without local SDK patches; author
  the ru-time support floor from actual oldest-host acceptance, not the vendor
  copy date. Keep database behavior and configured bindings intact.

**Acceptance:** generated configuration and registration declarations agree;
custom plugin ID/database options survive; no install/config generation opens
storage. All examples use stable v1 and work against the new host; their
existing behavior coverage is retained. ru-time passes `python3 -m unittest discover -s tests -v`,
optional isolated schema validation and native checks on the supported floor
and newest host. No runtime/default-test package is added.

### Stage E — required compatibility matrix and missing behavior

**Runyte:** `.github/workflows/ci.yml`, compatibility inventory/runner,
`docs/plugins/conformance.md`, `tests/persistent_host/plugin_examples.rs`,
`plugin_stable.rs` (ported from experimental coverage) and new stable behavior module; existing host/application,
document, observation, activity and manager test suites.
**ru-time:** `.github/workflows/tests.yml`, `tests/runyte-hosts.json`,
`test_native.py`, `test_wire.py`, `test_plugin.py`, `test_notes.py` and new
metadata/compatibility tests where needed.

- Preserve existing startup checks; add the frozen required lane and missing
  real-host scenarios in section 7. Decouple native host pins from vendor
  provenance while retaining deliberate pinned-source acceptance if useful.
- Verify boundary rejections, same-range success and oldest-host optional-feature
  fallback. Require native tests to run in CI, and retain moving-head jobs as
  advisory/scheduled coverage.

**Acceptance:** demonstrate the gate fails for an intentionally incompatible
host fixture, missing native dependency and bad pin, without committing those
faults. Full Linux/macOS required matrix succeeds with recorded immutable refs.
No tests write personal data or depend on sibling checkout paths/network accounts.

### Stage F — release readiness and migration publication

**Runyte:** finalized compatibility reference, release runbook, public guides and
CI inventory, all merged before the eventual `Cargo.toml`/`Cargo.lock` release
commit. **ru-time:** released stable tag, finalized support declaration,
provenance and host pins forming the first compatibility baseline.

- Review all unresolved decisions below, document the one-time cutover, and verify
  both repositories from clean checkouts with independently fetched pins.
- Execute the unchanged release sequence plus its new compatibility gates only
  when a release is separately requested. This implementation does not publish, bump the checkout package version,
  tag releases or increase CI permissions. Publication remains separately authorized.

**Acceptance:** exact candidate CI has no failed/skipped required compatibility
cells on Linux/macOS; release notes state stable support's starting version and
range/config migration. Frozen artifacts and public documentation agree. Record
the actual run IDs/SHAs before marking this plan completed.

For Rust implementation validation run `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, and canonical
`cargo llvm-cov --locked --workspace` on affected first-class targets, preserving
the coverage floor. Run the existing full plugin-conformance checks at client,
schema, host-wire and release boundaries. Validate startup/idle impact using
the existing benchmark register/harness when launch ordering changes; disabled
plugins must remain free of discovery/periodic work. Documentation-only planning
validation checks references, example syntax and diff hygiene rather than
claiming implementation tests passed.

## 9. Decisions and residual risks

| Maintainer decision | Recommended default / risk |
| --- | --- |
| First stable release | Decided for this implementation: 0.3.0. Package bump/publication is separate; 0.5 examples remain illustrative. |
| Stable surface | Audit and stabilize the existing public application surface. Provider outcome/approval semantics and limits are substantial obligations; unresolved groups must be explicitly excluded before v1 freezes. |
| Protocol spelling and evolution | `runyte-1`, distinct from both experimental epochs; small negotiated feature sets for later extensions. Avoid a general protocol resolver. |
| Range disagreements | Allow configured narrowing; reject widening/stale declarations even if this host lies in both. This adds a regeneration step but makes the pre-launch metadata meaningful. |
| Prerelease/development identification | Exact prerelease opt-in; no version-check bypass or runtime Git scan. Unlabeled development builds cannot be distinguished from releases; deciding to label dev packages differently is separate release-workflow work. |
| Experimental migration — decided | Remove both experimental paths at stable cutover; regenerate the maintainer's configuration and port ru-time/examples together. No existing-client compatibility obligation. |
| Frozen-client inventory growth | Retain first stable clients and every distinct released feature/schema profile for supported lines. Removal needs a documented support-boundary decision; repeatedly updating one latest pin would erase the guarantee. |
| Test artifact/release bootstrap | Agree who publishes and retains ru-time releases and full-SHA fixtures before the first stable gate becomes required. Build network failures must remain distinguishable from API failures. |
| Required external failures | Runyte owns within-range host regressions. Never bypass the promise by raising ru-time's minimum or making the frozen lane advisory; infrastructure failures still block publication until resolved. |

Passing a finite matrix cannot prove every third-party plugin compatible. The
promise rests on the audited contract, additive-change rules, frozen behavioral
evidence and ownership of regressions together. It does not warrant correctness
of arbitrary plugin code, external services or a modified binary with a reused
release number.
