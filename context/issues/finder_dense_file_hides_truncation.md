The content scanner uses `line_hits`, which stops after exactly 50,000
matching lines in one file. The scanner subsequently truncates that already
capped list to its 50,000-row budget, so it detects no overflow and reports
`limited: false` when a single file contains additional matches. An extended
query then narrows the incomplete corpus instead of restarting the scan and
cannot find a matching line that was omitted from its end.

Create one file containing 50,000 lines `needle` followed by
`needle rare`. Search Finder contents for `needle`, then extend the query to
`needle rare`. The final row should be found by a new scan. The broad scan
must report that its results were truncated, while exactly 50,000 matches
must still be a complete scan.

Read one overflow candidate before enforcing the shared result budget, without
publishing more than 50,000 rows. Preserve the public bounded `line_hits`
helper's existing result size. Cover synchronous and background scans,
exact-budget input and a narrowed query reaching the omitted row.
