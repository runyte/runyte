# Platform parity fixtures

The shared `../runyte.txt` and `../runyte-fast.txt` inventories were captured
from the pre-YAML Rust registry. These files add the platform-conditional
bindings from that same source, audited at `90dba021` (merged #151).

A search of all `#[cfg]`, `cfg!`, `target_os` and `target_family` occurrences
in the old `src/keymap.rs` finds exactly three conditional bindings, all
`target_os = "macos"`: global Command-C for Normal/Select and Insert/Replace,
and Media Command-C for Normal/Select. `macos.txt` preserves their complete
binding records: the matching Ctrl-Shift-C records with the original Rust
constructors' `Char('c'), Modifiers::SUPER` key (modifier bits 8). There are
no Windows- or Linux-only additions; their fixtures explicitly record that.
No expected row is generated from the YAML loader under test.

Native-target integration tests combine the base and matching platform file.
Loader unit tests also compile and validate Linux, Windows, macOS and the
other-target fallback on every host, in both fast-pane variants, against the
same frozen records and preferred-spelling inventory. Binding rows are compared
independently of cross-mode grouping order; namespace/context order is exact.
