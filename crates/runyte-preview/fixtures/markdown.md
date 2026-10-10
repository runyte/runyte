# Native document preview

Readable proportional text with **strong**, *emphasis*, ~~strikethrough~~,
and [a link](https://example.com/documentation). Raw `<script>` is text.

## Lists and quotes

- Ordinary item
- [x] Finished task
- [ ] Remaining task
  - Nested item

1. First ordered item
2. Second ordered item

> This blockquote should reflow with the pane width.

| Format | Behavior |
| --- | --- |
| Markdown | Readable, proportional typography |
| JSON | Pretty printing with diagnostic fallback |
| YAML | Comments and ordering preserved |

```rust
fn main() {
    let markup = "<script>must remain literal</script>";
    println!("{markup}");
}
```

![Local color grid](image.png)

## Scrolling

This paragraph deliberately repeats enough prose to exercise reflow. Narrow the
pane and verify that the words wrap continuously without becoming editor rows.
The source selection and any unsaved edits should remain intact after Escape.

```text
A deliberately long line: 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 END
```

## Last heading

The final line is reachable by scrolling. Shift-wheel reaches the long code line.
