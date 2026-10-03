After constructing a paste transaction, `paste_register` searches its complete
change list from the beginning for each replaced selection. It needs the retained
change because linewise expansion can make disjoint selections overlap, causing
transaction normalization to drop duplicate replacements. With N retained
replacements, finding them again performs quadratic work even when transaction
offset mapping itself is indexed.

Reproduce by selecting 20,000 separate one-character matches and pasting a
two-character register over all of them. Every replacement selection scans all
earlier transaction changes before it can select the inserted text.

Locate retained changes using their ordered boundaries. Preserve selection of
the actual retained replacement when original spans overlap, repeated insertions
share an offset, or an empty final line produces a zero-length replacement. Keep
the primary selection and single-step undo behavior unchanged.
