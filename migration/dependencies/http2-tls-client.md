# Dependency decision: HTTP/2 client for Corrosion

| Field | Value |
| --- | --- |
| Status | `blocked`: no candidate has passed the complete behavior gate |
| Capability | Registry name `http2-tls-client`; actual oracle capability is an asynchronous cleartext HTTP/2 prior-knowledge client (`h2c`) |
| Selected dependency | None while blocked |
| Strongest conditional candidate | Hyper `1.11.0`, hyper-util `0.1.20`, http-body-util `0.1.4`, h2 `0.4.15`, Tokio `1.53.1`, bytes `1.12.1`, fastrand `2.5.0`, and an as-yet ungated pure-Rust gzip decoder |
| License | MIT; MIT OR Apache-2.0; Apache-2.0 OR MIT |
| Research date | 2026-08-11 UTC |
| Request | Direct controller delegation; no dependency-request file exists at base `2234ddd` |

No package may consume the conditional stack. Fresh research replaced the
previous provisional `reqwest 0.13.4` choice after executable differential
work found that Reqwest cannot expose enough transport state for the frozen
oracle's retry policy. The lower-level Hyper stack passes useful h2c, timeout,
streaming, pooling, cancellation, platform, license, and security probes, but
it has not passed the complete behavior gate. H01 is one confirmed terminal
public-API mismatch. G01--G03 record other exact oracle behavior and
comparative evidence still required before any candidate can be approved.

Affected package: `internal/corrosion`
(`crates/ployz-internal-corrosion`). Its direct production callers are
`internal/machine`, `internal/machine/cluster`,
`internal/machine/corroservice`, and `internal/machine/store`.

## Oracle capability

The behavioral oracle is
[`upstream/uncloud/internal/corrosion`](../../upstream/uncloud/internal/corrosion),
especially [`client.go`](../../upstream/uncloud/internal/corrosion/client.go),
[`query.go`](../../upstream/uncloud/internal/corrosion/query.go), and
[`subscribe.go`](../../upstream/uncloud/internal/corrosion/subscribe.go).

The client contract is:

- one cloneable, concurrently usable, connection-pooled asynchronous client;
- HTTP/2 prior knowledge over cleartext TCP, with no HTTP/1 fallback and no TLS
  handshake;
- a three-second TCP connect timeout, no whole-request or read timeout, and
  cancellation inherited from the caller;
- IPv4 and bracketed IPv6 from the configured `netip.AddrPort`;
- incremental response bodies for finite NDJSON snapshots and indefinitely
  open subscriptions, with body close/drop unblocking the peer stream;
- GET and replayable owned-body POST, response status and headers, explicit
  JSON accept/content headers, and conditional bearer authorization;
- automatic `Accept-Encoding: gzip`, `User-Agent: Go-http-client/2.0`, and
  transparent gzip response decoding with encoding and length headers removed;
- the standard Go redirect policy, including its ten-hop default limit and
  method/body conversions;
- no idle-connection expiry, a 10 MiB response header-list limit, a 1 GiB
  connection WINDOW_UPDATE increment (an effective initial receive window of
  1 GiB plus 65,535 bytes), and a 4 MiB stream receive window;
- the internal `x/net/http2.Transport` retry policy, followed only for a
  surfaced `*net.OpError` by the package's separate two-second randomized
  exponential retry wrapper; and
- package-owned errors. Dependency display strings are not public API.

Primary sources:

- [frozen `client.go`](../../upstream/uncloud/internal/corrosion/client.go)
- [Go x/net/http2 v0.43.0 transport source](https://github.com/golang/net/blob/v0.43.0/http2/transport.go)
- [Go net/http redirect source](https://cs.opensource.google/go/go/+/refs/tags/go1.26.1:src/net/http/client.go)
- [RFC 9113 HTTP/2](https://www.rfc-editor.org/rfc/rfc9113)
- [frozen module versions](../../upstream/uncloud/go.mod)

### TLS, roots, verification, ALPN, and h2c

The registry name is misleading. `NewAPIClient` constructs an `http://` base
URL and configures `http2.Transport.AllowHTTP = true`. Its `DialTLSContext`
hook ignores the supplied `tls.Config` and opens a plain `net.Dialer` TCP
connection. No root store, certificate, SNI, hostname-verification, or ALPN
configuration exists.

The same limitation applies after a redirect to an `https://` URL: Go permits
the scheme but the custom dial hook still opens cleartext HTTP/2. A parity
implementation therefore permits only `http` and `https` redirect schemes and
speaks cleartext prior-knowledge HTTP/2 for both. Enabling rustls,
hyper-rustls, native-tls, platform roots, WebPKI roots, invalid-certificate
switches, or ALPN would be a behavior change and a new dependency request.

The conditional Hyper connector must use `enforce_http(false)` solely to admit
the oracle's `https`-spelled-but-cleartext redirect limitation. The initial
configured endpoint remains `http://<AddrPort>`.

### Proxy, CONNECT, DNS, and address handling

The frozen custom HTTP/2 transport has no HTTP proxy integration. It never
consults `HTTP_PROXY`, `HTTPS_PROXY`, or `NO_PROXY`, and the Corrosion client
constructs only GET and POST requests. CONNECT is not part of this capability.

The configured address is a literal `AddrPort`, so the initial request needs no
DNS. A redirect can name a host, and Go's `net.Dialer` then performs normal host
resolution. Hyper-util's `HttpConnector` supplies literal IPv4/IPv6 and normal
hostname resolution without enabling its proxy features. The probe passed both
IPv4 and bracketed IPv6 h2c.

### Automatic wire headers, gzip, pool lifetime, and HTTP/2 limits

The standalone frozen `http2.Transport` leaves `DisableCompression` false. For
every Corrosion GET and POST without an explicit `Accept-Encoding` or `Range`,
the transport adds `Accept-Encoding: gzip`. It also emits the default
`User-Agent: Go-http-client/2.0`. When the matching response is gzip-encoded,
Go removes `Content-Encoding` and `Content-Length`, sets unknown content
length, and transparently decompresses the body before the package's JSON
decoder reads it.

Hyper does not implement automatic content decoding. A package layer can set
both request headers, but transparent incremental gzip needs a separately
gated pure-Rust decoder and exact tests for finite and indefinitely open body
streams, truncation, corrupt gzip, cancellation, and header/error precedence.
`flate2 1.1.9` with only `rust_backend` is the leading unapproved primitive;
it is not selected by this record.

Other standalone Go transport defaults are observable under long-lived and
large-response workloads:

- idle connection timeout is zero;
- maximum response header-list size is 10 MiB;
- connection WINDOW_UPDATE increment is 1 GiB, making the effective initial
  receive window 1,073,807,359 bytes after the default 65,535-byte window; and
- stream receive window is 4 MiB.

Hyper-util defaults instead expire pooled idle connections after 90 seconds,
and Hyper defaults to 16 KiB headers, 5 MiB connection receive flow, and 2 MiB
stream receive flow. Its builder exposes `pool_idle_timeout(None)`,
`http2_max_header_list_size(10 << 20)`,
`http2_initial_connection_window_size((1 << 30) + 65_535)`, and
`http2_initial_stream_window_size(4 << 20)`, but this exact configuration still
needs dependency-time runtime probes.

## Exact redirect and bearer behavior

Go's `http.Client` owns redirects outside the `AuthRoundTripper`. For 301, 302,
and 303 it keeps GET/HEAD but changes other methods to GET and drops the body.
For 307 and 308 it preserves method and body only when the original request is
replayable. The request bodies created from `bytes.Reader` are replayable. A
missing or unparseable `Location` follows Go's normal response/error rules, and
the default redirect check stops before sending hop 11.

Go strips sensitive copied headers when the destination is not the same or a
subdomain, but the outer `AuthRoundTripper` then re-adds the configured bearer
token on every redirected round trip. The result is an observable
cross-origin bearer leak. Hyper performs no automatic redirects, which is the
right primitive: the package must own a compact redirect loop and reapply the
bearer on every hop, including chains that leave and later return to the
original origin.

The project-wide port contract explicitly requires preserving observable flaws
and limitations. Reproducing this bounded internal-client behavior follows that
existing direction; it does not authorize exposing the Corrosion endpoint or
token to an untrusted network. Package acceptance must differentially cover
301/302/303/307/308, GET/POST, relative/absolute locations, same origin,
different port/host, leave-and-return chains, replay, all headers, and the
ten-hop boundary.

## Exact retry contract

There are two distinct layers.

### HTTP/2 transport retry

`x/net/http2.Transport.RoundTripOpt` can make eight total attempts: the original
attempt plus seven retries while its retry counter is `<= 6`. Eligible failures
are:

- an unusable connection before the request begins;
- graceful GOAWAY that excludes the request;
- peer-originated stream `PROTOCOL_ERROR`; and
- `REFUSED_STREAM`.

The first retry is immediate. Later retries wait approximately 1, 2, 4, 8, 16,
and 32 seconds plus up to ten-percent positive jitter. Caller cancellation
interrupts every wait. Nil/empty bodies replay directly; owned query and
subscription bodies have `GetBody` and replay even after transmission began.

The live Go 1.26.1 probe against x/net v0.43.0 observed eight GET attempts on
stream IDs 1, 3, 5, 7, 9, 11, 13, and 15. Its measured intervals were immediate,
1.000 s, 2.001 s, 4.003 s, 8.007 s, 16.015 s, and 33.032 s; final elapsed time
was 64.061 s. That is direct evidence for the source-defined retry count and
jittered schedule.

GOAWAY has one special rule in
[`ClientConn.setGoAway`](https://github.com/golang/net/blob/v0.43.0/http2/transport.go#L936-L971):
if a non-`NO_ERROR` GOAWAY excludes stream 1, Go treats the error as terminal;
if it excludes a later stream, Go produces its retryable
`errClientConnGotGoAway` sentinel. This is the confirmed H01 mismatch for the
bounded Hyper candidate.

### Outer network-operation retry

Only an error that remains a `*net.OpError` after the HTTP/2 layer returns enters
`RetryRoundTripper`. cenkalti/backoff v4.3.0 starts at 100 ms, uses the default
0.5 randomization factor and 1.5 multiplier, and rejects any next interval that
would exceed two seconds total elapsed time. Caller cancellation interrupts the
wait. Protocol NACKs handled or surfaced by the HTTP/2 layer, response status,
and response-body errors never enter this wrapper.

## Hard gates

| Gate | Requirement | Result |
| --- | --- | --- |
| Behavior | h2c prior knowledge; no TLS/proxy; pooling; GET/POST; headers/gzip; streaming; 3 s connect timeout; cancellation; IPv4/IPv6; redirects; replay; limits; exact retry and error precedence | `blocked`: H01 is confirmed; G01--G03 lack exact dependency-time evidence. The Hyper probe establishes only the narrower mechanisms enumerated under Verification. |
| License and security | Permissive licenses, no advisory, no unnecessary TLS/native crypto, preserve the bounded oracle bearer limitation | `pass` for the tested Hyper graph: RustSec scanned 1,211 advisories and exited 0. `incomplete` for the required gzip adjunct until G01 gates its exact graph. |
| Platforms | Linux amd64/arm64 and portable macOS amd64/arm64; Windows compile is useful adjacent evidence | `pass` compile checks for all five targets; Linux amd64 runtime probe passed. Non-host checks are compile-only. |
| Maintenance and Rust version | Current, maintained, popular crates; MSRV no greater than Rust 1.96 | `pass` for the tested Hyper graph. Hyper/h2/Tokio are the active Tokio HTTP stack; declared MSRVs are at most 1.71. The gzip adjunct remains ungated. |
| Architecture | Natural async API and a bounded package policy layer, not a Go-shaped dependency facade | `blocked`. H01 prevents the bounded Hyper layer; direct h2 could expose stream identity only by replacing it with a package-owned transport, connection driver, and pool that has not passed a separate full-transport gate. |

## Candidate comparison

| Candidate | Result |
| --- | --- |
| `reqwest 0.13.4`, only `http2` | Rejected. Basic h2c/streaming behavior passes, but its default protocol-NACK policy makes only three attempts and has no delay; a custom classifier replaces rather than composes with the private protocol classifier. It also strips bearer across origins. Disabling its retries and redirects moves policy outward but still inherits Hyper's H01 gap. |
| Hyper `1.11.0` + hyper-util `0.1.20` + http-body-util `0.1.4` + public h2 error classification | **Strongest conditional candidate.** `http2_only(true)`, `retry_canceled_requests(false)`, a three-second `HttpConnector`, `Full<Bytes>` replay templates, and `Incoming` bodies pass useful base probes. H01 and G01--G03 block approval. |
| Direct `h2 0.4.15` + Tokio | `h2::client::SendStream::stream_id()` can supply the affected stream ID missing from Hyper. It does not supply a pooled HTTP client: the package would own TCP connect/timeout, handshake driver, request multiplexing, connection replacement, flow control, headers/body integration, retry, redirect, shutdown, and task-failure semantics. That is a materially different full-transport architecture, not a bounded adapter, and has no completed dependency-time lifecycle/pool probe. It remains a possible future attempt to clear H01, not an approved selection. |
| `corro-client 0.2.0-alpha.0` | Rejected. Alpha API, broad Corrosion/SQLite/DNS graph, low adoption, and different status, subscription, resubscription, and error behavior. |
| `isahc 2.0.1` | Rejected. Passive maintenance plus libcurl/nghttp2 C/FFI surface; no parity advantage for H01. |
| `awc 3.8.2` | Rejected. Actix-coupled, far lower adoption, and no API advantage for H01. |
| `ureq 3.4.0` | Rejected. No HTTP/2. |

Primary candidate APIs and manifests:

- [Hyper client HTTP/2 API](https://docs.rs/hyper/1.11.0/hyper/client/conn/http2/)
- [hyper-util legacy client builder](https://docs.rs/hyper-util/0.1.20/hyper_util/client/legacy/struct.Builder.html)
- [hyper-util `retry_canceled_requests`](https://docs.rs/hyper-util/0.1.20/hyper_util/client/legacy/struct.Builder.html#method.retry_canceled_requests)
- [hyper-util HTTP connector](https://docs.rs/hyper-util/0.1.20/hyper_util/client/legacy/connect/struct.HttpConnector.html)
- [h2 error API](https://docs.rs/h2/0.4.15/h2/struct.Error.html)
- [h2 send-stream API](https://docs.rs/h2/0.4.15/h2/client/struct.SendStream.html)
- [http-body-util `Full`](https://docs.rs/http-body-util/0.1.4/http_body_util/struct.Full.html)
- [Hyper 1.11.0 manifest](https://docs.rs/crate/hyper/1.11.0/source/Cargo.toml.orig)
- [hyper-util 0.1.20 manifest](https://docs.rs/crate/hyper-util/0.1.20/source/Cargo.toml.orig)
- [h2 0.4.15 manifest](https://docs.rs/crate/h2/0.4.15/source/Cargo.toml.orig)

## Conditional configuration (not approved)

If all gates are later cleared and this bounded stack remains selected, the
integrator would pin the HTTP primitives below. A gzip decoder is intentionally
absent until G01 selects one.

```toml
bytes = "=1.12.1"
fastrand = { version = "=2.5.0", default-features = false, features = ["std"] }
h2 = "=0.4.15"
http-body-util = "=0.1.4"
hyper = { version = "=1.11.0", default-features = false, features = ["client", "http2"] }
hyper-util = { version = "=0.1.20", default-features = false, features = ["client-legacy", "http2", "tokio"] }
tokio = { version = "=1.53.1", default-features = false, features = ["io-util", "net", "rt", "sync", "time"] }
```

The package would build one reusable client from an `HttpConnector` with a
three-second connect timeout, `enforce_http(false)`, `http2_only(true)`, and
`retry_canceled_requests(false)`. The builder must also set no pool idle
timeout, the 10 MiB response-header limit, the 1,073,807,359-byte connection
receive window, and the 4 MiB stream receive window recorded above. The package
would set the oracle user-agent and gzip-accept
headers, keep owned immutable body templates, implement redirects and both
retry layers, and incrementally decode gzip before consuming
`hyper::body::Incoming`. It must never enable Hyper HTTP/1, proxy features,
TLS, cookies, or automatic higher-level retries.

## Verification

### Hyper mechanism probe (not an exact candidate proof)

The isolated Rust 1.96 probe is
`/tmp/ployz-hyper-h2c-probe.JQNuu1`:

```text
Cargo.toml  518044ebee6ec2ef97d884b89f2726d4ebb6ee22e12b6f7c637a6d7b953f7d95
Cargo.lock  0fc596cd5d106140a503353906c3e20d695bc95a1dd91fa4c575b012cfb5070d
src/main.rs 7f3ab15e2aeeec9ea474c1b748708d488ab1c9fad3b76324edfee275138e9d61
```

It passed these bounded mechanisms:

- h2c prior knowledge, POST body, bearer/JSON headers, and response body;
- one warmed pooled connection carrying 15 concurrent additional streams;
- bracketed IPv6;
- incremental body reads and response-drop cancellation visible at the peer;
- public `h2::Error` classification of `REFUSED_STREAM` with hidden retries
  disabled, plus eight replayed owned-body attempts using deliberately scaled
  millisecond waits;
- measured three-second black-hole connect timeout, cancellation during
  connect, and distinct immediate refused-connection classification;
- malformed HTTP/2 and truncated response-body errors; and
- no implicit redirect in Hyper.

This probe is not the proposed exact configuration: it uses
`enforce_http(true)`, leaves Hyper's idle/header/window defaults untouched,
does not set the Go user-agent or gzip behavior, and uses scaled waits without
asserting the oracle timing distribution. It covers only `REFUSED_STREAM`, not
GOAWAY, peer `PROTOCOL_ERROR`, unusable pooled connections, partial body replay,
or final error precedence. Its retry-cancellation check times out a standalone
sleep rather than the full request loop. Its redirect case establishes only
that Hyper follows no redirects; it does not implement the Go redirect matrix.

Commands:

```text
cargo +1.96.0 fmt --all --check
cargo +1.96.0 check --locked --all-targets
cargo +1.96.0 clippy --locked --all-targets -- -D warnings
cargo +1.96.0 run --locked --offline
  PASS: exact Hyper h2c candidate behavior probe
cargo +1.96.0 check --locked --offline --target x86_64-unknown-linux-gnu
cargo +1.96.0 check --locked --offline --target aarch64-unknown-linux-gnu
cargo +1.96.0 check --locked --offline --target x86_64-apple-darwin
cargo +1.96.0 check --locked --offline --target aarch64-apple-darwin
cargo +1.96.0 check --locked --offline --target x86_64-pc-windows-gnu
  PASS (non-host targets compile only)
cargo audit --no-fetch --file Cargo.lock
  PASS: 43 packages, 1,211 advisories loaded
```

The probe directly depends on server/test-only features to create adversarial
peers; the conditional production feature set above omits them. No TLS or
native cryptographic implementation is in the graph.

### Exact Go retry probe

The isolated x/net v0.43.0 probe is
`/tmp/ployz-http2-go-probe.ayeom9`:

```text
go.mod  f8d428061da88f66a644ae9cae176e526249a1401a4d74036b801cde0bbe7a7e
go.sum  9b2cdd5edde52caea6848fbba3fd27c0754d69d052f70e695922985ab40b46db
main.go 8ae3c1d80b3e093c76e5cde325d4538130a14948ecd0132dcddec2d33eb53004
```

It uses a real TCP HTTP/2 peer that sends `REFUSED_STREAM` for each HEADERS
frame and records every attempt and interval. Its eight-attempt result is
reported in the retry section above.

## Other open behavior gates

### G01: automatic headers and streaming gzip

Select and hard-gate a pure-Rust incremental gzip decoder, provisionally
`flate2 1.1.9` with exactly `rust_backend`. Run Go/Hyper differential cases for
plain, valid gzip, corrupt gzip, truncated gzip, finite snapshot, indefinitely
open subscription, cancellation, and response-header mutation. Assert
`Accept-Encoding: gzip` and `User-Agent: Go-http-client/2.0` on every relevant
GET/POST and preserve Go's response-body error boundary.

### G02: complete retry and redirect differential

The scaled Hyper probe is API-reachability evidence only. An exact comparative
probe must cover `REFUSED_STREAM`, graceful GOAWAY, both non-graceful GOAWAY
stream branches, peer `PROTOCOL_ERROR`, unusable pooled connection, GET and
owned POST replay before and after partial body transmission, cancellation at
each real wait, final error category/precedence, and connection reuse. It must
measure the real 0/1/2/4/8/16/32-second jittered schedule and the separate
two-second network-operation retry layer.

The same exact configured client must exercise the complete redirect matrix:
301/302/303/307/308, GET/POST, relative/absolute locations, `http` and
`https`-spelled cleartext destinations, same origin, different port/host,
leave-and-return chains, body replay, bearer/header behavior, malformed/missing
location, and the ten-hop boundary.

### G03: exact connector, pool, limits, and error behavior

Re-run the mechanism probe with `enforce_http(false)`, no idle timeout, 10 MiB
header-list limit, 1,073,807,359-byte connection receive window, and 4 MiB
stream receive window. Cover a connection idle beyond 90 seconds, header blocks bracketing the
16 KiB and 10 MiB boundaries, flow-control behavior above Hyper's defaults,
concurrent reuse, IPv4/IPv6 and redirected hostname resolution, three-second
connect/cancellation, malformed input, truncated bodies, and drop/unblocking.

## Confirmed public-API blocker H01

The conditional Hyper stack surfaces a public `hyper::Error` whose source chain
contains public `h2::Error`. That API exposes `reason`, `is_go_away`,
`is_reset`, `is_remote`, and `is_io`. It exposes neither the GOAWAY
`last_stream_id` nor the affected request's stream ID. Hyper and hyper-util do
not attach a public stream ID or connection identity to their request future or
error.

Therefore a bounded policy layer can identify a remote non-`NO_ERROR` GOAWAY
but cannot decide which required oracle branch applies:

```text
excluded request used stream 1      -> terminal in frozen Go
excluded request used stream > 1    -> retryable in frozen Go
```

Parsing dependency `Debug` or `Display` output is rejected: stream ID is an
internal implementation detail, the GOAWAY error does not publicly carry last
stream ID, and text is not a stable classifier. Leaving Reqwest or Hyper's own
retries enabled also fails: their classifiers, caps, delays, and error set do
not implement the oracle rule.

To clear H01, one of these must produce dependency-time evidence:

1. a maintained public API in Hyper/hyper-util/h2 that exposes the affected
   stream identity through the pooled client; or
2. a fresh direct-h2 research result with an executable, reviewed connection
   manager proving pooled multiplexing, driver/task failure, replacement,
   shutdown, flow control, cancellation, all retry/GOAWAY branches, redirects,
   and platform behavior without recreating a second HTTP client unsafely.

Until H01 and G01--G03 are all cleared, `http2-tls-client` remains `blocked` and
`internal/corrosion` is not dependency-ready.

## Review

The first fresh adversarial networking review rejected the claim that H01 was
the sole blocker. It identified the omitted gzip/user-agent behavior, the
90-second/16-KiB/5-MiB/2-MiB Hyper defaults, and the mechanism probe's mismatch
with the proposed connector/retry/redirect configuration. Those findings are
now represented explicitly as G01--G03. A second fresh rereview rejected only
an off-by-65,535 connection-window target: Go's 1 GiB WINDOW_UPDATE is added to
the protocol's initial window, while Hyper accepts a target total. After the
record was corrected to 1,073,807,359 bytes, a third fresh read-only rereview
returned **CLEAN / ACCEPT** and confirmed all earlier corrections remain intact.
