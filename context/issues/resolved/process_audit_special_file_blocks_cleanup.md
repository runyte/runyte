---
title: "Process audit storage can block cleanup signals"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 1da9d33
---

## Resolution

Commit `1da9d33` (`fix(process): keep audit files from blocking cleanup signals`).

append_record now opens Unix audit destinations nonblocking and validates the descriptor as a regular file before writing. Audit failures remain best effort, so a FIFO cannot prevent the subsequent process operation or cleanup signal. Existing append semantics and the per-process byte budget remain intact.

Coverage: special_audit_destination_cannot_block_process_operations and its owned special_audit_fixture in src/process_group.rs exercise audited spawn and signal delivery with a temporary FIFO and a special device. The bounded subprocess timed out before the fix; all 10 process_group tests pass, with the owned fixture ignored when run directly.

## Report

The optional `RUNYTE_PROCESS_AUDIT` journal opens its destination with a
blocking append-only `OpenOptions` call. Pointing it to a named pipe without a
reader blocks `record_spawn` or `OwnedGroup::signal_with` before the process
operation can proceed. In particular, the diagnostic record can prevent a
cleanup signal from being delivered, contradicting the journal's best-effort
contract.

Use a temporary FIFO as the audit destination and invoke an audited process
operation in a bounded subprocess. Audit storage failures and special objects
must be ignored promptly, while regular-file journals retain append behavior
and the per-process byte budget. Validate the opened descriptor before writing;
checking only the pathname before a blocking open is insufficient.
