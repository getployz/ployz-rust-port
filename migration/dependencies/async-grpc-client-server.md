# Dependency decision: `async-grpc-client-server`

| Field | Value |
| --- | --- |
| Status | `approved` |
| Selected dependency | `tonic 0.14.6` with `tonic-build 0.14.6`, on `tokio 1.53.1` and response-aware `tower 0.5.3` middleware |
| License | `MIT` for Tonic, Tokio, Tower, Tokio Stream, Hyper Util, and Bytes; `MIT OR Apache-2.0` for HTTP |
| Research date | `2026-08-11` UTC |
| Request | Controller delegation for `async-grpc-client-server`; no request file exists at exact base `bb1d841c3ad59874c5076469a16aeb0ac409c3ea` |

## Scope and oracle contract

The dependency registry immediately blocks `internal/machine/api/pb` and
`internal/grpcversion` on this capability. The same selected transport stack is
also consumed later by `internal/machine` (server construction),
`pkg/client/connector` (channels and custom transports), and
`internal/machine/api/proxy` (raw-codec proxy calls). This decision does not
decide protobuf message representation, descriptor fidelity, package-level
proxy implementation, or connector retry policy. It does decide that the
selected stack has the public unknown-path and raw-codec surfaces required by
the proxy, and fixes their adapter contract below.

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
| License and security | Permissive licensing and no known RustSec advisories in the selected closure. | Exact direct manifests carry MIT or MIT/Apache-2.0-compatible licenses; the locked transitive closure is permissive (MIT, Apache-2.0, BSD-3-Clause, Unicode-3.0, or Unlicense combinations). A fresh `cargo audit` against 1,211 RustSec advisories reported zero vulnerabilities for the 64-package raw-proxy probe closure on 2026-08-11; the earlier 66-package generated-service closure was also clean. | `pass` |
| Platforms and targets | Build on Linux and macOS, x86_64 and aarch64; make Unix transport behavior explicit. | Both locked probes passed `cargo check` for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, and `aarch64-apple-darwin` under Rust 1.96. Tonic's built-in `unix:` connector is Unix-only; custom connectors remain available for SSH, stdio, and tunnel transports. | `pass` |
| Maintenance and Rust version | Maintained releases, declared MSRV compatible with the port, credible adoption. | Tonic 0.14.6 was released 2026-05-07, declares Rust 1.88, and has 3,256 crates.io reverse dependents. Tokio 1.53.1 and Tower 0.5.3 declare Rust 1.71 and 1.64 respectively. OpenTelemetry Rust and Linkerd2 Proxy use Tonic in their current manifests. | `pass` |
| Architectural constraints | Must not force a protobuf runtime while that critical decision is blocked; the ordinary four-service backend must remain un-intercepted, while conditional version middleware wraps only the two raw-codec unknown-service proxy frontends. | `tonic::codec::Codec` is generic, `tonic-build::manual` accepts caller-supplied message and codec paths, `Server::serve_with_incoming` accepts an arbitrary Tower service for the proxy-only listener, and public server/client `Grpc::streaming` accept a caller codec plus dynamic `PathAndQuery`. A codec-level runtime probe composed all four surfaces on unknown paths and preserved duplicate/binary metadata, bidirectional messages, success trailers, non-OK status/details, cancellation, rewritten whole-stream deadlines, and unsupported-compression rejection. A separate raw HTTP/2 probe covers multiplexing, resets, keepalive, and graceful shutdown. | `pass` |

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
- Tonic's public raw composition is sufficient without private Hyper/H2 APIs:
  `Server::serve_with_incoming` accepts the arbitrary Tower service used by a
  proxy-only listener; `server::Grpc::streaming` accepts an arbitrary HTTP body,
  codec, and `StreamingService`; and `client::Grpc::streaming` accepts a caller
  codec and dynamic `PathAndQuery`. `Routes::into_axum_router` also exposes a
  fallback if registered and unknown services ever must share one listener:
  [server codec](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic/src/server/grpc.rs),
  [client codec](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic/src/client/grpc.rs),
  [router](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic/src/service/router.rs).
- `MetadataMap` is backed by `http::HeaderMap` and preserves repeated ASCII and
  binary entries. `Streaming` parses arbitrary DATA boundaries and exposes
  terminal metadata; the server encoder serializes a terminal `Status`,
  including a code-OK status carrying successful custom trailers:
  [metadata map](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic/src/metadata/map.rs),
  [decoder](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic/src/codec/decode.rs),
  [encoder](https://github.com/grpc/grpc-rust/blob/6cb6056b5a748bc5a29bd48f4602dbc4e552bb7d/tonic/src/codec/encode.rs).
- The official gRPC-over-HTTP/2 protocol defines duplicate metadata, `-bin`
  values, the five-byte message envelope, compression, timeouts, trailers, and
  status details. Those are the wire-level acceptance oracle for the bounded
  adapter: [protocol](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md).
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
3. Build the two proxy frontends as a bounded codec-level fallback service, not
   a hand-built Hyper/H2 stack. Implement `RawCodec` with
   `Encode = Decode = bytes::Bytes`; capture the incoming `PathAndQuery` before
   decoding; serve that Tower service directly with
   `Server::serve_with_incoming` on each proxy-only listener; and call public
   `tonic::server::Grpc::streaming` and
   `tonic::client::Grpc<Channel>::streaming` for every method. This makes Tonic
   own gRPC envelopes, size limits, compression validation, metadata, and status
   encoding while leaving schema-independent messages to the proxy.
4. Preserve the frozen one-to-one pump: copy all application metadata including
   repeated ASCII and binary entries; apply the exact local/remote routing
   rewrites; pump request and response messages concurrently; copy initial
   backend metadata only after receiving the first backend message and before
   emitting that buffered message; if the backend fails before a message, do
   not expose its initial metadata. Propagate terminal non-OK code, message,
   binary details, and metadata; and, on success, obtain
   `Streaming::trailers()` and terminate the frontend response stream with
   `Status::with_metadata(Code::Ok, "", md)` so successful custom trailers
   survive. Do not use pure HTTP-body passthrough as the production message
   path: it bypasses the oracle's codec, compression, size-limit, and
   transport-header normalization behavior.
5. Preserve the frozen one-to-many limitation. The oracle installs no streamed
   detector, so plural-machine routing always broadcasts request messages to
   bounded per-backend channels, cheaply clones `Bytes`, applies backpressure,
   converts backend failures to protobuf payloads, mutates successful payloads,
   and concatenates completion-order payloads into exactly one response message.
   Copy every backend's initial metadata when that backend produces its first
   message, using the same timing rule as one-to-one, and aggregate every
   backend's successful trailers into the frontend trailer map. Preserve
   repeated ASCII and binary values rather than overwriting them. A backend
   failure becomes its protobuf error payload as the oracle requires, so its
   gRPC status metadata is not promoted to the frontend terminal status. Do not
   add a plural-machine streaming mode or impose deterministic backend order.
6. Parse `grpc-timeout` once at ingress into an absolute deadline and write the
   remaining duration to each outgoing backend request. Own all upstream tasks
   through the returned response stream; dropping that stream, a downstream
   reset, cancellation, or deadline expiry must cancel every backend. Tonic's
   transport timeout wraps only the service future, so it is not sufficient for
   a response body that outlives that future.
7. Keep the ordinary four-service backend created by `newGRPCServer`
   un-intercepted. Add both client-version metadata values with a Tonic client
   request interceptor. Wrap only the two raw fallback proxy frontends in the
   conditional version service. It validates before the director/codec: a
   rejected call returns `FailedPrecondition` **without**
   `uncloud-server-version`; after validation succeeds, it adds the server
   version to success, handler-status, and transport-error responses. An
   unconditional response-header layer is not equivalent.
8. Inspect client responses after gRPC decoding, not in a generic HTTP response
   layer. A unary facade checks the server version only on `Ok(Response<_>)` and
   never on `Err(Status)`. A natural `VersionedStreaming<T>` stream wrapper
   retains the initial metadata and exposes an explicit `header()` access that
   performs the one-time warning; merely creating or consuming the stream must
   not warn. This preserves the temporary Go `ClientStream.Header()` timing,
   and no frozen caller invokes that method.
9. Use `Endpoint::connect_with_connector` and `Server::serve_with_incoming` for
   the existing SSH, stdio, tunnel, and Unix transports. Built-in UDS is not a
   Windows transport; this matches the currently tested target matrix. Preserve
   the remote backend's cached channel, 10-second connect attempt, no call retry,
   reconnect-after-failure behavior, and 15-second maximum backoff with a small
   owner-managed lifecycle state machine; Tonic's reconnect service is not the
   same as grpc-go's background loop.

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
- **Proxy parity remains package work, not a dependency blocker.** The public
  fallback and raw-codec surfaces pass the dependency gate, but the proxy
  implementor must still prove the exact one-to-one and one-to-many contract
  above, including successful custom trailers, completion-order aggregation,
  whole-stream cancellation/deadlines, and remote reconnect timing. A pure
  raw-HTTP tunnel is only a lifecycle/differential harness and is not the
  production adapter.
- **Server/channel construction has downstream owners.** `internal/machine`
  must keep its ordinary four-service backend un-intercepted and apply version
  middleware only to its two proxy frontends.
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
  are insecure gRPC over separately protected/local transports and import no
  grpc-go compressor. The codec adapter must therefore reject unsupported
  compressed messages as Tonic does; do not silently add gzip, deflate, or zstd.
  Add TLS or compression only in response to a new oracle requirement.

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

The registered-service probe did not expire a deadline or directly observe a
remote reset, so a second disposable runtime probe targeted the HTTP/2
lifecycle substrate. It used public `Server::serve_with_incoming_shutdown`, an
arbitrary `tower::Service<Request<tonic::body::Body>>`, and raw `Channel` calls
between a client, proxy, and synthetic upstream over real TCP HTTP/2. It asserted:

- exact unknown service and method paths, 13 forwarded calls, and concurrent
  multiplexing on one channel;
- duplicate ASCII and base64-encoded binary request/response metadata;
- gRPC message envelopes split across HTTP DATA frames, request and response
  trailers, non-OK status/message/details, and duplicate terminal metadata;
- the conditional version ordering, including no server header on pre-handler
  rejection and a header on upstream non-OK responses;
- opaque gzip-flagged message preservation in the raw-body differential path;
- a 150 ms `grpc-timeout` expiring in Tonic's transport, a downstream response
  drop causing the upstream body to be dropped through an HTTP/2 reset, and
  keepalive plus graceful server shutdown after reset/deadline traffic.

```text
PASS unknown_paths=13 forwarded=13 metadata=duplicates+binary
trailers=ok+status_details streaming=opaque cancellation=RST deadline=150ms
compression=gzip_opaque h2=multiplex+keepalive+graceful_shutdown
```

One adversarial six-run repetition observed a race in this raw-body harness's
150 ms deadline assertion: Tonic's transport timeout won five times and the
harness's synthetic `Unavailable` mapping won once. That race does not support
the production deadline claim. The deterministic codec-level probe below owns
that claim; the raw-body probe is evidence only for HTTP/2 transport and
lifecycle behavior, not the production message adapter.

The decisive third disposable probe instantiated `RawCodec` with
`Encode = Decode = Bytes` on both `tonic::server::Grpc::streaming` and
`tonic::client::Grpc<Channel>::streaming`. An arbitrary Tower service captured
the unknown `PathAndQuery` before decoding and forwarded it to a synthetic
upstream. Six consecutive runs asserted:

- an unregistered service and method path traversing both codec surfaces;
- three bidirectional messages, including a 64 KiB message, without protobuf
  interpretation;
- repeated ASCII and binary request metadata, initial response metadata,
  successful trailers, and non-OK terminal metadata;
- a response message followed by `FailedPrecondition`, preserving message,
  opaque binary details, and repeated ASCII/binary metadata;
- downstream stream drop closing the upstream producer;
- a 350 ms ingress deadline enforced across the whole response stream, after a
  60 ms proxy delay, with a measured 200--320 ms remaining outgoing timeout and
  upstream producer closure; and
- a gzip-flagged message rejected as `Unimplemented` because no compression
  feature is selected.

```text
PASS codec_unknown_path raw_codec=server+client metadata=duplicates+binary
bidi=3_messages trailers=ok+non_ok_details cancellation=drop
deadline=whole_stream+remaining compression=unimplemented
```

Together the codec and HTTP/2 probes demonstrate the bounded public adapter;
no custom Hyper/H2 implementation is required. Harness-only `futures-util`,
`http-body`, and `http-body-util` entries are not approved production
dependencies. The disposable probe hashes were:

```text
Cargo.toml  40024e4611121a00dda60df1964e7982659c5112dff22f6c5e1e923d05d2d1ed
Cargo.lock  8dfd8de5012cfda3b629c90f57a6f8425c9f8b8788df36fc35dc5547110ba085
src/main.rs f172ea230bc38b9cfc5fd5b866a0f3b96d929162a05fbe6bd50bb080d29598bd
src/bin/codec_proxy.rs 4670a6ad641f79be514d52464888f296fb63dceee59d75ce411fd8028334ef2d
```

On Rust 1.96.0, formatted warnings-denied Clippy, six locked codec-probe runs,
and locked codec-probe checks for all four required targets passed. `cargo
audit` loaded 1,211 advisories and found no vulnerabilities in the 64-package
closure. That lock resolved Tonic 0.14.6 with Hyper 1.11.0, H2 0.4.15, Axum
0.8.9, and Hyper Util 0.1.20. The earlier 66-package generated-service probe
remains the evidence for `tonic_build::manual`, all oracle RPC shapes, custom
connectors, successful codec-level trailers, and the grpcversion client/server
timing. The selected stack's effective MSRV remains Tonic's declared Rust 1.88.

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

A subsequent fresh corrected review rejected commit
`14c0359544e3dbd795277c1434c3339d2a68a590` because it incorrectly assigned
version middleware to the ordinary four-service backend. This revision fixes
the oracle mapping: that backend is un-intercepted, and only the two
unknown-service raw-codec proxy frontends receive version middleware. The
feature configuration was also made byte-for-byte equivalent to the record's
dependency declarations.

Fresh read-only adversarial reviewer `/root/fresh_adversarial_review` then
returned `FINDINGS` against the 2026-08-11 blocker-clearing revision:

1. The raw HTTP/2 probe did not instantiate the proposed server/client codec
   seam. Fixed by the executable codec-level unknown-path probe above.
2. The one-to-many contract omitted initial metadata and trailer aggregation.
   Fixed by making their timing, aggregation, duplicate, and binary-value rules
   explicit in step 5.
3. The raw probe's deadline result raced. Fixed by narrowing that probe to
   lifecycle evidence and making the six-run codec probe the whole-stream
   deadline and remaining-time evidence.

The same read-only reviewer then returned `CLEAN` after independently running
the codec probe 10 times, the raw lifecycle probe 10 times, Rust 1.96 formatting
and warnings-denied Clippy, all four locked Linux/macOS x86_64/aarch64 checks,
and `cargo audit`. The reviewer confirmed that all three findings are resolved,
the one-to-many metadata/trailer contract matches grpc-proxy v0.5.1, the scope
contains only this owned record, and the frozen upstream tree is unchanged.

Immediately blocked package registry entries:

- `internal/machine/api/pb`
- `internal/grpcversion`

Known downstream consumers/follow-up gates:

- `internal/machine` — ordinary backend server construction uses this stack
  without version interception; its two proxy frontends use the bounded adapter
  above after their internal dependencies are integrated.
- `pkg/client/connector` — channel/custom-transport construction uses this
  stack; it remains retry-gated.
- `internal/machine/api/proxy` — the external raw proxy capability is resolved;
  the package remains `waiting-internal` on `internal/machine/api/pb` and must
  satisfy the adapter acceptance contract during implementation/review.
