# Dependency decision: `grpc-metadata-interceptor`

| Field | Value |
| --- | --- |
| Status | `human-decision-required` (fresh adversarial review pending) |
| Selected dependency | **No standalone dependency.** Reuse `tonic 0.14.6` and direct `tower 0.5.3` from the deduplicated `async-grpc-client-server` stack. |
| License | `MIT` for both direct crates |
| Research date | `2026-08-11` UTC |
| Request | Controller delegation for `grpc-metadata-interceptor`; no request file exists at base `147dbcf` |

## Decision and deduplication

This capability is naturally part of the async gRPC stack. Do not add an
interceptor helper crate. Tonic already owns the gRPC-aware `MetadataMap`,
ASCII/binary key and value types, `Request`, `Response`, `Status`, and
request-side `Interceptor`. Tower is the service middleware model underneath
Tonic and is required by the async gRPC decision for response-aware client and
server work. A separate metadata/interceptor dependency would duplicate those
types or wrap the same APIs without supplying missing behavior.

This record therefore reuses the exact versions and features selected by
[`async-grpc-client-server`](async-grpc-client-server.md):

```toml
tonic = { version = "=0.14.6", default-features = false, features = ["codegen", "router", "transport"] }
tower = { version = "=0.5.3", default-features = false, features = ["util"] }
```

`internal/grpcversion` must not add another runtime, HTTP implementation,
metadata map, interceptor facade, or response middleware crate. The integrator
owns the workspace declarations; this record does not authorize manifest or
lockfile edits by the package implementor.

## Oracle contract

The frozen implementation and tests are
[`interceptor.go`](../../upstream/uncloud/internal/grpcversion/interceptor.go)
and
[`interceptor_test.go`](../../upstream/uncloud/internal/grpcversion/interceptor_test.go).
The direct attachment points are the five connector implementations under
[`pkg/client/connector`](../../upstream/uncloud/pkg/client/connector) and the
two server constructions in
[`internal/machine/machine.go`](../../upstream/uncloud/internal/machine/machine.go).

The dependency-facing behavior is:

| Area | Required observable behavior | Selected API and constraint |
| --- | --- | --- |
| Request keys | Send `uncloud-client-version` and `uncloud-min-server-version` on every unary and streaming call. | Add lowercase ASCII keys through Tonic metadata or the equivalent HTTP headers in a Tower service. |
| Duplicate/overwrite behavior | `metadata.AppendToOutgoingContext` appends. Existing caller values remain first, and the server reads only the first value. A caller can therefore override either injected value; preserve this observable limitation. | Use `MetadataMap::append`/`HeaderMap::append`, **not `insert`**. Tonic's `get` returns the first value and `get_all` preserves duplicate-value order. |
| Case | gRPC keys are case-insensitive and arrive normalized to lowercase. | Keep static constants lowercase. Tonic's `MetadataKey::from_bytes` normalizes dynamic keys and comparisons/lookups are case-insensitive. |
| Values | Missing, empty, non-text, or invalid semantic-version values become `0.0.0`; only the first duplicate is parsed. | Convert the first ASCII value with `to_str`; map absence, empty strings, conversion failures, and semver parse failures to zero rather than surfacing a metadata error. |
| Binary metadata | The three version fields are ASCII and never use `-bin`; unrelated binary metadata must continue through untouched. | Tonic separates `get`/`append` from `get_bin`/`append_bin` and round-trips opaque bytes. Do not iterate, rewrite, or reject unrelated metadata. |
| Server rejection | Check the client version first, then its minimum server version. Reject before the handler with gRPC `FAILED_PRECONDITION` and the oracle's exact message. | Return `Status::failed_precondition`; a Tonic interceptor or Tower service may short-circuit before invoking the inner service. |
| Server response header | Only an accepted request gets `uncloud-server-version` as initial response metadata. It is staged before the handler and is present on the handler's success or error response. A version-policy rejection has no such header. | Use a response-aware Tower layer (Tonic `Interceptor` cannot see responses). Layer/order it so rejection bypasses header injection while every delegated response, including an application status error, gets the static ASCII header. |
| Unary client warning | Inspect the server header only after the RPC succeeds. Missing/empty/invalid/below-minimum means `0.0.0` and prints exactly one process-wide warning. Transport/status failure does not warn. | Inspect metadata on `Ok(Response<_>)`, or make a Tower response/body future preserve the same success boundary. Do not warn merely because an HTTP/2 response head arrived. |
| Streaming client warning | The Go wrapper warns only when `ClientStream.Header()` is explicitly called and succeeds; stream creation, message receive, and header error alone do not warn. | Tonic exposes initial metadata on `Response<Streaming<_>>`, but eager inspection would change timing. Preserve the delayed/explicit-header limitation in the package's natural client wrapper or middleware and characterize it in tests. |
| Cancellation | Metadata work is synchronous. Cancellation and stream drop retain the underlying gRPC behavior; no background work survives a dropped call. | Tonic interceptors are synchronous. Any Tower wrapper must delegate polling directly, add no task/buffer, and drop the inner future/body normally. |
| Platforms | No metadata behavior varies by OS. Shipped CLI targets are Linux/macOS amd64/arm64; daemon targets are Linux amd64/arm64. | Selected metadata and Tower APIs are platform-neutral. Transport platform behavior remains owned by the async gRPC decision. |

The Go append/first-value behavior is not incidental. Exact grpc-go 1.74.2
source documents and tests it in
[`metadata.go`](https://github.com/grpc/grpc-go/blob/v1.74.2/metadata/metadata.go)
and
[`metadata_test.go`](https://github.com/grpc/grpc-go/blob/v1.74.2/metadata/metadata_test.go).
Its `SetHeader` contract also says staged headers are emitted when the first
message or final status is sent, including a handler error:
[`server.go`](https://github.com/grpc/grpc-go/blob/v1.74.2/server.go#L2069-L2094).

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Typed gRPC metadata, duplicate ordering, case-insensitive keys, ASCII/binary separation, request short-circuit, response header mutation/inspection, unary and streaming coverage, exact status code, transparent cancellation. | Tonic's metadata and status APIs cover the gRPC types; its own interceptor documentation explicitly limits interceptors to request metadata/rejection and recommends Tower for response-aware work. A locked Rust probe reproduced duplicate-first, case, invalid-value, binary, response-metadata, and `FailedPrecondition` behavior. The async gRPC probe exercised request and response middleware across unary/server-streaming/bidirectional calls. Required layer and timing constraints are recorded above. | `pass` |
| License and security | Permissive license; no known advisory in the selected graph; no new unsafe/FFI surface. | Tonic 0.14.6 and Tower 0.5.3 declare MIT. The 28-dependency metadata probe graph declared only MIT, Apache-2.0, compatible combinations, and Unicode-3.0 terms. `cargo audit --no-fetch --deny warnings` scanned 1,211 RustSec advisories at database commit `d0861df1eab469d3c58d6b836ce48b5766e5f217` and found no vulnerability. Resolved `bytes 1.12.1` is newer than the `>=1.11.1` fix for RUSTSEC-2026-0007. Package code needs no unsafe or FFI. | `pass` |
| Platforms and targets | Linux/macOS x86_64 and aarch64; no OS-specific semantics. | The metadata-only locked probe passed Rust 1.96 checks for all four shipped target triples. The same four-target check passed for the full async gRPC probe. Tonic metadata and Tower service traits have no OS gates. | `pass` (non-Linux checks are compile-only, which is sufficient for this platform-neutral capability) |
| Maintenance and Rust version | Current maintained releases, MSRV no greater than Rust 1.96, broad production adoption. | Tonic 0.14.6 was released 2026-05-07, declares MSRV 1.88, and official crates.io data reported 353.7M total/81.5M recent downloads and 3,256 reverse dependents. Tower 0.5.3 was released 2026-01-12, declares MSRV 1.64, and reported 586.2M total/153.9M recent downloads and 4,559 reverse dependents. Both are current, non-yanked releases. | `pass` |
| Architectural constraints | Reuse the chosen async gRPC/Tokio/Tower architecture; do not pre-empt protobuf or add a Go-shaped facade. | These APIs are the native metadata and middleware surfaces of the selected stack. No protobuf crate is needed for this capability, and the implementation can be a small domain policy plus ordinary service layers. | `pass` |

### Primary-source evidence

- Tonic 0.14.6's
  [`Interceptor` contract](https://docs.rs/tonic/0.14.6/tonic/service/interceptor/trait.Interceptor.html)
  accepts `Request<()>`, can mutate/check `MetadataMap`, and can reject with a
  `Status`; it explicitly directs response-aware middleware to Tower. The
  [implementation](https://docs.rs/tonic/0.14.6/src/tonic/service/interceptor.rs.html)
  preserves the request body and bypasses the inner service when rejected.
- [`MetadataMap`](https://docs.rs/tonic/0.14.6/tonic/metadata/struct.MetadataMap.html)
  documents `append`, first-value `get`, ordered `get_all`, replacement by
  `insert`, and distinct binary methods. The
  [`MetadataKey` source](https://docs.rs/tonic/0.14.6/src/tonic/metadata/key.rs.html)
  documents normalization and case-insensitive comparison; the
  [`MetadataValue` source](https://docs.rs/tonic/0.14.6/src/tonic/metadata/value.rs.html)
  documents visible-ASCII validation, fallible text conversion, and opaque
  binary conversion.
- [`Response`](https://docs.rs/tonic/0.14.6/tonic/struct.Response.html) exposes
  initial response metadata, while
  [`Streaming`](https://docs.rs/tonic/0.14.6/tonic/struct.Streaming.html) exposes
  messages and trailing metadata. Initial stream metadata is therefore on the
  outer `Response`, not on each message.
- [`Status`](https://docs.rs/tonic/0.14.6/tonic/struct.Status.html) provides
  `failed_precondition`, and
  [`Code`](https://docs.rs/tonic/0.14.6/tonic/enum.Code.html) fixes its wire value
  at 9.
- The official [gRPC metadata guide](https://grpc.io/docs/guides/metadata/) and
  [HTTP/2 protocol](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md)
  define case-insensitive keys, ASCII versus `-bin` values, initial response
  headers, duplicate-value ordering, and the reserved `grpc-` namespace.
- Exact release manifests declare versions, features, licenses, and MSRVs for
  [`tonic 0.14.6`](https://docs.rs/crate/tonic/0.14.6/source/Cargo.toml.orig)
  and
  [`tower 0.5.3`](https://docs.rs/crate/tower/0.5.3/source/Cargo.toml.orig).
  Current release/download/dependent data came from the official crates.io APIs
  for [Tonic](https://crates.io/api/v1/crates/tonic) and
  [Tower](https://crates.io/api/v1/crates/tower) on the research date.

## Candidate comparison

| Candidate | Behavior and ecosystem fit | Adoption/maintenance/build cost | Decision |
| --- | --- | --- | --- |
| `tonic 0.14.6` + `tower 0.5.3` already selected for async gRPC | Exact native metadata types, request rejection, response-aware service composition, all RPC shapes, statuses, and normal cancellation. The recorded package policy supplies only version semantics. | Current, high-adoption releases; no second runtime/protocol stack and no extra package beyond the deduplicated async gRPC graph. | **Select; no standalone dependency.** |
| Tonic `Interceptor` alone | Exact request metadata and rejection API, but its contract only receives `Request<()>`; it cannot add or inspect response headers. | Zero extra graph, but fails response behavior. | Reject as the complete mechanism; use it only for request-side work with Tower for responses. |
| Official `grpc 0.9.0` metadata interceptors | Provides attach/capture header interceptors, but the release documentation says it is a preview, all APIs are unstable, and it is not recommended for production. It would replace rather than complement Tonic. | Only 14 reverse dependents in the async gRPC research; successor architecture still changing. | Reject on production-stability gate and because the async gRPC decision already rejects it. |
| `grpcio 0.13.0` / gRPC C Core | Supports custom metadata and interceptors but brings a second gRPC implementation, unsafe FFI/CMake, C Core/BoringSSL defaults, and incompatible codegen/runtime coupling. | Last release was in 2023, no declared MSRV, materially larger native platform/security surface. | Reject on architecture, maintenance, and build/security cost. |
| Raw `http::HeaderMap` or custom protocol metadata | Can represent duplicate HTTP fields, but does not itself enforce gRPC ASCII/`-bin` distinctions or provide gRPC statuses and stream integration. | Reimplements semantics already maintained by Tonic and risks reserved-header mistakes. | Reject as a standalone solution; Tower may manipulate validated static headers at the service boundary while package policy reads via Tonic types. |
| `tower-http` or niche interceptor helper crates | Generic/auth/observability helpers do not implement this application's version policy or its unusual warning timing. They still sit on Tower/Tonic. | Extra dependency and abstraction with no behavior advantage or adoption justification for this narrow policy. | Reject. |

## Selected integration

Follow the selected stack's natural service model:

1. Define the three lowercase ASCII keys once. Parse dynamic version strings to
   `AsciiMetadataValue` explicitly; the fixed minimum is a validated static
   value. Treat a failed incoming `to_str` or semver parse as zero.
2. On the client request path, `append` current-client and minimum-server values
   after all existing metadata. Never `insert`, clear, or rebuild the map.
3. On the server path, validate request metadata synchronously before polling
   the handler. Return `Status::failed_precondition` with the exact oracle text
   and do not attach the server-version header to this rejection.
4. For a request that passes validation, delegate unchanged and add exactly one
   `uncloud-server-version` initial response header to every inner response,
   including a handler status error. Put validation outside response injection,
   or implement the two actions in one Tower service, so the reject path cannot
   accidentally gain the header.
5. Keep the response warning's success and timing boundaries. Unary status or
   transport failures do not warn. Streaming creation/message receipt does not
   warn; only a successful explicit header-equivalent operation does. Use the
   process-wide atomic exactly-once gate only after determining a warning is
   required.
6. Middleware must not spawn, buffer, retry, alter readiness, consume body
   frames/trailers, or translate cancellation. It should delegate the inner
   future/body and preserve drop behavior.

Tonic's built-in interceptor can be used where generated client/server types
make request-only composition convenient. Response behavior belongs in the
same package's ordinary Tower layer/future; do not introduce a generic wrapper
whose purpose is to imitate grpc-go's function signatures.

### Known limitations

- Tonic interceptors are synchronous and request-only. Tower middleware is not
  optional for a global response-aware implementation.
- Tonic exposes streaming initial metadata when `Response<Streaming<_>>`
  resolves, earlier than grpc-go's explicit `ClientStream.Header()` method.
  Eager warning checks would be a parity regression even though the metadata is
  available; preserve the oracle's delayed limitation deliberately.
- `MetadataMap::insert` removes all prior duplicate values. It is correct for
  the single server-version header emitted by this package but incorrect for
  the two client request headers, which must append.
- Invalid metadata constructed from dynamic values is rejected by Tonic before
  transport. The package's emitted values are semver strings and therefore
  valid visible ASCII. Incoming conversion failures still map to zero as
  required.
- This record does not approve protobuf, retry, proxy, TLS, reflection, or
  compression dependencies. Those remain separate gates in the async gRPC
  decision.

## Verification

The disposable probe at `/tmp/ployz-grpc-metadata-probe` pins Tonic 0.14.6 with
default features disabled and the `codegen` feature. It asserts:

- existing/request-injected duplicate order `caller-first, 1.2.3` and
  first-value lookup;
- uppercase lookup and dynamic-key normalization;
- invalid key and newline-containing ASCII value rejection;
- unrelated binary byte round-trip `00ff6f7061717565`;
- response metadata insertion; and
- interceptor rejection with `Code::FailedPrecondition`.

Commands and results:

```sh
cargo +1.96.0 run --locked --offline \
  --manifest-path /tmp/ployz-grpc-metadata-probe/Cargo.toml
# PASS; printed the expected duplicate, case, binary, status, and response values

cargo +1.96.0 clippy --locked --offline --all-targets \
  --manifest-path /tmp/ployz-grpc-metadata-probe/Cargo.toml -- -D warnings
# PASS

cargo audit --no-fetch --deny warnings \
  --file /tmp/ployz-grpc-metadata-probe/Cargo.lock
# PASS; 28 dependencies, 1,211 advisories, no vulnerability

for target in \
  x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu \
  x86_64-apple-darwin aarch64-apple-darwin
do
  cargo +1.96.0 check --locked --offline --target "$target" \
    --manifest-path /tmp/ployz-grpc-metadata-probe/Cargo.toml
done
# PASS; non-Linux checks are compile-only
```

The broader probe recorded by
[`async-grpc-client-server`](async-grpc-client-server.md) additionally passed
actual request/response Tower middleware across unary, server-streaming, and
bidirectional calls over custom async IO. Its exact 66-package graph also
passed RustSec and the same four targets.

## Review

Fresh adversarial review is required because this capability controls every
gRPC request and response. Approval is forbidden until a new reviewer
independently challenges duplicate ordering, reject/header layer order, unary
and streaming warning timing, binary/invalid metadata, status/error mapping,
cancellation transparency, stack deduplication, candidates, licenses/security,
maintenance/adoption, MSRV, and platform evidence, and all findings are
resolved here.

Affected package/registry entry:

- `internal/grpcversion`

Direct downstream attachment points:

- `internal/machine`
- `pkg/client/connector`
