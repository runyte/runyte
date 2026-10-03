Terminal insert-character (`CSI n @`) and delete-character (`CSI n P`)
operations repeatedly shift the same row. `Grid::insert_characters` calls
`Vec::insert` once per requested cell, and `Grid::delete_characters` calls
`Vec::remove` once per cell. Inserting or deleting a substantial fraction of a
wide row therefore copies a quadratic number of cells on the editor thread.

The requested count is bounded by the remaining row width, but the geometry
contract permits wide, short terminal panes. A single operation near column
zero can move hundreds of millions of cells at the maximum supported width.
Ordinary full-screen applications can also issue these operations repeatedly.

Both operations should shift the retained cells once, fill the vacated range
with the current background, and preserve existing wide-character cleanup and
wrap provenance. Work should grow linearly with row width rather than the
product of row width and character count.

Reproduce at the emulator boundary by populating a wide row, positioning the
cursor near its beginning, and issuing `CSI 16000 @` or `CSI 16000 P` in a
32768-column, one-row terminal. Smaller deterministic cases should verify the
exact shifted content, background cells, count clamping, and wide characters
crossing the insertion, deletion, or right-edge boundary.
