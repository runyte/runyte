The baseline startup, idle, and persistent attachment benchmarks answer terminal
capability queries independently for each PTY read. PTY reads do not preserve
escape-sequence boundaries. A query split across two reads is therefore ignored,
which can stall an editor waiting for its answer and invalidate startup or quit
measurements.

For example, a child writes `ESC [` and then `6n` after a short delay, waits for
the cursor-position response, and only then writes its document marker. The
baseline harness never sends `ESC [ 1 ; 1 R`, never observes the document, and
times out. The readiness benchmark already retains a bounded stream suffix to
handle this case, but the other harnesses do not.

All PTY benchmark paths should retain incomplete capability queries across reads
and phase boundaries, answer each completed query once, and keep retained data
bounded. Ordinary document output must not cause duplicate answers.
