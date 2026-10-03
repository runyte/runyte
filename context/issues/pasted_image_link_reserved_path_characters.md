# Pasted-image references can interpret pathname characters as Markdown syntax

`pasted_image::destination` wraps spaces and parentheses and escapes angle
brackets, but leaves `#` unchanged. With a configured state root such as
`cache#scratch`, an image reference contains a target like
`cache#scratch/cache/images/<hash>.png`. Following that Markdown link with
`gf` splits the destination at `#` and treats the rest as a heading fragment,
instead of opening the stored image. External Markdown readers also interpret
the character as a URL fragment. `App::image_reference_target` also replaces every backslash with `/` on
Unix, although a backslash is a literal filename character there. A state root
named `cache\scratch` is consequently spelled as the nonexistent nested path
`cache/scratch`. Percent-encoded path spellings must retain their literal
meaning when introduced to repair the link encoding.

Image references should preserve the exact stored image identity for supported
pathnames. A reproduction uses temporary workspace/configuration storage with
a state-root directory containing `#`, pastes an image, and follows the generated
link. The stored file exists, but navigation currently looks up an unrelated
path or heading. Ordinary `.runyte/cache/images/` references should keep their
existing readable spelling.
