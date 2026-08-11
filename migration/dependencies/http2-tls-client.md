# Dependency decision: HTTP/2 client for Corrosion

| Field | Value |
| --- | --- |
| Status | `blocked` pending the required fresh adversarial networking review |
| Capability | As registered: `http2-tls-client`; oracle-accurate scope: asynchronous HTTP/2 prior-knowledge client over cleartext TCP (`h2c`) |
| Provisional dependency | `reqwest = { version = "=0.13.4", default-features = false, features = ["http2"] }` |
| License | `MIT OR Apache-2.0` |
| Research date | 2026-08-11 UTC |
| Request | Direct controller delegation; no dependency-request file exists at base `175dce4` |

No package may consume this provisional selection until the required adversarial
review is recorded below and the status becomes `approved`. The registry name is
misleading: the frozen package never performs TLS. Enabling TLS would not improve
parity and would add a certificate-verification and cryptographic dependency
surface that production requests cannot reach.

## Capability and oracle contract

The behavioral oracle is
[`upstream/uncloud/internal/corrosion`](../../upstream/uncloud/internal/corrosion),
especially [`client.go`](../../upstream/uncloud/internal/corrosion/client.go),
[`query.go`](../../upstream/uncloud/internal/corrosion/query.go), and
[`subscribe.go`](../../upstream/uncloud/internal/corrosion/subscribe.go). Direct
production callers are under `internal/machine`, `internal/machine/corroservice`,
and `internal/machine/store`.

The selected transport must support:

- an async, cloneable, connection-pooled client safe for concurrent requests;
- HTTP/2 prior knowledge over an `http://<SocketAddr>` URL, including IPv4 and
  bracketed IPv6, with no HTTP/1 fallback and no TLS handshake;
- a three-second TCP connect timeout, no client-wide request or read timeout,
  and cancellation inherited from the caller's context/future;
- incremental response-body reads for newline-delimited query snapshots and
  indefinitely open subscription changes, with dropping/closing a response
  unblocking the peer stream;
- GET and replayable owned-body POST requests, explicit JSON accept/content
  headers, response headers/status, and conditional `Authorization: Bearer`;
- a package-owned, cancellation-aware exponential retry window beginning at
  100 ms and bounded at about two seconds for transient transport failures,
  while response-body decode failures and HTTP statuses are not transport
  retries; and
- stable package-owned error boundaries. Dependency-specific display strings
  and classifiers are not part of the public contract.

The Go client constructs only `http://` URLs and sets `http2.Transport.AllowHTTP
= true`; its `DialTLSContext` ignores the supplied `tls.Config` and opens a plain
`net.Dialer` TCP connection. The Go HTTP/2 documentation likewise defines
`AllowHTTP` as insecure plain-text HTTP/2. Corrosion listens on a concrete
socket address and is reached on loopback or the management network. Bearer
credentials therefore remain cleartext on that trusted network, which is an
intentional oracle limitation rather than permission to expose the endpoint to
an untrusted network.

Primary oracle and protocol sources:

- [Go `x/net/http2.Transport` documentation](https://pkg.go.dev/golang.org/x/net/http2#Transport)
- [Go `x/net` v0.43.0 HTTP/2 transport source](https://github.com/golang/net/blob/v0.43.0/http2/transport.go)
- [RFC 9113 HTTP/2](https://www.rfc-editor.org/rfc/rfc9113)
- [Frozen Go module versions](../../upstream/uncloud/go.mod)
- [Frozen release targets](../../upstream/uncloud/.goreleaser.yaml)

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | h2c prior knowledge; pooling; GET/POST; headers; streaming; 3 s connect timeout; no overall/read timeout; cancellation; replayable bodies; package-owned retry/error policy | Reqwest's [`ClientBuilder`](https://docs.rs/reqwest/0.13.4/reqwest/struct.ClientBuilder.html) exposes `http2_prior_knowledge`, `connect_timeout`, proxy/redirect policy, and pooled-client configuration. [`Response::chunk`](https://docs.rs/reqwest/0.13.4/reqwest/struct.Response.html#method.chunk) is available without the `stream` feature. [`Request::try_clone`](https://docs.rs/reqwest/0.13.4/reqwest/struct.Request.html#method.try_clone) supports owned-body replay. The loopback probe passed the concrete wire and cancellation cases. | `pass` for the required transport primitives; package acceptance must characterize retry policy and the known NACK limit below |
| License and security | Permissive license, no advisory, no unnecessary TLS/native crypto, no implicit proxy route | The [0.13.4 manifest](https://docs.rs/crate/reqwest/0.13.4/source/Cargo.toml.orig) declares `MIT OR Apache-2.0`, MSRV 1.85, and shows that only `http2` is selected. A 1,211-advisory RustSec scan of the exact target-inclusive 114-dependency probe lock exited 0. All 113 third-party package licenses were present and permissive/compatible. | `pass`, pending independent review |
| Platforms and targets | Linux daemon amd64/arm64; portable compilation for release-adjacent macOS and Windows targets | Rust 1.96 checks passed for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, and `x86_64-pc-windows-gnu`. The runtime h2c probe ran on Linux x86_64. No TLS/native trust code is selected. | `pass`; non-Linux checks are compile-only |
| Maintenance and Rust version | Current, maintained, widely used, MSRV no greater than workspace Rust 1.96 | Official [crates.io metadata](https://crates.io/api/v1/crates/reqwest) on 2026-08-11 reported 0.13.4 current/non-yanked, released 2026-05-25, MSRV 1.85, 634,362,047 total and 159,403,965 recent downloads, and 27,966 reverse dependencies. The repository's latest release is 0.13.4. | `pass` |
| Architectural constraints | Async Tokio-compatible natural API; no Go-shaped dependency adapter; exact capability only | Reqwest is already async and pooled. The package should own Corrosion request/event/retry semantics and use the dependency's client/request/response model. `json`, `stream`, proxy, compression, and TLS features are unnecessary. | `pass`, pending independent review |

## TLS, roots, and hostname verification

TLS is deliberately **not selected**. There is no TLS/root/hostname observable
contract in the frozen package:

1. `NewAPIClient` hardcodes `http://`.
2. The Go HTTP/2 transport permits cleartext and substitutes a plain TCP dialer
   for the TLS dial hook.
3. Every direct caller supplies a `netip.AddrPort`, not a hostname or URL.
4. No certificate, root store, SNI, ALPN, or hostname-verification option exists.

The evaluated reqwest `rustls` feature would bring `hyper-rustls`, Rustls,
AWS-LC, and `rustls-platform-verifier`. Reqwest 0.13.4's source constructs a
platform verifier when no custom roots are supplied, merges extra roots on its
supported targets, enables SNI, verifies certificate names by default, and
offers explicitly dangerous opt-outs. The
[platform verifier](https://docs.rs/rustls-platform-verifier/0.7.0/rustls_platform_verifier/)
uses Windows/macOS platform verification and Linux system roots with WebPKI.
Those are reasonable HTTPS defaults, but they are irrelevant to this package
and materially increase build/security surface. A future HTTPS, hostname, or
custom-CA requirement must return to the dependency gate; this record does not
approve silently adding `rustls`, `native-tls`, custom roots, or invalid-certificate
flags.

## Candidate comparison

Official crates.io metadata was queried on 2026-08-11. Adoption is an ecosystem
signal, not a security guarantee.

| Candidate | Behavior and maintenance evidence | Adoption | Disposition |
| --- | --- | --- | --- |
| [`reqwest 0.13.4`](https://docs.rs/reqwest/0.13.4/reqwest/) with only `http2` | Direct prior-knowledge h2c, pooled async requests, unconditional chunk API, connect-only timeout, replayable owned bodies, maintained current release, MSRV 1.85, no TLS/native crypto in the selected graph | 634.4M total / 159.4M recent downloads; 27,966 reverse dependencies | **Provisional selection:** strongest combination of fit, adoption, and integration simplicity |
| [`hyper 1.11.0`](https://docs.rs/hyper/1.11.0/hyper/) + [`hyper-util 0.1.20`](https://docs.rs/hyper-util/0.1.20/hyper_util/) + `http-body-util` | Exact low-level h2 machinery and the substrate under reqwest; current, MSRV 1.63/1.64. A package would need to own connector, pool, connection-driver, body, retry, and error plumbing across several direct crates. | Hyper 829.2M total / 182.7M recent and 5,283 reverse dependencies; hyper-util 410.8M / 122.8M | Reject among passing primitives: much more transport code and review surface without a required capability advantage |
| First-party [`corro-client 0.2.0-alpha.0`](https://crates.io/api/v1/crates/corro-client) | Corrosion's own README recommends it and it uses reqwest h2c, but the published API is alpha, declares no MSRV, depends on a broad Corrosion/SQLite/DNS type stack, enables keepalive, and changes status, ID, subscription, resubscription, and error behavior from this oracle | 2,908 total / 182 recent downloads; exact alpha release 175 downloads | Reject: domain-shaped but behaviorally different, heavy, unstable, and scarcely adopted |
| [`isahc 2.0.1`](https://docs.rs/isahc/2.0.1/isahc/) with libcurl HTTP/2 | Supports prior-knowledge HTTP/2, async incremental bodies, cancellation on drop, stable `is_network`, timeouts, and broad TLS; MSRV 1.85. Its own manifest says passively maintained and its HTTP/2 path adds bundled/native libcurl and nghttp2 build/FFI surface. | 16.9M total / 1.3M recent; 144 reverse dependencies | Reject: lower adoption, passive maintenance, and unnecessary C/FFI build/security cost |
| [`awc 3.8.2`](https://docs.rs/awc/3.8.2/awc/) | Async/streaming and has an explicitly named `dangerous-h2c` feature, but is coupled to the Actix ecosystem, declares no MSRV in crates.io metadata, and brings no parity advantage | 12.3M total / 0.9M recent; 108 reverse dependencies | Reject: far lower adoption and wrong runtime/ecosystem fit |
| [`ureq 3.4.0`](https://docs.rs/ureq/3.4.0/ureq/) | Popular, lightweight blocking client with good TLS choices, but no HTTP/2 support | 175.4M total / 49.3M recent; 3,242 reverse dependencies | Reject: fails the protocol hard gate |

## Selected integration

If the adversarial review accepts the provisional selection, the integrator
must pin exactly:

```toml
reqwest = { version = "=0.13.4", default-features = false, features = ["http2"] }
```

The package should construct one reusable client in its natural async model:

```rust,ignore
reqwest::Client::builder()
    .http2_prior_knowledge()
    .connect_timeout(std::time::Duration::from_secs(3))
    .no_proxy()
    .build()
```

Do not enable `blocking`, `json`, `stream`, compression, cookies, proxies, or a
TLS feature for this capability. `Response::chunk()` provides incremental bytes
without `stream`. The package owns NDJSON framing/decoding, bearer injection,
status/header interpretation, retry/backoff, and its public errors.

### Timeout and cancellation contract

- `connect_timeout(3 s)` covers connection establishment and requires a Tokio
  timer. Reqwest async has no total timeout by default; do not call `timeout` or
  `read_timeout` on the client/request.
- Each async send/chunk future must be selected against the caller cancellation
  signal. Dropping the future/response cancels that HTTP/2 stream; the executable
  probe observed the server response body being dropped after the client
  dropped an open response.
- Closing rows/subscriptions must drop the response even when no further bytes
  arrive. Higher-level resubscription backoff remains package-owned.

### Retry and error contract

- Serialize request bodies to owned bytes once so `Request::try_clone` succeeds.
- Wrap errors in package-owned variants such as request construction, connect/
  transport, body read, HTTP status, and event decode. Reqwest's documented
  [`Error`](https://docs.rs/reqwest/0.13.4/reqwest/struct.Error.html) classifiers
  may inform mapping, but are not the package API.
- Retry only failures before response headers that the package classifies as
  transient network/transport errors, stop promptly on cancellation, and bound
  total elapsed retry time to the oracle's two seconds. Do not retry HTTP status
  responses or errors encountered while consuming an already-returned body.
- Reqwest itself retries safe HTTP/2 `GOAWAY(NO_ERROR)` and `REFUSED_STREAM`
  protocol NACKs, but its 0.13.4 default caps these at two retries. The Go
  transport has a different internal retry budget. This rare attempt-count and
  timing divergence must be adversarially assessed and explicitly accepted or
  this decision remains blocked; reqwest's public custom classifier cannot
  compose with its private protocol-NACK classifier.
- Reqwest redirects differ subtly from Go's `http.Client` limit and sensitive
  header propagation. Corrosion's fixed IP endpoints do not redirect, but the
  package acceptance probe must characterize a 3xx response and either preserve
  the oracle policy in package code or record the redirect behavior as an
  accepted external limitation. Never leak the bearer token to another host.

## Verification

Two isolated Rust 1.96 probes were created outside the repository:

- `/tmp/ployz-http2-probe.QFz4IH`: real loopback Hyper HTTP/2 server plus the
  exact reqwest feature set. It verified h2c prior knowledge, HTTP/2 version,
  POST/body and bearer/JSON headers, chunked open responses, response-drop
  cancellation visible to the server, replayable owned bodies, and connection
  error classification.
- `/tmp/ployz-http2-min.ri5lWK`: exact one-dependency consumer used for lock,
  feature, platform, license, MSRV, Clippy, and RustSec checks.

Commands and results:

```text
cargo +1.96.0 run --locked --offline
  PASS outside the network sandbox (loopback bind required)
cargo +1.96.0 fmt --all --check
  PASS after formatting
cargo +1.96.0 check --locked --offline --all-targets
cargo +1.96.0 clippy --locked --offline --all-targets -- -D warnings
  PASS
cargo +1.96.0 check --locked --offline --target x86_64-unknown-linux-gnu
cargo +1.96.0 check --locked --offline --target aarch64-unknown-linux-gnu
cargo +1.96.0 check --locked --offline --target x86_64-apple-darwin
cargo +1.96.0 check --locked --offline --target x86_64-pc-windows-gnu
  PASS (non-Linux checks compile only)
cargo audit --no-fetch --file Cargo.lock
  PASS; 114 locked dependencies; 1,211 advisories at DB
  d0861df1eab469d3c58d6b836ce48b5766e5f217 (2026-08-11)
cargo tree --locked --offline -e features -i reqwest
  only reqwest feature `http2`
```

The target-inclusive minimal lock had 113 third-party packages. Every package
declared a license; the expressions were MIT, Apache-2.0, BSD-3-Clause, ISC,
Unicode-3.0, or compatible combinations. This is probe evidence, not approval
of a future integrated lock with Cargo feature unification.

## Known limitations and package acceptance obligations

1. Run a Corrosion-compatible h2c fixture over IPv4 and IPv6 where available,
   including more than one concurrent stream and reuse of one client.
2. Verify connect cancellation and the three-second connect boundary separately
   from an indefinitely open response body.
3. Exercise pre-header TCP reset, refused connection, DNS-inapplicable literal
   address, HTTP/2 `REFUSED_STREAM`, graceful/non-graceful GOAWAY, malformed
   frames, body truncation, and cancellation during retry/backoff; compare
   attempts and elapsed bounds with the Go oracle.
4. Prove GET/POST replay rules, conditional bearer behavior including an empty
   token, response-header/status handling, and that retries do not mutate the
   caller's logical request.
5. Characterize 3xx behavior and bearer containment as described above.
6. Re-run RustSec, licenses, exact feature inspection, Rust 1.96 checks, and
   Linux amd64/arm64 builds on the integrated lock. Cargo features are additive;
   explicitly retain `.no_proxy()` and the h2c URL invariant if another crate
   later enables proxy or TLS features on reqwest.
7. HTTPS, roots, hostnames, SNI, invalid certificates, and mTLS are outside this
   approval and require a fresh dependency decision.

## Review

Fresh adversarial networking review: **pending because all available agent
slots were occupied during this research task**. This critical capability
cannot become approved until a different researcher assesses at minimum the
misnamed TLS scope, h2c security boundary, default protocol-NACK retry mismatch,
redirect/bearer behavior, cancellation, feature-unification exposure, platform
claims, licenses, and the probe evidence.

Affected package: `internal/corrosion` (`crates/ployz-internal-corrosion`).
