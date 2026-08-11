# Dependency request: `go-compatible-json-codec`

| Field | Value |
| --- | --- |
| Status | `research-required` |
| Capability | Go `encoding/json`-compatible JSON and incremental NDJSON codec |
| Waiting packages | `internal/corrosion`, `internal/dns` |

## Required behavior

Preserve compact newline-terminated encoding, `omitempty`, HTML and
U+2028/U+2029 escaping, case-insensitive fields, last duplicate wins, null and
empty distinctions, unknown fields, Go 64-bit `int` range, arbitrary admin JSON
objects, incremental NDJSON events, and package-specific tuple formats.

## Constraints

Rust 1.96; Linux and macOS amd64/arm64; memory-safe; permissive license; bounded
adapters rather than Go-shaped APIs. Exact versions/features require explicit
human authority because this capability was discovered after the current
approval scope.

## Research assignment

Compare the popular idiomatic Serde/serde_json stack and credible alternatives
against pinned Go executable probes. Obtain a fresh adversarial internal review.

## Completion

Record exact dependencies/features, adapter obligations, divergences, probes,
and clean review in `migration/dependencies/go-compatible-json-codec.md`.
