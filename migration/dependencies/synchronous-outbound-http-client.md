# Dependency decision: synchronous outbound HTTP client

| Field | Value |
| --- | --- |
| Status | `blocked` |
| Selected dependency | Provisional: `reqwest = { version = "=0.13.4", default-features = false, features = ["blocking", "gzip", "http2", "rustls"] }` |
| License | `MIT OR Apache-2.0` |
| Research date | 2026-08-11 UTC |
| Request | Direct controller delegation; no dependency-request file exists |

This decision is deliberately **blocked**. The candidate passes the researcher's
hard gates, but outbound networking and TLS are critical capabilities and require
a fresh adversarial second researcher before approval. The controller must not
approve this dependency or advance an affected package on this record alone.

## Capability and oracle contract

The behavioral oracle is
[`upstream/uncloud/internal/dns/client.go`](../../upstream/uncloud/internal/dns/client.go),
with its direct caller in
[`upstream/uncloud/internal/machine/cluster/dns.go`](../../upstream/uncloud/internal/machine/cluster/dns.go).
The required client must provide:

- synchronous URL parsing and request construction, including typed malformed,
  missing-scheme, and unsupported-scheme failures;
- HTTP POST with compact JSON followed by exactly one newline, explicit
  `Content-Type: application/json`, and conditional
  `Authorization: Bearer <token>`;
- HTTP and HTTPS using platform trust roots on Linux and macOS, with Windows
  behavior assessed;
- Go `http.DefaultClient`-like redirects, environment proxies, persistent
  connections, transparent gzip decompression, and no overall request timeout;
- complete response-body reads before status interpretation, with reliable RAII
  closure and reuse after a successful complete read; and
- separately typed endpoint/request-build, transport, body-read, and HTTP-status
  errors.

Oracle details that an adapter must preserve include sequential record creation,
partial accumulated output when a later record fails, response-body reading before
status handling, special 401 body interpretation, and the unusual accepted status
range `200..=300` (300 is accepted). Go's JSON encoder supplies the trailing
newline. Go's default redirect policy follows at most ten redirects; 301/302/303
can change POST to GET, while 307/308 preserve a replayable body.

Primary Go evidence:

- [Go 1.26 `net/http.Client` source](https://raw.githubusercontent.com/golang/go/go1.26.0/src/net/http/client.go)
- [Go 1.26 default transport source](https://raw.githubusercontent.com/golang/go/go1.26.0/src/net/http/transport.go)
- [Oracle Go version](../../upstream/uncloud/go.mod)

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Blocking HTTP/HTTPS POST, replayable redirects, proxies, pooling, gzip, full body consumption, no overall timeout, and sufficiently precise error boundaries | [`reqwest::blocking`](https://docs.rs/reqwest/0.13.4/reqwest/blocking/), [`ClientBuilder`](https://docs.rs/reqwest/0.13.4/reqwest/blocking/struct.ClientBuilder.html), [`Response`](https://docs.rs/reqwest/0.13.4/reqwest/blocking/struct.Response.html), redirect implementation, and focused loopback probes described below | `pass` (researcher; adapter required) |
| License and security | Acceptable license, platform-verifiable TLS, no current RustSec advisories in the exact resolved graph | Reqwest is `MIT OR Apache-2.0`; selected `rustls` feature uses `rustls-platform-verifier`; local `cargo audit --no-fetch --deny warnings` passed against advisory DB commit `d0861df1eab469d3c58d6b836ce48b5766e5f217` | `pass` (researcher) |
| Platforms and targets | Rust 1.96 on Linux and macOS; assess Windows | Linux Rust 1.96 build/probe passed; Windows GNU Rust 1.96 cross-check passed; reqwest 0.13.4 CI covers macOS and Windows GNU/MSVC; native macOS and Windows trust-store runtime tests remain integration requirements | `pass` (provisional) |
| Maintenance and Rust version | Current release, declared MSRV no newer than 1.96, active project | Reqwest 0.13.4 declares Rust 1.85, was released 2026-05-25, and its repository was active at research time | `pass` |
| Architectural constraints | Natural synchronous API, approved features only, no package/root/oracle coupling | Blocking client is reusable and cloneable; adapter can be package-local; no root or package edits are needed for this decision | `pass` (researcher) |

The overall decision remains `blocked` despite these provisional passes because the
mandatory adversarial networking/TLS review has not happened.

## Candidate comparison

Adoption figures came from the official crates.io API on 2026-08-11. Reverse
dependency counts are direct counts reported by that API and are used as an
ecosystem-fit signal, not as a security guarantee.

| Candidate | Current primary evidence | Fit and disposition |
| --- | --- | --- |
| [`reqwest` 0.13.4](https://docs.rs/reqwest/0.13.4/reqwest/) | 634,322,955 total downloads, 158,856,481 recent downloads, 27,959 reverse dependencies; Rust 1.85; `MIT OR Apache-2.0` | **Provisional selection.** By far the most adopted passing high-level client. Its blocking API, default ten-redirect policy, HTTP/2, gzip, pooling, environment proxies, and rustls platform verifier cover the capability with explicit configuration. Cost: a Tokio-backed internal blocking runtime and a comparatively large AWS-LC-based graph. |
| [`ureq` 3.4.0](https://docs.rs/ureq/3.4.0/ureq/) | 175,375,556 total downloads, 49,156,144 recent downloads, 3,238 reverse dependencies; Rust 1.85; `MIT OR Apache-2.0` | Strong, lighter, idiomatic blocking alternative. It can use the platform verifier and can disable status-as-error behavior, but it has no HTTP/2, its proxy environment selection differs materially, its convenient full-read path has a default size limit, and 307/308 replay needs more adapter work. Lower adoption and no compensating behavioral advantage. |
| [`isahc` 2.0.1](https://docs.rs/isahc/2.0.1/isahc/) | 16,924,748 total downloads, 1,280,671 recent downloads, 144 reverse dependencies; Rust 1.85; `MIT` | Libcurl can cover HTTP/2, pooling, redirects, proxies, and decompression, but redirects default off, its default connect timeout differs, and the project marks itself passively maintained. The C/libcurl/TLS build and platform matrix adds risk without an oracle advantage. |
| [`curl` 0.4.50](https://docs.rs/curl/0.4.50/curl/) | 43,239,759 total downloads, 4,041,781 recent downloads, 295 reverse dependencies; `MIT` | Maintained low-level libcurl bindings, not a high-level blocking client. Choosing it would require implementing request bodies, callbacks, redirect/error policy, full-read semantics, and reusable-handle policy around FFI. Its lower-level API and native build complexity lose to reqwest. |
| [`attohttpc` 0.31.0](https://docs.rs/attohttpc/0.31.0/attohttpc/) | 31,031,715 total downloads, 5,712,883 recent downloads, 78 reverse dependencies; no declared Rust version; `MPL-2.0` | Synchronous but HTTP/1.1-only, far less adopted, with an undeclared MSRV and more default-policy adaptation. Its weak-copyleft license also needs policy confirmation. It offers no advantage sufficient to displace reqwest. |

Official adoption endpoints used were
`https://crates.io/api/v1/crates/<crate>` and
`https://crates.io/api/v1/crates/<crate>/reverse_dependencies?page=1&per_page=1`.
Maintenance was checked against the candidates' official repositories and release
metadata. Candidate documentation was inspected for feature, timeout, redirect,
proxy, body, TLS, and status behavior rather than inferred from download counts.

## Selected integration

If the adversarial review accepts this selection, use exactly:

```toml
reqwest = { version = "=0.13.4", default-features = false, features = ["blocking", "gzip", "http2", "rustls"] }
```

Do not enable `default-tls`, `system-proxy`, `json`, `charset`, `brotli`,
`deflate`, `zstd`, `multipart`, `query`, or cookies for this capability. The
`rustls` feature selects rustls with `rustls-platform-verifier`; gzip is the only
automatic content coding enabled because Go's default transport only advertises
gzip. The base client already includes environment-proxy support. Enabling
`system-proxy` would additionally consult macOS and Windows operating-system proxy
settings, which Go's default client does not do.

Build and share one `reqwest::blocking::Client` configured with:

```rust,ignore
reqwest::blocking::Client::builder()
    .timeout(None)
    .connect_timeout(Some(std::time::Duration::from_secs(30)))
    .pool_max_idle_per_host(2)
    .build()
```

`timeout(None)` is mandatory: the blocking builder otherwise applies a 30-second
overall timeout, while Go's `http.DefaultClient` has no overall timeout. The
30-second connect timeout approximates Go's dial timeout, and the per-host idle
limit matches Go's default of two. It does not reproduce Go's separate 10-second
TLS-handshake timeout or global 100-idle-connection cap; those differences must be
accepted explicitly or handled by a narrowly scoped adapter after measurement.

### Required adapter behavior

1. Parse/build the endpoint and request before sending. Map malformed URL,
   missing/unsupported scheme, and request-construction errors to a package-owned
   typed `InvalidEndpoint`/request-build variant. Do not expose reqwest's error
   classifiers as the package API.
2. Serialize JSON with the package's separately approved JSON dependency, append
   exactly one `b'\n'`, and pass replayable owned bytes. Do not enable reqwest's
   `json` feature. Replayable bytes are required for 307/308 redirects.
3. Set only the oracle's explicit request headers: JSON content type and
   conditional bearer authorization. Never construct a bearer header for an empty
   token. Strip a URL from a reqwest error with `without_url()` before surfacing or
   logging it if an endpoint could contain user information.
4. Map request execution failures to typed `Transport`. Consume every successful
   HTTP response with `Response::bytes()` and map any error from that operation to
   typed `BodyRead`. A truncated-body probe produced an error classified by
   reqwest as decode rather than body, so `Error::is_body()` alone is insufficient.
5. Interpret status only after full body consumption. Do not call
   `error_for_status()` or use `StatusCode::is_success()`: the oracle accepts
   `200..=300`. Preserve its special 401 body handling and return a package-owned
   typed status error for other rejected statuses.
6. Preserve sequential POSTs and partial accumulated results when a later record
   fails. Let the fully consumed response and owned client follow normal RAII;
   never leak or retain a response on an error path.

### Known divergences from Go defaults

- Reqwest inserts `Accept: */*`; Go does not. Reqwest does not insert Go's
  `User-Agent: Go-http-client/1.1`. Both may insert `Accept-Encoding: gzip`, and
  transports add framing/host headers. The oracle's explicit headers are
  reproducible, but byte-for-byte ambient wire headers are not. The second
  researcher must decide whether this violates the caller's "exact headers"
  requirement; tests must record the actual wire set.
- Reqwest's environment proxy matcher also recognizes `ALL_PROXY`; Go's
  `ProxyFromEnvironment` documents `HTTP_PROXY`, `HTTPS_PROXY`, and `NO_PROXY`.
  CGI protections, lowercase variables, precedence, NO_PROXY matching, and HTTPS
  CONNECT behavior require parity tests. `system-proxy` stays disabled.
- Redirect defaults are close but not assumed identical. Test 301/302/303 method
  conversion, replay of buffered POST bodies on 307/308, the ten-hop limit,
  relative locations, and cross-origin stripping of Authorization. Domain and
  subdomain rules for sensitive headers may differ.
- Pool, TCP keepalive, protocol-error retry, DNS, Happy Eyeballs, HTTP/2, and
  connection-race details differ. Reqwest has no direct knobs for every
  `http.DefaultTransport` field. In particular, unexpected automatic retry of a
  POST would be correctness-sensitive and needs an adversarial probe.
- The selected rustls verifier uses the Windows trust API/store and macOS
  Security.framework/keychain. On Linux it loads native CA files through
  `rustls-native-certs` and webpki; revocation behavior is not identical across
  platforms and Linux roots are loaded once for a client. See
  [`rustls-platform-verifier` 0.7.0](https://docs.rs/rustls-platform-verifier/0.7.0/rustls_platform_verifier/).
- `reqwest::blocking` uses an internal Tokio runtime thread, and the selected
  rustls provider brings AWS-LC/C build tooling. This is acceptable for a
  synchronous caller only if the integration build and shutdown probes remain
  clean.

### Required implementation tests

An affected package is not complete until tests cover:

- malformed URLs, missing/unsupported schemes, endpoint path joining, and trailing
  slash/double-slash behavior matching the oracle's string concatenation;
- exact compact JSON bytes plus one newline, field omission/casing/order expected
  by the oracle, explicit content type, bearer inclusion/omission, and the actual
  complete ambient header set;
- 200, 300, redirect-followed 301/302/303/307/308, rejected status, 401 special
  bodies, malformed response JSON, and reading the full body before status;
- redirect limit, method/body transformations, replayability, relative locations,
  cross-origin credential stripping, and refusal to replay a non-buffered body;
- `HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`, CGI behavior, lowercase variants, and
  the documented `ALL_PROXY` divergence, including HTTPS CONNECT;
- transparent gzip and uncompressed responses, truncated/chunk failures as
  `BodyRead`, transport failures as `Transport`, and status errors as `Status`;
- reuse of one TCP connection after a complete response read and reliable cleanup
  on all failure paths;
- sequential record creation and partial accumulated output after a later error;
- no 30-second overall deadline, while connect timeout behavior is bounded and
  documented; and
- HTTPS against platform-trusted and untrusted roots on Linux and macOS, plus a
  native Windows trust-store/runtime check if Windows remains supported.

## Verification performed

A focused crate outside the repository pinned the exact dependency, edition 2024,
and `rust-version = "1.96"`. Its loopback server observed two POSTs on the same TCP
connection, exact newline-terminated bodies, conditional Authorization, transparent
request defaults, and explicit acceptance of status 300. It also verified that a
malformed URL is a builder error and that a truncated `Content-Length` body fails
while consuming `Response::bytes()`.

Commands and results:

```text
cargo +1.96.0 run --locked --offline
  pass on Linux; HTTP loopback probe and connection reuse passed

cargo +1.96.0 check --locked --offline --target x86_64-pc-windows-gnu
  pass after installing the GNU Windows cross compiler

cargo audit --no-fetch --deny warnings
  pass; 171 locked packages scanned, no vulnerabilities reported
  RustSec advisory DB commit d0861df1eab469d3c58d6b836ce48b5766e5f217

cargo tree -e normal,build -f '{p}\t{l}'
  all resolved normal/build dependency license expressions were permissive
```

Notable exact transitive versions in the probe lock were `rustls 0.23.43`,
`rustls-platform-verifier 0.7.0`, `aws-lc-rs 1.18.0`, `aws-lc-sys 0.44.0`,
`hyper 1.11.0`, `hyper-util 0.1.20`, `tower-http 0.6.11`, and `tokio 1.53.1`.
They are evidence for this research snapshot, not additional direct dependency
pins. Workspace integration must regenerate the lock and rerun RustSec and license
policy checks.

Linux execution was native. A macOS cross-build could not be made meaningful from
Linux without an Apple SDK/compiler; reqwest's tagged 0.13.4
[first-party CI workflow](https://raw.githubusercontent.com/seanmonstar/reqwest/v0.13.4/.github/workflows/ci.yml)
tests macOS and Windows GNU/MSVC targets. Windows was compile-assessed, not
native-runtime-assessed. Native TLS trust behavior remains a required test.

## Review

No fresh adversarial second researcher has reviewed this capability. Therefore:

- **Review result:** `blocked`
- **Affected package packet:** none exists at research time
- **Affected Go package:** `upstream/uncloud/internal/dns`
- **Direct caller:** `upstream/uncloud/internal/machine/cluster`

The fresh reviewer must independently reproduce or challenge the exact feature
selection, platform-root behavior, RustSec/license/MSRV results, body-read error
boundary, redirect and credential rules, proxy precedence/CGI safety, POST retry
behavior, no-overall-timeout configuration, ambient-header divergence, and native
macOS/Windows support claims. Any finding must be resolved in this record before a
controller can change its status to `approved`.
