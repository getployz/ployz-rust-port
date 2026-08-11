# Dependency decision: `async-grpc-client-server`

| Field | Value |
| --- | --- |
| Status | `human-decision-required` (fresh adversarial review pending) |
| Selected dependency | `tonic 0.14.6` with `tonic-build 0.14.6`, on `tokio 1.53.1` and response-aware `tower 0.5.3` middleware |
| License | `MIT` for Tonic, Tokio, Tower, Tokio Stream, Hyper Util, and Bytes; `MIT OR Apache-2.0` for HTTP |
| Research date | `2026-08-11` UTC |
| Request | Controller delegation for `async-grpc-client-server`; no request file exists at base `147dbcf` |

## Scope and oracle contract

This decision is deliberately limited to the async gRPC service/client/runtime
capability used by `internal/machine/api/pb` and `internal/grpcversion`. It does
not decide protobuf message representation, descriptor fidelity, transparent
proxying, or connector retry policy.

The frozen oracle has four services and 38 RPCs: 34 unary, three server-streaming,
and one bidirectional-streaming RPC. The service definitions are
[`machine.proto`](../../upstream/uncloud/internal/machine/api/pb/machine.proto),
[`cluster.proto`](../../upstream/uncloud/internal/machine/api/pb/cluster.proto),
[`caddy.proto`](../../upstream/uncloud/internal/machine/api/pb/caddy.proto), and
[`docker.proto`](../../upstream/uncloud/internal/machine/api/pb/docker.proto).
The server registers all four services in
[`machine.go`](../../upstream/uncloud/internal/machine/machine.go). The version
interceptors in
[`interceptor.go`](../../upstream/uncloud/internal/grpcversion/interceptor.go)
add two request metadata values, reject incompatible clients with
`FailedPrecondition`, add an initial response header, and inspect that header on
both unary and streaming clients. Callers use TCP, Unix sockets, SSH transports,
stdio, and WireGuard rather than gRPC TLS.

No frozen server registers the gRPC reflection service. Generated service
descriptors are a separate concern of the blocked
[`protobuf-codegen-runtime`](protobuf-codegen-runtime.md) decision.

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Preserve canonical paths, all oracle streaming shapes, metadata and response headers, status details/trailers, deadlines, cancellation, and custom async transports. | Tonic exposes unary/client-streaming/server-streaming/bidirectional codec paths, generic codecs, metadata-bearing `Request`/`Response`/`Status`, trailers, timeout encoding, custom channel connectors, and custom server incoming IO. A locked executable probe exercised the oracle-required subset. | `pass` |
| License and security | Permissive licensing and no known RustSec advisories in the selected closure. | Exact manifests carry MIT or MIT/Apache-2.0-compatible licenses. `cargo audit` against 1,211 RustSec advisories reported zero vulnerabilities for the 66-package locked probe closure on 2026-08-11. | `pass` |
| Platforms and targets | Build on Linux and macOS, x86_64 and aarch64; make Unix transport behavior explicit. | The exact lockfile passed `cargo check` for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, and `aarch64-apple-darwin`. Tonic's built-in `unix:` connector is Unix-only; custom connectors remain available for SSH, stdio, and tunnel transports. | `pass` |
| Maintenance and Rust version | Maintained releases, declared MSRV compatible with the port, credible adoption. | Tonic 0.14.6 was released 2026-05-07, declares Rust 1.88, and has 3,256 crates.io reverse dependents. Tokio 1.53.1 and Tower 0.5.3 declare Rust 1.71 and 1.64 respectively. OpenTelemetry Rust and Linkerd2 Proxy use Tonic in their current manifests. | `pass` |
| Architectural constraints | Must not force a protobuf runtime while that critical decision is blocked; response-aware metadata logic must fit without generated-Go API imitation. | `tonic::codec::Codec` is generic. `tonic-build::manual` generates service APIs around caller-supplied message types and codec paths; its Prost integration was moved out to `tonic-prost-build`. Tonic interceptors cover request metadata/rejection, while ordinary Tower middleware covers response headers. | `pass` |

### Primary evidence

- The exact Tonic 0.14.6 release documents its async HTTP/2 gRPC model,
  streaming, metadata, authentication, health checks, and Rust 1.88 MSRV in the
  [release README](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/README.md)
  and declares features in the
  [release manifest](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic/Cargo.toml).
- `tonic-build` states that Prost functionality moved to `tonic-prost-build`,
  and its manual builder accepts input/output types, streaming flags, and an
  arbitrary codec path: [library](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic-build/src/lib.rs),
  [manual generator](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic-build/src/manual.rs).
- The Tonic interceptor contract intentionally receives `Request<()>` and
  directs response-aware logic to Tower middleware:
  [interceptor source](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic/src/service/interceptor.rs).
  `Status` preserves binary details and custom metadata:
  [status source](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic/src/status.rs).
- Channels accept a custom connector and provide a Unix connector, while servers
  accept custom `AsyncRead + AsyncWrite` incoming streams:
  [endpoint](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic/src/transport/channel/endpoint.rs),
  [Unix connector](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic/src/transport/channel/uds_connector.rs),
  [server](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic/src/transport/server/mod.rs).
- Crates.io records exact release, MSRV, download, and dependent data for
  [Tonic](https://crates.io/api/v1/crates/tonic),
  [Tokio](https://crates.io/api/v1/crates/tokio), and
  [Tower](https://crates.io/api/v1/crates/tower). Established-project manifests
  provide additional adoption evidence from
  [OpenTelemetry Rust](https://github.com/open-telemetry/opentelemetry-rust/blob/main/opentelemetry-otlp/Cargo.toml)
  and [Linkerd2 Proxy](https://github.com/linkerd/linkerd2-proxy/blob/main/Cargo.toml).

## Candidate comparison

| Candidate | Evidence and fit | Decision |
| --- | --- | --- |
| `tonic 0.14.6` | Idiomatic Tokio/Tower stack; generic codec; generated or manually-described services; all streaming shapes; structured status/metadata/trailers; custom transports; broad adoption and active maintenance. Smallest credible integration that does not pre-decide protobuf. | **Select.** |
| official `grpc 0.9.0` | The official successor stack is a preview layered on Tonic. Its [exact README](https://github.com/grpc/grpc-rust/blob/2cda2946b403adee79805949c26c9f960d5c63b1/grpc/README.md) says it is not recommended for production and all APIs are unstable; channel source still contains unimplemented public behavior. `grpc-protobuf` is tied to Google's protobuf runtime. Crates.io showed only 14 reverse dependents. | Reject: production stability and unresolved protobuf coupling. Revisit after stabilization. |
| `grpcio 0.13.0` | Mature feature surface, including streams, metadata, cancellation, rich errors, and C-Core service-config retries. However it is an unsafe FFI/CMake wrapper over gRPC C Core with platform-specific bindings and vendored BoringSSL defaults. Its codegen supports rust-protobuf 2/3 or Prost 0.11, all incompatible with the current protobuf gate. Version 0.13.0 was released in 2023 and declares no MSRV. See the [release README](https://github.com/tikv/grpc-rs/blob/5442991f322901d28e318bac5736abe13a10794c/README.md) and [manifest](https://github.com/tikv/grpc-rs/blob/5442991f322901d28e318bac5736abe13a10794c/Cargo.toml). | Reject: message-runtime coupling, native build and audit surface, weaker interceptor fit, and maintenance risk. |
| `volo-grpc 0.12.2` | Active pure-Rust async implementation with layers, metadata, timeouts, Unix sockets, and streaming. Its codec is coupled to Pilota's message trait/codegen, introducing another unapproved protobuf model; adoption is low (about 40k total downloads at research time) and defaults add compression/native build surface. See its exact [manifest](https://docs.rs/crate/volo-grpc/0.12.2/source/Cargo.toml.orig), [README](https://docs.rs/crate/volo-grpc/0.12.2/source/README.md), and [codec](https://docs.rs/crate/volo-grpc/0.12.2/source/src/codec/mod.rs). | Reject: protobuf coupling, ecosystem risk, and integration cost. |
| `tarpc 0.37` | Maintained Rust RPC framework with good Tokio integration, but it does not implement the standard gRPC-over-HTTP/2 wire protocol. | Reject: wire incompatibility. |

## Selected integration

Pin these exact direct dependencies. Feature choices are intentionally narrow:

```toml
[dependencies]
bytes = "=1.12.1"
http = "=1.5.0"
hyper-util = { version = "=0.1.20", features = ["tokio"] }
tokio = { version = "=1.53.1", features = ["io-util", "macros", "net", "rt-multi-thread", "sync", "time"] }
tokio-stream = { version = "=0.1.19", default-features = false, features = ["net"] }
tonic = { version = "=0.14.6", default-features = false, features = ["codegen", "router", "transport"] }
tower = { version = "=0.5.3", default-features = false, features = ["util"] }

[build-dependencies]
tonic-build = { version = "=0.14.6", default-features = false, features = ["transport"] }
```

Do not add `tonic-prost`, `tonic-prost-build`, `tonic-reflection`, or
`tonic-types`. The first two would pre-empt the blocked protobuf decision;
`tonic-reflection 0.14.6` itself depends on Prost/Tonic-Prost, and the oracle
does not register network reflection. The future package implementation should:

1. Feed the protobuf gate's selected message types and codec into
   `tonic_build::manual`, preserving the package/service/method strings from the
   frozen `.proto` files.
2. Implement the four natural generated server traits and clients. Represent
   streaming methods as async streams; dropping the call future/stream is the
   cancellation mechanism, consistent with Tonic's
   [cancellation example](https://github.com/grpc/grpc-rust/tree/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/examples/src/cancellation).
3. Use a Tonic request interceptor for the two client-version metadata values
   and server compatibility rejection. Use a Tower `Layer` for response-aware
   work: injecting `uncloud-server-version` and inspecting response headers on
   clients. Preserve streaming trailers and `Status` details.
4. Use `Endpoint::connect_with_connector` and `Server::serve_with_incoming` for
   the existing SSH, stdio, tunnel, and Unix transports. Built-in UDS is not a
   Windows transport; this matches the currently tested target matrix.

### Known limitations and follow-up gates

- **Protobuf remains blocked.** This decision makes the gRPC transport/runtime
  choice safe but does not unblock `internal/machine/api/pb` until
  `protobuf-codegen-runtime` is resolved.
- **Service-config retries are not supplied by Tonic.** The frozen connector's
  `UNAVAILABLE` retry policy lives in
  [`pkg/client/connector/common.go`](../../upstream/uncloud/pkg/client/connector/common.go),
  outside the affected packages. Tonic's standard retry design remains an open
  [upstream issue](https://github.com/grpc/grpc-rust/issues/1463), and naive
  Tower retries cannot clone streaming HTTP request bodies
  ([issue 733](https://github.com/grpc/grpc-rust/issues/733)). Require a separate
  retry dependency/design decision before porting that connector; do not claim
  parity from `tower::retry` alone.
- **Transparent unknown-service proxying is out of scope.** The frozen proxy
  uses a raw-byte codec and unknown-service handler. Require a separate proxy
  design/probe before `internal/machine/api/proxy` is released.
- **No transport TLS or compression features are enabled.** Frozen connections
  are insecure gRPC over separately protected/local transports and configure no
  gRPC compression. Add either only in response to a new oracle requirement.

### Verification

A disposable exact-version probe generated one unary, one server-streaming, and
one bidirectional-streaming method with `tonic_build::manual` and a custom byte
codec. It ran a client and server over `tokio::io::duplex` through a custom
connector and verified request interceptors, Tower response-header middleware,
metadata, `Status` details, trailers, and the `grpc-timeout` deadline header.

```text
tonic probe passed: custom codec, custom connector, unary/server/bidi streaming,
request/response middleware, status details, metadata, trailers, and deadline header
```

The locked 66-package graph passed `cargo run --locked`, `cargo audit`, and
`cargo check --locked` on the four targets named in the hard-gate table using
Rust 1.96.0. The selected stack's effective MSRV is Tonic's declared Rust 1.88.

## Review

Fresh adversarial review is required because this is a critical networking
capability. The review is pending an available isolated agent slot. Approval is
forbidden until the reviewer independently challenges the candidate set,
protobuf boundary, interceptor semantics, retry/proxy exclusions, features,
platforms, licenses/security, adoption, maintenance, and MSRV, and all findings
are resolved here.

Affected package packets/registry entries:

- `internal/machine/api/pb`
- `internal/grpcversion`
