The picker shown for multiple language-server locations duplicates its title's
initial letter. A references response opens a picker titled `Rreferences`, and
multiple definition results produce `Ddefinition`.

The title should capitalize the request label once, producing `References` or
`Definition`. The location rows and navigation behavior are otherwise unchanged.

Reproduce by requesting references for a symbol whose server returns at least
two locations and inspecting the resulting picker title.
