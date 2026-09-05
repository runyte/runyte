# Language-server configurations

Runyte configures `rust-analyzer` automatically. To use another language
server:

1. Install the server executable and make sure it is on `PATH` (or put its
   absolute path in `command`).
2. Copy the matching snippet below into
   `$XDG_CONFIG_HOME/runyte/config.yaml`, or `~/.config/runyte/config.yaml`
   when `XDG_CONFIG_HOME` is unset. When combining snippets, keep one `lsp:`
   heading and place every language entry below it.
3. Exit and reopen standalone Runyte. For a persistent session, use
   `runyte --session-restart [WORKSPACE]` and repeat any non-default
   `--config PATH`. Then open a file in that language, approve workspace LSP execution in the
   first-open prompt (or `:lsp-trust`), and run `:lsp-status`.
   `:service-health` reports whether the active document has a configured and
   attached server. A launch failure appears in `:lsp-status` after the first
   start attempt and in the notification center. `:lsp-restart <language>`
   restarts a server from the configuration already loaded by the running
   editor but does not reread the file.

These command lines are exercised by `tests/lsp_real_servers.rs`:

- [Rust / rust-analyzer](rust-analyzer.yaml) — included mainly as an explicit
  reference because Runyte already supplies it.
- [Python / Pyright](pyright.yaml)
- [Swift / SourceKit-LSP](sourcekit-lsp.yaml)
- [C and C++ / clangd](clangd.yaml)
- [JavaScript / typescript-language-server](typescript-language-server.yaml)
- [Go / gopls](gopls.yaml)
- [Markdown / Marksman](marksman.yaml)

The key immediately below `lsp` must be one of Runyte's built-in language
names. `command` names the executable to launch, `args` is its argument list,
and optional `initialization_options` is passed verbatim in the LSP handshake.
Runyte starts one process per configured language and does not invoke a shell.
Unknown keys below `lsp` are rejected, apart from the reserved `enable` setting
and legacy `servers` wrapper.

Older configurations may contain an extra `servers:` level. It remains
accepted, but has no behavior of its own; new snippets use the flatter
`lsp.<language>` shape.
