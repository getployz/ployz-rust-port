# ANSI-aware grapheme terminal truncation

## Decision

Approve:

```toml
console = { version = "=0.16.4", default-features = false, features = ["ansi-parsing", "std", "unicode-width"] }
unicode-segmentation = { version = "=1.13.3", default-features = false }
```

Use `console::AnsiCodeIterator` to retain original ANSI/control slices and
`unicode_segmentation::UnicodeSegmentation::graphemes(..., true)` for atomic
extended grapheme boundaries. Use `console::measure_text_width` for terminal-cell
width. Append the ellipsis only within the remaining cell budget and retain the
complete ANSI slices after the cut so trailing resets are not lost.

## Evidence

- [`console` 0.16.4](https://github.com/console-rs/console/tree/0.16.4) exposes
  `AnsiCodeIterator`, `measure_text_width`, and ANSI-aware truncation machinery;
  the selected feature set enables parsing and Unicode terminal widths. It is
  MIT, Rust 1.71, widely used, and already researched for this workspace.
- [`unicode-segmentation` 1.13.3](https://github.com/unicode-rs/unicode-segmentation/tree/v1.13.3)
  implements Unicode extended grapheme clusters, including ZWJ emoji. It is
  MIT OR Apache-2.0 and needs no runtime, FFI, build script, or package `unsafe`.
- Ployz's Rust 1.96 exceeds both MSRVs and both crates are cross-platform.

## Rejected alternatives and limits

- `cli-truncate 0.1.1` explicitly truncates by Unicode scalar rather than
  grapheme cluster and can split ZWJ emoji.
- `print-positions 0.6.1` preserves graphemes and basic ANSI sequences but counts
  each grapheme as one position rather than measuring its terminal-cell width.
- Local code may coordinate the two approved iterators; it must not implement a
  second ANSI parser or Unicode segmentation algorithm.
