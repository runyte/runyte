The shared terminal retention budget omits a terminal's primary-screen
scrollback while its alternate screen is active. Both payload accounting and
eviction inspect only `Emulator::grid()`, which selects the alternate grid.
That grid has no history, but the primary grid and its scrollback remain
allocated. Several full-screen programs can therefore retain more history than
the workspace's 64 MiB terminal budget reports or enforces.

Retained primary scrollback must continue to count and remain eligible for
oldest-history eviction while an alternate screen is visible. Eviction must
leave both live screens intact, preserve active alternate-screen reads, advance
the conservative read revision, and report lost primary history when the
terminal returns to that screen.

Reproduce by producing primary-screen scrollback, entering the alternate screen
with `ESC [ ? 1049 h`, and comparing retained payload accounting before and
after. With a small injected workspace budget, output in another terminal
should evict the older retained rows even while the first terminal uses its
alternate screen.
