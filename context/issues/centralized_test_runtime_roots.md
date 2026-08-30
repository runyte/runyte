# Short test runtime roots are duplicated and hardcode `/tmp`

Tests that start local persistent hosts need short Unix-domain socket paths.
Several helpers therefore construct roots directly below `/tmp`, including
code in `src/main.rs`, `src/workspace/catalog.rs`,
`tests/diagnostic_log.rs`, `tests/persistent_host.rs`,
`tests/local_protocol.rs`, and `tests/workspace_bulk.rs`. Related unit tests in
other modules have since added more copies of the same policy.

The copies differ in naming, canonicalization, collision handling, and
permission setup. They bypass `TMPDIR` without one shared statement of why the
platform temporary directory is unsuitable. macOS also advertises some
temporary paths through `/var` while canonicalizing them below `/private/var`,
so a fixture that does not canonicalize its root can compare two spellings of
the same directory.

One test-runtime-root policy should own:

- selection and canonicalization of a short temporary base;
- the measured Unix-socket path budget, including the endpoint suffix Runyte
  adds;
- collision-proof per-test names containing a short diagnostic label;
- owner-only permissions where a runtime directory requires them; and
- cleanup that cannot target another test's directory.

Unit and integration tests may need separate wrappers because integration
tests link a normally compiled library, but both should use the same policy
and naming rules. Environment-derived locations must remain injectable and
tests must not write to the repository, a person's configuration, platform
cache, or real persistent-session state. Process-global environment mutation
is not an acceptable way to isolate parallel tests.

Validation should cover a long advertised temporary directory, the macOS
`/var` versus `/private/var` alias, concurrent allocation, stale names left by
a reused PID, and the final socket path length on supported Unix platforms.
