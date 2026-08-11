# Dependency decision: synchronous outbound HTTP client

| Field | Value |
| --- | --- |
| Status | `blocked` |
| Capability | Synchronous HTTP/HTTPS client for `internal/dns`, preserving the observable Go 1.26 `http.DefaultClient` contract |
| Approved dependency | None |
| Most promising candidate | Hyper 1.11.0 / Hyper-util 0.1.20 / Hyper-rustls 0.27.9 behind package-owned policy |
| Rejected provisional candidate | Reqwest 0.13.4 |
| Research date | 2026-08-11 UTC |
| Affected Go package | `upstream/uncloud/internal/dns` |
| Direct caller | `upstream/uncloud/internal/machine/cluster` |

Reqwest's failure is not proof that the capability is impossible. Raw Hyper
removes several irreducible Reqwest differences and is the best candidate found,
but the current evidence does not pass the behavior, platform, or pure-Rust hard
gates. The controller must not add this graph to the workspace or release the
waiting package on this record.

## Oracle contract

The immutable behavior comes from
[`client.go`](../../upstream/uncloud/internal/dns/client.go) and its direct caller
[`cluster/dns.go`](../../upstream/uncloud/internal/machine/cluster/dns.go). There
are no upstream package tests, so executable Go characterization is required.

The client must preserve:

- synchronous POST through the process-wide default transport; cancellation of
  the caller's gRPC context after entry does not cancel this outbound request;
- endpoint concatenation and Go request/URL validation;
- newline-terminated compact JSON, exact conditional headers, default
  `User-Agent`, transport-added gzip, HTTP/1.1 and HTTP/2, platform trust,
  environment proxies, and persistent connections;
- Go's redirect, retry, proxy bypass, DNS/dial, phase-timeout, and pool rules;
- full body read before status handling, numeric `200..=300`, the special 401
  contract, and stable package error boundaries; and
- sequential record creation with the successful prefix returned on a later
  failure.

Primary oracle sources:

- [Go 1.26.1 client and redirects](https://github.com/golang/go/blob/go1.26.1/src/net/http/client.go)
- [Go 1.26.1 default transport](https://github.com/golang/go/blob/go1.26.1/src/net/http/transport.go)
- [Go 1.26.1 environment proxy matcher](https://github.com/golang/go/blob/go1.26.1/src/vendor/golang.org/x/net/http/httpproxy/proxy.go)
- [Go 1.26.1 bundled HTTP/2 implementation](https://github.com/golang/go/blob/go1.26.1/src/net/http/h2_bundle.go)
- [Go 1.26.1 request construction](https://github.com/golang/go/blob/go1.26.1/src/net/http/request.go)

## Candidate comparison

Official crates.io metadata queried on 2026-08-11:

| Candidate | Adoption | Result |
| --- | --- | --- |
| Hyper 1.11.0 family | Hyper: 829,167,715 total / 182,705,507 recent downloads; Hyper-util: 410,809,660 / 122,810,171; Hyper-rustls: 567,994,046 / 137,206,622 | **Most promising, not approved.** It leaves headers, redirects, proxy matching, gzip, and overall timeout under package control while retaining HTTP/2, but the blockers below remain. |
| Reqwest 0.13.4 | 634,362,047 total / 159,403,965 recent | Rejected. It defaults `Accept: */*`, owns redirect and sensitive-header policy, lacks the global idle cap, and its public retry API cannot reproduce the oracle classifier. |
| Ureq 3.4.0 | 175,384,473 total / 49,311,367 recent | Rejected. It has a natural blocking API and strong proxy/pool controls, but is deliberately HTTP/1.1-only. |
| Isahc 2.0.1 / curl 0.4.50 | libcurl-backed | Rejected. Native libcurl/TLS transport does not meet the requested Rust transport design and makes exact policy more platform-dependent. |
| Attohttpc 0.31.0 and smaller clients | HTTP/1.1-oriented | Rejected. No HTTP/2 and no parity advantage over Ureq. |

Selected-candidate primary sources:

- [Hyper 1.11 client API](https://docs.rs/hyper/1.11.0/hyper/client/index.html)
- [Hyper-util 0.1.20 legacy client](https://docs.rs/hyper-util/0.1.20/src/hyper_util/client/legacy/client.rs.html)
- [Hyper-util 0.1.20 connectors](https://docs.rs/hyper-util/0.1.20/hyper_util/client/legacy/connect/struct.HttpConnector.html)
- [Hyper-util 0.1.20 proxy connectors](https://docs.rs/hyper-util/0.1.20/hyper_util/client/legacy/connect/proxy/index.html)
- [Hyper-util 0.1.20 composable pools](https://docs.rs/hyper-util/0.1.20/hyper_util/client/pool/index.html)
- [Hyper-rustls 0.27.9](https://docs.rs/hyper-rustls/0.27.9/hyper_rustls/struct.HttpsConnectorBuilder.html)
- [Rustls platform verifier 0.7.0](https://docs.rs/rustls-platform-verifier/0.7.0/rustls_platform_verifier/)
- [H2 0.4.15 error API](https://docs.rs/h2/0.4.15/h2/struct.Error.html)
- [Rustls-RustCrypto 0.0.2-alpha production warning](https://docs.rs/crate/rustls-rustcrypto/0.0.2-alpha)

## Probed Hyper graph

This is a research graph, not an approved dependency set:

```toml
base64 = { version = "=0.22.1", default-features = false, features = ["std"] }
bytes = "=1.12.1"
flate2 = { version = "=1.1.9", default-features = false, features = ["rust_backend"] }
h2 = "=0.4.15"
http = "=1.5.0"
http-body-util = "=0.1.4"
hyper = { version = "=1.11.0", features = ["client", "http1", "http2"] }
hyper-rustls = { version = "=0.27.9", default-features = false, features = ["http1", "http2", "ring", "rustls-platform-verifier", "tls12"] }
hyper-util = { version = "=0.1.20", features = ["client", "client-legacy", "client-pool", "client-proxy", "http1", "http2", "tokio"] }
ipnet = "=2.12.1"
tokio = { version = "=1.53.1", features = ["rt-multi-thread", "net", "sync", "time"] }
```

Do not infer approval from these pins. In particular, this graph does not expose
`tokio-rustls` and `rustls` directly for a connector that can put independent
deadlines around TCP and TLS phases.

## Hard-gate results

| Gate | Result | Evidence |
| --- | --- | --- |
| Observable behavior | **Blocked.** | Raw Hyper proved exact basic request control, connection reuse, complete body collection, numeric status 300, and visibility of one `REFUSED_STREAM`. It did not prove redirects, proxy execution, URL handling, independent timeout phases, global LRU behavior, cross-origin plain-proxy reuse, or Go's GOAWAY retry classifier. |
| License | **Pass for the probed graph only.** | All 101 resolved third-party package records declared permissive expressions. Any revised feature/dependency graph requires a fresh scan. |
| Security | **Pass for the probed graph only.** | `cargo audit --no-fetch --deny warnings` scanned the 102-package lockfile against 1,211 advisories and exited 0. This does not approve an unproven adapter or a revised graph. |
| Platforms | **Blocked.** | Linux x86_64 executed and Linux aarch64 checked. Native macOS trust and adapter behavior were not executed. Linux-to-macOS cross compilation stopped for lack of an Apple compiler/SDK and is not a native pass. |
| Rust 1.96 / maintenance | **Pass for the probed graph.** | The graph builds on Rust 1.96; declared MSRVs are at most 1.85; the selected families are current and maintained. |
| Pure-Rust architecture | **Blocked pending a proven graph.** | `ring 0.17.14`, selected by the tested Rustls provider, compiles C and assembly. The `rustls-rustcrypto 0.0.2-alpha` pure-Rust provider explicitly warns against production use, so it fails the security gate. Libcurl remains rejected for the same native-transport concern at a larger scope. |

Overall verdict: **blocked**.

## Ordered falsifiable blockers

Do not commission a full adapter yet. The next research task is only item 1;
stop and keep the capability blocked if it fails. Each later proof is warranted
only after every earlier item passes. A package implementation must not be used
as a dependency probe.

1. **HTTP/2 retry feasibility.** Build a local fault-injection peer and compare
   Go 1.26 with the unmodified Hyper candidate for `REFUSED_STREAM` and GOAWAY,
   including LastStreamID/stream-ID eligibility, `retry <= 6`, and jittered
   backoff. If behavior differs, demonstrate a public API that exposes enough
   metadata to correct it. The current public `h2::Error` observation proves a
   reason code only. A mismatch without public correction data is the terminal
   behavior blocker.
2. **Pure-Rust provider feasibility.** Produce a security-acceptable TLS graph
   consistent with the repository's pure-Rust constraint. Ring's C/assembly and
   Rustls-RustCrypto's production warning mean the current graph cannot pass.
3. **TLS phases and proxy routes.** With the passing provider, add exact direct
   lower-level TLS dependencies if needed and demonstrate separate 30-second
   DNS/TCP and 10-second TLS handshake deadlines for direct TLS, HTTP CONNECT,
   TLS-to-HTTPS-proxy followed by CONNECT and TLS-to-origin, SOCKS5, and SOCKS5h.
   Exercise Go-compatible environment precedence, CGI rejection, `NO_PROXY`,
   credentials, hostname verification, and DNS locality against the Go oracle.
4. **Pool identity and bounds.** Demonstrate 90-second idle expiry, two idle
   HTTP/1 connections per Go cache key, 100 total idle HTTP/1 connections with
   LRU eviction, shared HTTP/2 connections, stale-connection replay, and plain
   HTTP proxy reuse across different destination origins. Hyper's legacy pool
   key is destination scheme plus authority, so its default keying is not proof.
5. **Native platforms.** Run the resulting adapter and platform-verifier
   scenarios natively on both Linux and macOS. Cross-compilation without the
   Apple SDK is insufficient.

After all five feasibility gates pass, final adapter acceptance must also
differential-test endpoint/URL construction, exact HTTP/1 wire headers and body,
all redirect classes and credential propagation, gzip,
response/body/status/error ordering, and sequential partial results. These are
not part of the next HTTP/2 feasibility probe; the selected raw Hyper APIs
already expose the policy needed to implement them later.

## Verification performed

The focused Rust 1.96 research crate outside the repository ran:

```text
cargo +1.96.0 run --locked
  PASS: exact controllable headers, no Accept, HTTP/1 reuse, status 300,
  complete body collection, and platform-verifier construction
  PASS: one Hyper/h2 REFUSED_STREAM is visible in the public error source chain
  PASS: HTTP CONNECT, SOCKS5 local-DNS, and SOCKS5h compositions type-check

cargo +1.96.0 check --locked --all-targets
cargo +1.96.0 clippy --locked --all-targets -- -D warnings
  pass on Linux x86_64

cargo +1.96.0 check --locked --all-targets --target aarch64-unknown-linux-gnu
  pass

cargo audit --no-fetch --deny warnings
  exit 0; 102 locked packages; 1,211 advisories loaded

/opt/go1.26.1/bin/go run /tmp/ployz-hyper-sync-http-go-probe.go
  PASS: exact Go headers, HTTP/1 reuse, numeric status 300, full body read
```

The Go and Rust probes agree only on the exercised basic behavior. They do not
close the blockers above.

## Fresh adversarial review

The required read-only reviewer returned **BLOCK** on the proposed approval:

- the selected high-level TLS connector cannot expose separate connect and TLS
  deadlines with the proposed direct graph;
- the probe did not establish GOAWAY stream eligibility or Go's HTTP/2 retry
  loop;
- legacy pool keying does not preserve cross-origin plain-proxy reuse;
- major hard-gate claims were deferred rather than proven; and
- Ring's C/assembly was not reconciled with the pure-Rust requirement.

The record was corrected to `blocked`. A fresh exact-tip review must verify that
this final record makes no approval claim beyond its evidence.
