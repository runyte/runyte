The plugin benchmark's process cleanup calls `os.pidfd_open` and
`signal.pidfd_send_signal` unconditionally. These attributes are absent from
some Linux Python builds, including the isolated CPython 3.12 interpreter used
for this review. Benchmark setup can consequently launch processes before
discovering that its identity-safe cleanup is unavailable.

The context-access benchmark inherits the same process cleanup but constructs
its own session. It also requires the capability check before writing grants
or launching its editor. Both command-line entry points should explain which
required interface is unavailable.

The corresponding mocked cleanup tests also require the real attributes to
exist. Running `python -m unittest discover -s benchmarks -v` under such an
interpreter fails three cleanup tests with `AttributeError: <module 'os'
(frozen)> does not have the attribute 'pidfd_open'` before their mocked behavior
runs.

Check required cleanup capabilities before launching benchmark processes and
report a clear unsupported-interpreter error. Preserve the existing rule that
cleanup signals pinned process handles, never replacement numeric PIDs. Mocked
unit tests should supply the interfaces they exercise independently of whether
the host interpreter exposes them. Cover refusal before subprocess admission.
