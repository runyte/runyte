# Subprocess tests load the developer's Runyte configuration

`tests/diagnostic_log.rs::rotation_bounds_the_host_log_across_a_restart` can fail
its expectation of `HostResponse::ShuttingDown` when the developer's default
configuration includes the `dbviewer` plugin. With the source tree unchanged,
the reported comparison was:

| Configuration | Result |
| --- | --- |
| Default configuration containing `dbviewer` | Failed |
| The same configuration without `dbviewer` | Passed |
| Empty temporary `XDG_CONFIG_HOME` | Passed |

The fixture's `bundled_runyte` helper isolates runtime and cache storage but
does not override `XDG_CONFIG_HOME`. Its subprocess consequently loads personal
settings and may start configured plugins. This makes results depend on the
developer's environment and violates the repository's test-isolation boundary.
The previous plugin-input fix needed a temporary empty configuration for its
coverage run; that workaround does not make the subprocess fixtures isolated.

Expected behavior is for every test-launched editor or persistent-session host
that loads configuration to use a temporary fixture-owned configuration root.
Tests that exercise particular settings must pass their own explicit config.
Parent process environment must not be mutated because tests run concurrently.
Audit the other direct CLI, host and editor-child launch paths for the same gap.

Regression coverage should run the fixture under an intentionally invalid or
plugin-bearing temporary parent configuration and prove that the editor still
uses fixture defaults. It should also show that explicit fixture configurations
continue to take effect. No regression should read or modify personal settings,
start personal plugins, or write runtime state into tracked repository context.
