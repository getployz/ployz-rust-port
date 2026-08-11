# Dependency request: streaming gzip content decoding

| Field | Value |
| --- | --- |
| Status | `research-required` |
| Capability | Pure-Rust incremental gzip response decoding |
| Waiting packages | `internal/corrosion` |

## Required behavior

The frozen `internal/corrosion` client inherits Go HTTP transport behavior that
adds `Accept-Encoding: gzip`, emits `User-Agent: Go-http-client/2.0`, and
incrementally decodes matching gzip response bodies while removing the
content-encoding and content-length presentation headers. Select only the gzip
decoder capability; header policy remains package-owned.

The dependency must support byte-exact incremental async integration for plain
bodies, valid gzip, concatenated members if the Go oracle accepts them, corrupt
and truncated gzip, finite NDJSON snapshots, indefinitely open subscription
streams, caller cancellation, and response-body error precedence. Evidence must
come from exact Go 1.26.1 differential probes and the callers identified in
`migration/dependencies/http2-tls-client.md` G01. Exclude
`upstream/uncloud/experiment/**` entirely.

## Constraints

- Rust 1.96 and the shipped Linux/macOS amd64/arm64 targets.
- Pure Rust in the selected configuration: no C/zlib FFI or system package.
- Tokio/Hyper-compatible incremental backpressure and cancellation; no full-body
  buffering and no detached decoder task.
- Permissive license, active maintenance, acceptable RustSec result, exact
  version/features/lock, and no package-owned unsafe code.
- Prefer the popular idiomatic passing Rust solution after hard gates.

## Research assignment

Compare all credible maintained Rust gzip/DEFLATE choices using primary source,
exact manifests, adoption data, and executable Go/Rust probes. Treat a crate
named by the HTTP/2 decision as only a candidate. Record rejected candidates and
the exact reason each loses.

## Completion

Create `migration/dependencies/streaming-gzip-content-decoding.md`. Because this
touches a production network body pipeline, obtain a fresh adversarial
dependency review before approval.
