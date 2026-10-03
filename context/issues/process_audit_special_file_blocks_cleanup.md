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
