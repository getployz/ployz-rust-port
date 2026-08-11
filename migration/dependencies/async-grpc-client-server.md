# Dependency decision: `async-grpc-client-server`

| Field | Value |
| --- | --- |
| Status | `human-decision-required` (corrected candidate pending fresh re-review) |
| Selected dependency | `tonic 0.14.6` with `tonic-build 0.14.6`, on `tokio 1.53.1` and response-aware `tower 0.5.3` middleware |
| License | `MIT` for Tonic, Tokio, Tower, Tokio Stream, Hyper Util, and Bytes; `MIT OR Apache-2.0` for HTTP |
| Research date | `2026-08-11` UTC |
| Request | Controller delegation for `async-grpc-client-server`; no request file exists at base `147dbcf` |

## Scope and oracle contract

The dependency registry immediately blocks `internal/machine/api/pb` and
`internal/grpcversion` on this capability. The same selected transport stack is
also consumed later by `internal/machine` (server construction),
`pkg/client/connector` (channels and custom transports), and
`internal/machine/api/proxy` (raw-codec proxy calls). This decision does not
decide protobuf message representation, descriptor fidelity, transparent
proxying, or connector retry policy; the latter two consumers therefore retain
explicit follow-up gates below.

The frozen oracle has four services and 38 RPCs: 34 unary, three server-streaming,
and one bidirectional-streaming RPC. The service definitions are
[`machine.proto`](../../upstream/uncloud/internal/machine/api/pb/machine.proto),
[`cluster.proto`](../../upstream/uncloud/internal/machine/api/pb/cluster.proto),
[`caddy.proto`](../../upstream/uncloud/internal/machine/api/pb/caddy.proto), and
[`docker.proto`](../../upstream/uncloud/internal/machine/api/pb/docker.proto).
The ordinary backend registers all four services without version interceptors in
[`machine.go`](../../upstream/uncloud/internal/machine/machine.go). That file
instead attaches the version interceptors to two raw-codec transparent-proxy
frontends built with `UnknownServiceHandler`: the local proxy constructed during
machine setup and the cluster proxy constructed after initialization. The
version interceptors in
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
| Behavior | Preserve canonical paths, all oracle streaming shapes, metadata and conditional response headers, status details/trailers, deadlines, cancellation, and custom async transports. | Tonic exposes unary/client-streaming/server-streaming/bidirectional codec paths, generic codecs, metadata-bearing `Request`/`Response`/`Status`, trailers, timeout encoding, custom channel connectors, and custom server incoming IO. The corrected locked probe asserts version rejection/success/error ordering and client warning timing as well as the oracle RPC shapes. Tonic documents cancellation by dropping an in-flight future/stream. | `pass` |
| License and security | Permissive licensing and no known RustSec advisories in the selected closure. | Exact manifests carry MIT or MIT/Apache-2.0-compatible licenses. `cargo audit` against 1,211 RustSec advisories reported zero vulnerabilities for the 66-package locked probe closure on 2026-08-11. | `pass` |
| Platforms and targets | Build on Linux and macOS, x86_64 and aarch64; make Unix transport behavior explicit. | The exact lockfile passed `cargo check` for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, and `aarch64-apple-darwin`. Tonic's built-in `unix:` connector is Unix-only; custom connectors remain available for SSH, stdio, and tunnel transports. | `pass` |
| Maintenance and Rust version | Maintained releases, declared MSRV compatible with the port, credible adoption. | Tonic 0.14.6 was released 2026-05-07, declares Rust 1.88, and has 3,256 crates.io reverse dependents. Tokio 1.53.1 and Tower 0.5.3 declare Rust 1.71 and 1.64 respectively. OpenTelemetry Rust and Linkerd2 Proxy use Tonic in their current manifests. | `pass` |
| Architectural constraints | Must not force a protobuf runtime while that critical decision is blocked; the ordinary four-service backend must remain un-intercepted, while conditional version middleware wraps only the two raw-codec unknown-service proxy frontends. | `tonic::codec::Codec` is generic and `tonic-build::manual` accepts caller-supplied message types and codec paths. The semantic middleware harness proves ordering on a registered custom-codec service, but this research did not demonstrate the same composition through a Tonic raw-codec unknown-service/transparent-proxy path. | `blocked` pending the proxy design/probe |

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
  directs response-aware logic to Tower middleware, but the oracle's conditional
  response behavior requires a purpose-built service rather than an
  unconditional response-mutation layer:
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
3. Keep the ordinary four-service backend created by `newGRPCServer`
   un-intercepted. Add both client-version metadata values with a Tonic client
   request interceptor. Only the two transparent-proxy frontends may receive the
   conditional version service, after the proxy gate demonstrates a raw-codec
   unknown-service composition. That service must validate the raw metadata
   headers before calling the proxy handler: a rejected call returns
   `FailedPrecondition` **without** `uncloud-server-version`; after validation
   succeeds, it adds the server-version header to the proxy response on both
   handler success and handler error. An unconditional response-header layer is
   not equivalent.
4. Inspect client responses after gRPC decoding, not in a generic HTTP response
   layer. A unary facade checks the server version only on `Ok(Response<_>)` and
   never on `Err(Status)`. A natural `VersionedStreaming<T>` stream wrapper
   retains the initial metadata and exposes an explicit `header()` access that
   performs the one-time warning; merely creating or consuming the stream must
   not warn. This preserves the temporary Go `ClientStream.Header()` timing,
   and no frozen caller invokes that method.
5. Preserve successful terminal trailers and terminal streaming `Status`
   details/metadata. The corrected probe covers both cases.
6. Use `Endpoint::connect_with_connector` and `Server::serve_with_incoming` for
   the existing SSH, stdio, tunnel, and Unix transports. Built-in UDS is not a
   Windows transport; this matches the currently tested target matrix.

Direct-support-crate ownership is intentional: `bytes` backs the chosen custom
codec and raw status details; `http` names the conditional service's request and
response types; `tower` supplies `Service`, `Layer`, and connector utilities;
`hyper-util` adapts Tokio IO for custom connectors; `tokio-stream` adapts
listeners and channel-backed request streams; and Tokio supplies runtime, IO,
networking, synchronization, and timers. Implementors may remove a direct
support dependency only when their crate does not use that surface; they must
not change the exact selected version transitively.

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
- **Transparent unknown-service proxying is an architectural blocker.** The
  frozen local and cluster proxy frontends combine the raw-byte codec,
  `UnknownServiceHandler`, and version interceptors; the ordinary registered
  four-service backend has no version interceptors. The semantic probe below
  uses a registered custom-codec service and therefore does not prove this
  composition. Require a separate Tonic raw-codec unknown-service proxy
  design/probe before the conditional server middleware is accepted or
  `internal/machine/api/proxy` is released.
- **Server/channel construction has downstream owners.** `internal/machine`
  must keep its ordinary four-service backend un-intercepted and apply version
  middleware only to its two proxy frontends after the proxy gate passes.
  `pkg/client/connector` must reuse this exact stack for TCP, Unix, SSH/stdio,
  and WireGuard channels, but stays gated on the retry decision. These are known
  consumers even though the current dependency registry lists only the two
  immediately blocked packages.
- **Metadata dependency is deduplicated.** `grpc-metadata-interceptor` is not a
  separate transport/runtime selection: `internal/grpcversion` must implement
  that behavior with this approved Tonic/Tower stack and the exact timing above.
  The controller must reconcile that currently unmatched package blocker when
  integrating records; this researcher does not edit registries.
- **No transport TLS or compression features are enabled.** Frozen connections
  are insecure gRPC over separately protected/local transports and configure no
  gRPC compression. Add either only in response to a new oracle requirement.

### Verification

A disposable exact-version probe used precisely the versions, default-feature
settings, and feature lists in the TOML block above. It generated one unary, one
server-streaming, and one bidirectional-streaming method with
`tonic_build::manual` and a custom byte codec, then ran a client and registered
service over `tokio::io::duplex` through a custom connector. Its conditional
version service asserted that rejection happens before the handler and has no
server-version header, while compatibility-passing success and handler-error
responses both have the header. Client assertions covered post-decode unary
checks (success only), no warning on unary error or stream creation, and
one-time warning only on explicit stream-header access. It also verified unary
and terminal-stream status details/metadata, terminal success trailers, and
`grpc-timeout` header encoding.

```text
tonic probe passed: conditional version middleware, post-decode unary checks,
explicit stream-header timing, custom codec/connector, RPC shapes, unary and
terminal-stream status details/metadata, success trailers, and deadline header
```

The probe encodes but does not expire a deadline, and it does not claim to have
observed remote cancellation. Those behaviors rely on Tonic's exact-release
request documentation and cancellation example cited above and require
package-level parity tests with real handlers. More importantly, the registered
service probe does **not** demonstrate `UnknownServiceHandler`-equivalent
routing or raw transparent proxying, so it cannot release the architectural
gate above. The locked 66-package graph passed `cargo run --locked`,
`cargo audit`, and `cargo check --locked` on the four targets named in the
hard-gate table using Rust 1.96.0. The selected stack's effective MSRV is
Tonic's declared Rust 1.88.

## Review

Fresh adversarial reviewer
`/root/async_grpc_research/async_grpc_fresh_adversarial_review` reviewed exact
candidate commit `9f456597e18466ad33fc4d73b0a221492a03f004` and returned `FINDINGS`:

1. Generic response middleware did not preserve conditional server-header,
   unary-success-only warning, or explicit streaming-header timing. Fixed with
   the precise server/client design and corrected probe above.
2. The record omitted direct downstream server, channel, and raw-proxy
   consumers. Fixed by separating immediately blocked packages from all known
   consumers and retaining the retry/proxy gates.
3. The verification description overstated deadline, cancellation, and trailer
   coverage. Fixed by adding asserted success trailers and terminal stream
   status details/metadata, and explicitly narrowing deadline/cancellation
   claims.

A newly named fresh-context re-review of the corrected commit is still required
before approval.

A subsequent fresh corrected review rejected commit
`14c0359544e3dbd795277c1434c3339d2a68a590` because it incorrectly assigned
version middleware to the ordinary four-service backend. This revision fixes
the oracle mapping: that backend is un-intercepted, and only the two
unknown-service raw-codec proxy frontends receive version middleware. Because
the disposable probe covers a registered service rather than that proxy path,
the architectural gate is now explicitly blocked pending the proxy design/probe.
The feature configuration was also made byte-for-byte equivalent to the record's
dependency declarations. Fresh re-review remains queued.

Immediately blocked package registry entries:

- `internal/machine/api/pb`
- `internal/grpcversion`

Known downstream consumers/follow-up gates:

- `internal/machine` — ordinary backend server construction uses this stack
  without version interception; both versioned proxy frontends remain
  proxy-gated.
- `pkg/client/connector` — channel/custom-transport construction uses this
  stack; it remains retry-gated.
- `internal/machine/api/proxy` — raw-codec client behavior remains proxy-gated.
