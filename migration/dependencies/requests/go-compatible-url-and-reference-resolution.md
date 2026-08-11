# Dependency request: `go-compatible-url-and-reference-resolution`

| Field | Value |
| --- | --- |
| Status | `research-required` |
| Capability | Go `net/url`-compatible endpoint parsing and redirect reference resolution |
| Waiting packages | `internal/dns` |

## Required behavior

Parse concatenated endpoint strings and resolve redirect references with the
oracle's escaping of spaces, fragment removal, repeated slashes, dot segments,
query/fragment-only references, credential propagation, and redirect header
policy.

## Constraints

Rust 1.96; Linux and macOS amd64/arm64; memory-safe; permissive license; no
silent URL normalization beyond the frozen behavior. Exact versions/features
require explicit human authority because this capability is new.

## Research assignment

Compare popular maintained URL crates and a bounded adapter using pinned Go
differentials. Obtain a fresh adversarial internal review.

## Completion

Record the exact dependency/features, adapter rules, deviations, probes, and
clean review in
`migration/dependencies/go-compatible-url-and-reference-resolution.md`.
