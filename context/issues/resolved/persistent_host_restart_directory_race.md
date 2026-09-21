---
title: "Persistent host restart can lose its endpoint directory before bind"
status: resolved
reported: 2026-09-21
resolved: 2026-09-21
commit: ccaeda6
---

## Resolution

`ccaeda6` (`Serialize endpoint directory preparation with retiring host cleanup`)
moves `LocalEndpoint::bind` directory preparation inside the stable workspace
identity lock. Previously, the retiring host's second cleanup could remove the
replacement's newly prepared but still empty endpoint directory before bind.
The same critical section now covers preparation, socket binding and metadata
publication as well as cleanup. The lock lives under a separate stable parent,
so preparing the endpoint directory does not become a lock prerequisite.

`retiring_host_cleanup_before_bind_lock_cannot_remove_the_replacement_directory`
in `src/workspace/transport.rs` forces the retiring server's second cleanup at
the former race boundary, then verifies replacement publication and a real
socket connection. The existing
`restart_keeps_a_fallback_host_on_its_original_endpoint` in
`tests/persistent_host.rs` covers native restart and endpoint preservation.
Linux and macOS lifecycle stress and both coverage suites pass at `ccaeda6`
in [CI run 35590400592](https://github.com/runyte/runyte/actions/runs/35590400592).
The separate Node conformance startup timeout in that run is recorded in
[`node_conformance_readiness.md`](../node_conformance_readiness.md).

## Report

The Linux lifecycle stress job in
[CI run 35589383537](https://github.com/runyte/runyte/actions/runs/35589383537)
failed `restart_keeps_a_fallback_host_on_its_original_endpoint` at commit
`f0d7c8c`. The restart command reported that the replacement host could not bind
its `.runyte/host/workspace.sock`, with `No such file or directory (os error 2)`.
The ordinary Linux gates and macOS lifecycle stress passed in the same run.

The test starts a fallback host without `XDG_RUNTIME_DIR`, rejects a duplicate
host using another runtime directory, then restarts the original host while
preserving its fallback endpoint. The replacement should publish that endpoint
successfully and accept the subsequent stop request.

`LocalEndpoint::bind` prepares its endpoint directory before acquiring the
stable workspace identity lock. `LocalEndpoint::cleanup` removes an empty
endpoint directory while holding that lock. Shutdown calls cleanup explicitly
and again through `LocalServer::drop`. A delayed cleanup can therefore remove
the directory after the replacement prepares it but before it acquires the
lock and binds its socket. The fixture deletes its project only after restart
and shutdown, so its final cleanup does not explain the bind failure.

Endpoint directory preparation, socket binding and publication must share the
same identity-lock critical section as cleanup. Cover the ordering directly;
retries and scheduler delays do not establish ownership.
