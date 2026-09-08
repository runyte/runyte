# Application conformance and release checks

Conformance checks a plugin's public messages and behavior under refusal,
cancellation and disconnect. A schema-valid message alone cannot establish that
the plugin owns a handle, observed the current revision or has foreground
authority. Use the [authoring guide](authoring.md) for installation and the
[application contract](applications.md) for operation semantics.

## Run checks without a TUI or account

From a clean checkout, create a disposable development environment:

```sh
plugin_checks="$(mktemp -d "${TMPDIR:-/tmp}/runyte-plugin-checks.XXXXXX")"
python3 -m venv "$plugin_checks/venv"
plugin_python="$plugin_checks/venv/bin/python"
"$plugin_python" -m pip install jsonschema==4.23.0
"$plugin_python" docs/plugins/check_schema.py
"$plugin_python" docs/plugins/check_applications.py
```

`check_schema.py` checks epoch 1 and its uppercase example.
`check_applications.py` checks epoch 2 fixtures and actual example processes,
including local file-manager requests and memory-provider messages. It supplies
public host messages itself; it does not connect to a running editor. Keep the
schema, SDK, examples and checks from the same checkout.

Run the remaining SDK/controller checks:

```sh
for check in jobs observations validation models queries processes handoffs activity state \
             remote_provider downloads uploads upload_review remote_status \
             remote_operations media media_review
do
  "$plugin_python" "docs/plugins/check_${check}.py"
done
```

The transport fixtures additionally need their pinned development dependencies:

```sh
"$plugin_python" -m pip install -r docs/plugins/requirements-sftp.txt \
  -r docs/plugins/requirements-ftp-test.txt
for check in sftp_ui sftp ftp_ui ftp
do
  "$plugin_python" "docs/plugins/check_${check}.py"
done
```

These fixtures create temporary keys, certificates, passwords and files and run
loopback servers. They need local socket access; they do not use personal SSH
configuration, service accounts or a live FTP server. Paramiko is a plugin-side
SFTP dependency. The shipped FTP/FTPS transport uses the Python standard library;
its test-server packages are development dependencies.

Install mpv separately, then run the real local-media checks. Null audio/video
outputs let them run without a display or audio device:

```sh
"$plugin_python" docs/plugins/check_mpv_backend.py
```

The checker uses `mpv` on `PATH`, or the `MPV` environment variable for an explicit
executable. It reports skipped real-backend cases if mpv is absent. A skip is not
evidence of playback, quietness or process cleanup. The Node example has its own
check and needs an installed Node.js runtime:

```sh
"$plugin_python" docs/plugins/check_node.py
```

The Node checker also accepts an explicit `NODE` executable and reports skips
when no runtime is available; record those skips rather than treating them as
another language's successful conformance run.

Once the Python dependencies, mpv and Node are installed, run every checked-in
`check_*.py` suite through the central entry point:

```sh
"$plugin_python" docs/plugins/check_all.py --require-backends
```

The runner discovers suites in filename order and stops at the first failure.
`--require-backends` refuses missing mpv or Node instead of accepting backend
skips. Without this option, individual suites report their own skips.

For changes to Runyte, run the host and worker tests as well:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo llvm-cov --locked --workspace
```

The [CI workflow](../../.github/workflows/ci.yml) is the executable list of
automated checks. The [coverage register](../../context/reference/test-coverage.md)
defines the canonical measurement and current floor. Python and Node execution
are not included in Rust line coverage. CI schedules the complete plugin runner
and canonical coverage on Linux and macOS; a configured job is not evidence
that its current run or native acceptance has passed.

## What each layer proves

| Layer | Checks | Does not establish |
| --- | --- | --- |
| JSON Schema Draft 2020-12 | Message direction, variants, required fields, value shapes and declared structural bounds | Stream framing, exact encoded byte limits, ownership, current revisions or successful side effects |
| Public-wire example tests | Handshake and correlation against deterministic host messages; concrete example behavior | An actual editor's admission, foreground rules or shutdown implementation |
| SDK/controller tests | Bounded dispatch, cancellation races, baseline ordering, staging and local state machines | Real network protocol behavior or native rendering |
| Real transport/helper fixtures | Local SSH/FTP/TLS/mpv behavior, binary bytes, protocol faults and child cleanup | Every server, codec, output device or operating system |
| Runyte host/App/worker tests | Ownership, quotas, revision fences, native input semantics, cleanup and editor-state effects | Every terminal backend or measured startup/idle performance |
| Native and performance acceptance | Actual attachment/input/rendering, process lifetime and measured activity | A portable result unless repeated on each required platform/backend |

Use the schema's `$defs.hostMessage` and `$defs.pluginMessage` when checking one
direction. The schema root accepts either direction. Every line still needs a
UTF-8 byte-length check including its newline; schema string lengths do not count
JSON escape expansion or guarantee a complete encoded frame fits. Implement
bounded decoding before retaining large containers, including staged model JSON.
Unknown plugin envelope fields and malformed structural messages can end a
connection; unsupported methods are ordinary errors.

## Conformance matrix

The following maps implemented groups to checked-in tests. File links identify
where behavior is exercised, not a claim that a particular machine has run it.
Companion review files add fault/race cases to several groups.

| Group | Public/example checks | Host or App behavior checks |
| --- | --- | --- |
| Epoch negotiation, IDs, registration and bounded worker delivery | [check_schema.py](check_schema.py), [check_applications.py](check_applications.py), [check_node.py](check_node.py) | [worker tests](../../src/plugin/tests/worker.rs), [application tests](../../src/workspace/host/tests/plugin_applications.rs) |
| Native views, staged models, patches and immutable reads | [check_models.py](check_models.py), [check_applications.py](check_applications.py) | [model tests](../../src/workspace/host/tests/plugin_models.rs), [model review](../../src/workspace/host/tests/plugin_model_review.rs) |
| Queries, viewport metadata and accepted actions | [check_queries.py](check_queries.py) | [query tests](../../src/workspace/host/tests/plugin_view_queries.rs), [query review](../../src/workspace/host/tests/plugin_view_query_review.rs) |
| Subscription baseline/order/coalescing/resynchronization | [check_observations.py](check_observations.py) | [host observations](../../src/workspace/host/tests/plugin_observations.rs), [registry tests](../../src/plugin/tests/observation.rs) |
| Text, snapshots and selections | [check_applications.py](check_applications.py) covers structural fixtures | [application tests](../../src/workspace/host/tests/plugin_applications.rs) exercise live targets and revision fences |
| Local document open/create/save/close | [check_applications.py](check_applications.py) | [documents](../../src/workspace/host/tests/plugin_documents.rs), [document jobs](../../src/workspace/host/tests/plugin_document_jobs.rs) |
| Prompt/pick/confirm/form and revision-bound validation | [check_validation.py](check_validation.py), file-manager wire case in [check_applications.py](check_applications.py) | [interaction](../../src/workspace/host/tests/plugin_interaction.rs), [validation](../../src/workspace/host/tests/plugin_validation.rs), [privacy/races](../../src/workspace/host/tests/plugin_validation_review.rs) |
| Local metadata and confirmed filesystem mutations | [check_applications.py](check_applications.py) | [stat](../../src/workspace/host/tests/plugin_filesystem_stat.rs), [recursive bounds](../../src/workspace/host/tests/plugin_filesystem_recursive.rs), [apply](../../src/workspace/host/tests/plugin_filesystem_apply.rs) |
| Sealed binary download publication | [check_downloads.py](check_downloads.py), actual adapter wire cases in [check_sftp.py](check_sftp.py) / [check_ftp.py](check_ftp.py) | [staging](../../src/workspace/host/tests/plugin_staging.rs), [private staging storage](../../src/plugin/tests/staging.rs) |
| Provider reads, conditional/confirmed writes and unknown outcomes | [check_remote_provider.py](check_remote_provider.py), memory-provider cases in [check_applications.py](check_applications.py) | [reads](../../src/workspace/host/tests/plugin_providers.rs), [writes](../../src/workspace/host/tests/plugin_provider_writes.rs), [weak overwrite](../../src/workspace/host/tests/plugin_provider_overwrite.rs) |
| Remote inspect, conservative rebind and native reload recovery | [check_applications.py](check_applications.py) validates inspection wire shapes | [inspect](../../src/workspace/host/tests/plugin_provider_inspect.rs), [rebind](../../src/workspace/host/tests/plugin_provider_rebind.rs), [recovery](../../src/workspace/host/tests/plugin_recovery.rs), [worker accounting](../../src/workspace/host/tests/plugin_recovery_review.rs) |
| SSH host verification, FTPS, remote namespace operations and upload promotion | [check_sftp.py](check_sftp.py), [check_ftp.py](check_ftp.py), [check_remote_operations.py](check_remote_operations.py), [check_uploads.py](check_uploads.py), [upload review](check_upload_review.py) | Native approval reuses existing interaction/job contracts; remote side effects stay plugin-side |
| Managed binary helpers and process-group lifetime | [check_processes.py](check_processes.py) | [host processes](../../src/workspace/host/tests/plugin_processes.rs), [runtime](../../src/plugin/tests/process_runtime.rs) |
| Terminal/system handoffs and notifications | [check_handoffs.py](check_handoffs.py) | [handoffs](../../src/workspace/host/tests/plugin_handoffs.rs), [notifications](../../src/workspace/host/tests/plugin_notifications.rs) |
| Finite jobs, activity leases and owner stop/restart | [check_jobs.py](check_jobs.py), [check_applications.py](check_applications.py), [check_activity.py](check_activity.py) | [activity](../../src/workspace/host/tests/plugin_activity.rs), [manager](../../src/workspace/host/tests/plugin_manager.rs), [worker-slot review](../../src/workspace/host/tests/plugin_manager_review.rs) |
| Settings schemas and conditional private state | [check_state.py](check_state.py) | [settings](../../src/workspace/host/tests/plugin_settings.rs), [state](../../src/workspace/host/tests/plugin_state.rs) |
| Media playlist, controls, cancellation and paused quietness | [check_mpv_backend.py](check_mpv_backend.py), [check_media.py](check_media.py), [media review](check_media_review.py) | Reuses managed-helper/activity contracts; real mpv remains an external dependency |

## Dynamic preconditions to test in an application

Exercise the success path and these refusals against a host or a faithful test
port. An all-valid-fixtures test alone is insufficient:

- Foreign, closed and old-generation handles must not affect live resources.
  Text/model/query/selection revisions have distinct meanings. Undo to identical
  text can still make a captured edit stale.
- Change focus, type, detach or close a target between preparation and completion.
  Background publication must not steal focus, and a fresh native confirmation
  must authorize the exact immutable candidate that will be applied.
- Cancel before an issued-handle response is processed, during work, and just
  before success delivery. Keep bounded early-cancellation records. Do not release
  a helper or memory reservation until its actual owner has finished with it.
- Fill queues and quotas, delay replies and stall readers. Refusals must preserve
  previous model/text/baseline, and cleanup must not wait behind unbounded output.
- Close the connection during a remote mutation. Distinguish a proven no-change
  rejection from an unknown outcome; restarting or reading matching bytes is not
  proof that a previous remote operation cannot commit later.
- Stop and restart owners while work is queued. Old callbacks must not publish
  into the replacement, replay commands or reuse its handles. Detach alone must
  not create a second worker, connection or player.
- Test secret input, malformed helper output and unusual Unicode. Secrets must
  not appear in models, snapshots, errors, state or protocol diagnostics.
- After a job, cancellation or pause settles, check that no periodic publication,
  renewal or polling continues without an active reason.

## Evidence and remaining release gates

Keep test results specific: checkout/commit, command, target OS/architecture,
dependency versions, passed/skipped cases and fixture type. Temporary native
smoke scripts or local logs are not reproducible conformance artifacts unless
their harness is checked in. The matrix above is a coverage map, not a release
certificate or a complete automated adversarial cross-product.

The [active plan](../../context/plans/active/PLAN_PLUGIN_APPLICATIONS.md) still
defines broader platform and performance acceptance. This guide does not claim
that macOS, every terminal backend, startup comparisons, paused/idle CPU, slow
consumer measurements or the full multi-application stress matrix have passed.
Run and record them on the specified targets before making that claim. Pure
controller tests and null-output mpv fixtures also do not establish audible
playback quality or behavior with every video/output device.

Spotify and YouTube service access is optional, account-dependent validation;
see the [service adapter guide](media-services.md). Passing local tests does not
grant API access, demonstrate live service playback or supply an inline video
renderer. Binary remote fixtures similarly do not make best-effort FTP/SFTP
checks into atomic compare-and-swap.
