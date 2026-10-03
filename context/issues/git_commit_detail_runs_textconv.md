Opening a commit detail can run a configured text conversion command.

`GitCliProvider::commit_detail` runs `git show --patch --no-ext-diff` without
`--no-textconv`. Git therefore applies a configured `diff.<driver>.textconv`
command selected by the file's `diff` attribute. Other Runyte diff readers
explicitly disable both mechanisms. A converter can have side effects or
delay opening the commit, and the displayed patch represents converted text
rather than the stored file contents.

Configure a text converter for a tracked file through `.gitattributes` and
repository configuration, make a commit changing that file, and open that
commit from the Git history. The converter executes. Reading a commit patch
should use the stored text without executing conversion commands, consistent
with other native diff views.
