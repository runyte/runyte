Generating the manual, tutorial, or contextual help runs
`key_spelling::resolve_with_map` over the authored text. The resolver recounts
all Unicode characters in the accumulated output at least twice for every
literal input character. Resolving a long page therefore performs quadratic
work on the editor thread even when most of the page has no key markers.
Plugin-supplied help is escaped and passes through the same resolver.

The required source-to-rendered offset map uses character offsets rather than
bytes. Maintain its output position incrementally as literal characters,
escaped braces, and key substitutions are appended. Preserve exact text,
substitution ranges, and every map boundary, including multibyte characters
and combining marks. A large plain-text regression should cover the formerly
quadratic path without a flaky wall-clock assertion.
