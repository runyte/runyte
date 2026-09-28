# All cursors have the same color

In a multi-selection, every visible caret uses the same color. The primary
caret cannot be identified by its caret color, especially when the selections
are points with no range background. The primary caret should be visually
distinct from secondary carets while the selection ranges and editing behavior
remain unchanged.

Reproduction: create multiple carets in an editable buffer and compare their
colors. The reported example is shown in the [GitHub attachment](https://github.com/user-attachments/assets/5ac3160b-20c4-496d-b30e-eda011396b2d).
