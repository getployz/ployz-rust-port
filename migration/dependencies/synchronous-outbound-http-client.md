# Dependency decision: synchronous outbound HTTP client

| Field | Value |
| --- | --- |
| Status | `blocked` |
| Approved dependency | None |
| Rejected provisional selection | `reqwest = { version = "=0.13.4", default-features = false, features = ["blocking", "gzip", "http2", "rustls"] }` |
| Candidate license | `MIT OR Apache-2.0` |
| Research date | 2026-08-11 UTC |
| Request | Direct controller delegation; no dependency-request file exists |

The exact provisional reqwest selection fails the behavior hard gate. A fresh
adversarial reviewer independently reached the same `BLOCK` verdict. The
controller must not approve this dependency or advance an affected package on
this record.

Reqwest remains a plausible transport for a future reconsideration, but only
behind substantial package-owned redirect and proxy policy. That design has not
been implemented or proven. Enabling SOCKS would also change the resolved graph,
so the audit, license, MSRV, and target results below do not approve a revised
feature set.

## Capability and oracle contract

The behavioral oracle is
[`upstream/uncloud/internal/dns/client.go`](../../upstream/uncloud/internal/dns/client.go),
with its direct caller in
[`upstream/uncloud/internal/machine/cluster/dns.go`](../../upstream/uncloud/internal/machine/cluster/dns.go).
The required client must provide:

- synchronous HTTP and HTTPS POST using Go 1.26 `http.DefaultClient` behavior;
- compact JSON followed by exactly one newline, explicit
  `Content-Type: application/json`, and conditional
  `Authorization: Bearer <token>`;
- Go-compatible URL/request/execution errors, redirects, proxy environment,
  persistent connections, transparent gzip, HTTP/2, platform trust, and no
  overall request timeout;
- a complete response-body read before status interpretation and closure, with
  `200..=300` accepted, special 401 parsing, and package-owned typed error
  boundaries for endpoint/request construction, transport, body read, status,
  authentication, and decoding; and
- sequential record creation with partial accumulated output when a later
  record fails.

Go's JSON encoder supplies the trailing newline. `ReserveDomain` sends no bearer
header. `CreateRecords` sends it only for a non-empty token. Both call through
the shared `http.DefaultClient`; a package implementation must likewise share
one process-wide blocking client rather than create a runtime and pool per
operation.

Primary oracle sources:

- [Go 1.26 client, redirect, body, and error handling](https://github.com/golang/go/blob/go1.26.0/src/net/http/client.go)
- [Go 1.26 default transport, gzip, pooling, retry, and proxy setup](https://github.com/golang/go/blob/go1.26.0/src/net/http/transport.go)
- [Go 1.26 environment proxy matcher](https://github.com/golang/go/blob/go1.26.0/src/vendor/golang.org/x/net/http/httpproxy/proxy.go)
- [Go 1.26 bundled HTTP/2 retry loop](https://github.com/golang/go/blob/go1.26.0/src/net/http/h2_bundle.go)
- [Oracle Go version](../../upstream/uncloud/go.mod)

Primary candidate sources:

- [Reqwest 0.13.4 feature manifest](https://github.com/seanmonstar/reqwest/blob/v0.13.4/Cargo.toml)
- [Reqwest 0.13.4 client builder and transport defaults](https://github.com/seanmonstar/reqwest/blob/v0.13.4/src/async_impl/client.rs)
- [Reqwest 0.13.4 blocking timeout and runtime](https://github.com/seanmonstar/reqwest/blob/v0.13.4/src/blocking/client.rs)
- [Reqwest 0.13.4 redirect policy and sensitive-header logic](https://github.com/seanmonstar/reqwest/blob/v0.13.4/src/redirect.rs)
- [Reqwest 0.13.4 retry policy](https://github.com/seanmonstar/reqwest/blob/v0.13.4/src/retry.rs)
- [Hyper-util 0.1.20 environment proxy matcher](https://docs.rs/hyper-util/0.1.20/src/hyper_util/client/proxy/matcher.rs.html)
- [Tower HTTP 0.6.11 redirect implementation](https://docs.rs/tower-http/0.6.11/src/tower_http/follow_redirect/mod.rs.html)
- [Rustls platform verifier 0.7.0 platform behavior](https://docs.rs/crate/rustls-platform-verifier/0.7.0/source/README.md)

## Hard gates

| Gate | Evidence and result |
| --- | --- |
| Behavior | **Fail.** The selected features omit Go-supported SOCKS proxies. Reqwest's automatic proxy rules, redirects, credential propagation, malformed-Location handling, retry limits, ambient headers, and socket/TLS timeout boundaries materially differ. No tested adapter closes them. |
| License and security | **Pass for the rejected four-feature probe graph only.** Reqwest is `MIT OR Apache-2.0`; all resolved normal/build expressions were permissive; RustSec scanned 171 locked dependencies at advisory DB commit `d0861df1eab469d3c58d6b836ce48b5766e5f217` and exited 0. A SOCKS-enabled graph requires a fresh check. |
| Platforms and targets | **Incomplete.** Rust 1.96 Linux native and Windows GNU cross-checks passed. Native macOS/Windows platform-root behavior was not executed. Linux-to-macOS cross compilation failed without an Apple SDK/compiler and is not evidence of a candidate defect or a native pass. |
| Maintenance and Rust version | **Pass.** Reqwest 0.13.4 declares Rust 1.85, was current on 2026-08-11, and was released 2026-05-25. Rust 1.96 satisfies the declared MSRV. |
| Architectural constraints | **Fail as selected.** A reusable blocking client is natural, but parity requires substantial package policy rather than configuration alone; workspace feature unification can also silently change TLS, proxy, and coding behavior. |

Overall verdict: **blocked**.

## Adversarial behavior findings

### Ambient headers and request construction

The loopback probe observed reqwest send `Accept: */*` and
`Accept-Encoding: gzip`, with no User-Agent. Go sends
`User-Agent: Go-http-client/1.1`, adds gzip under its conditions, and does not
send `Accept: */*`. A builder can set the Go User-Agent, but reqwest exposes no
public way to remove its default Accept header. This is an irreducible wire
divergence unless explicitly accepted.

Malformed URL syntax fails Go `http.NewRequest`. Missing and unsupported schemes
can reach `Client.Do` and fail in transport. The probe found reqwest classify
malformed, missing-scheme, `file`, and `ftp` inputs as request-builder errors.
The package must define and test its own stable typed boundary rather than expose
reqwest classifiers; exact Go phase parity remains unproven.

### Redirects and sensitive headers

Go follows at most ten requests total in this path: the initial request plus up
to nine redirected requests. Reqwest's default `Policy::limited(10)` sent eleven
requests in the probe; `limited(9)` matches the count only.

For 301/302/303, both transports convert the oracle's POST to GET and remove the
body. For 307/308, owned bytes are replayable. Important policy still differs:

- Go forwards Authorization and explicit cookie headers to the same hostname
  and its subdomains, ignoring scheme and port. This includes a same-host port
  change and an HTTPS-to-HTTP redirect. Reqwest strips sensitive headers on any
  origin change. The probe reproduced the same-host/different-port difference.
- Reqwest's public redirect policy cannot restore headers already stripped by
  the middleware.
- A malformed `Location` makes Go close the redirect response and return a
  `*url.Error`. Tower HTTP returns the 3xx response unchanged when URI
  resolution fails, changing the adapter's error or status outcome.
- Go drains up to 2 KiB of a redirect response before closing it so the
  connection can be reused. A manual loop must preserve the required
  read/close/reuse ordering.

Exact parity therefore requires disabling automatic redirects and implementing
a package-owned redirect loop. Forwarding a bearer token across scheme changes
or to subdomains is also a security-sensitive oracle behavior and requires an
explicit controller acceptance, not an accidental middleware default.

### Proxy environment

Go 1.26 uses `HTTP_PROXY`, `HTTPS_PROXY`, and `NO_PROXY`; it supports HTTP,
HTTPS, `socks5`, and `socks5h` proxy URLs. The selected reqwest features omit
`socks`, so the provisional selection directly lacks an oracle capability.

Reqwest/hyper-util automatic environment proxy policy also differs:

- it recognizes `ALL_PROXY`, which Go ignores;
- in CGI, it disables all environment proxies, whereas Go errors only when an
  HTTP proxy applies and still permits `HTTPS_PROXY`;
- Go implicitly bypasses `localhost` and loopback IPs; hyper-util does not;
- Go supports port-qualified `NO_PROXY` entries; hyper-util ignores the
  destination port;
- Go treats `.example.com` as subdomain-only; hyper-util treats it like the
  exact domain plus subdomains; and
- Go skips an empty uppercase variable and tries lowercase; hyper-util stops at
  the first present uppercase variable even if empty.

Exact parity requires `.no_proxy()` plus a package-owned selector/matcher that
implements Go's precedence, CGI errors, implicit bypasses, CIDR/IP/domain/port
matching, proxy authentication, CONNECT, and SOCKS behavior. Directly disabling
`system-proxy` is insufficient in a shared Cargo graph because Cargo feature
unification can enable it elsewhere and add macOS/Windows OS proxy discovery.

### Retry, timeout, pooling, and gzip

Reqwest retries safe HTTP/2 protocol NACKs at most twice. Go's HTTP/2 transport
retries while its retry counter is at most six, with replay through `GetBody`
and backoff after the first retry. Reqwest's relevant classifier is private, so
the public retry builder cannot simply reproduce Go's policy. HTTP/2
`REFUSED_STREAM` and GOAWAY behavior was not executable-probed.

The blocking reqwest builder defaults to a 30-second overall timeout;
`.timeout(None)` is required for Go parity. A 30-second reqwest connect timeout
does not reproduce Go's separate 30-second dial and 10-second TLS-handshake
deadlines. Reqwest 0.13.4 source also defaults to TCP keepalive idle/interval/
retry values `15s/15s/3` and a 30-second `TCP_USER_TIMEOUT` on Linux, Android,
and Fuchsia. Go effectively uses `30s/15s/9` and no `TCP_USER_TIMEOUT`.

Reqwest defaults to a 90-second idle timeout and unlimited idle connections per
host; Go defaults to 90 seconds, two per host, and 100 globally. Reqwest exposes
the per-host control but no equivalent global maximum. A process-wide client is
necessary to obtain the intended shared pool. Its blocking implementation owns
an internal Tokio runtime thread and joins it on final drop.

For the oracle's ordinary POSTs, gzip insertion and transparent decompression
work. General default parity is false: Go suppresses auto-gzip for HEAD and Range
requests, while the resolved Tower HTTP 0.6.11 decompression service does not
inspect method or Range before inserting `Accept-Encoding`.

### Response ordering and typed errors

The loopback probe confirmed complete body consumption and reuse of one HTTP/1
connection. It also confirmed a truncated `Content-Length` body fails during
`Response::bytes()`. Reqwest classified that error as decode, not body. The
adapter must map every error returned while consuming bytes to `BodyRead`, not
rely on `Error::is_body()`.

After successful consumption, the adapter must capture status, then parse 401,
accept numeric `200..=300`, reject other statuses, and finally decode successful
JSON. It must not call `error_for_status()` or `StatusCode::is_success()`.
For 401, it must decode `AuthErrorResponse`, preserve the `data.noDomain` sentinel,
return the generic authentication failure otherwise, and keep malformed 401 JSON
separate from either authentication result. Response ownership/RAII must close
after the read on every path.

## Candidate comparison

Official crates.io metadata was checked on 2026-08-11. Adoption is an ecosystem
signal, not a security guarantee.

| Candidate | Primary metadata | Disposition |
| --- | --- | --- |
| [`reqwest` 0.13.4](https://docs.rs/reqwest/0.13.4/reqwest/) | 634,362,047 total downloads; 159,403,965 recent; 27,962 reverse dependencies; Rust 1.85; `MIT OR Apache-2.0` | Most adopted and still the most plausible transport, but **not approved** because its selected features and built-in policies fail parity. It also brings a Tokio runtime and AWS-LC native build. |
| [`ureq` 3.4.0](https://docs.rs/ureq/3.4.0/ureq/) | 175,384,473 total; 49,311,367 recent; 3,241 reverse dependencies; Rust 1.85; `MIT OR Apache-2.0` | Lighter natural blocking API, but no HTTP/2 and no demonstrated redirect/proxy advantage. Rejected. |
| [`isahc` 2.0.1](https://docs.rs/isahc/2.0.1/isahc/) | 16,925,653 total; 1,283,553 recent; 144 reverse dependencies; Rust 1.85; `MIT` | Libcurl coverage is broad, but the project is passively maintained and adds native/TLS platform complexity. Rejected. |
| [`curl` 0.4.50](https://docs.rs/curl/0.4.50/curl/) | 43,240,434 total; 4,051,367 recent; 295 reverse dependencies; `MIT` | Maintained low-level FFI bindings, not a high-level client; too much package policy and callback code. Rejected. |
| [`attohttpc` 0.31.0](https://docs.rs/attohttpc/0.31.0/attohttpc/) | 31,035,016 total; 5,733,489 recent; 78 reverse dependencies; undeclared MSRV; `MPL-2.0` | HTTP/1.1-only, weak-copyleft policy question, and no behavioral advantage. Rejected. |

## Reconsideration requirements for reqwest

The minimum feature line to investigate is not approved, but must include SOCKS:

```toml
reqwest = { version = "=0.13.4", default-features = false, features = ["blocking", "gzip", "http2", "rustls", "socks"] }
```

A future package probe must build one process-wide client and begin from explicit
configuration equivalent to:

```rust,ignore
reqwest::blocking::Client::builder()
    .tls_backend_rustls()
    .timeout(None)
    .connect_timeout(Some(std::time::Duration::from_secs(30)))
    .pool_idle_timeout(Some(std::time::Duration::from_secs(90)))
    .pool_max_idle_per_host(2)
    .tcp_keepalive(Some(std::time::Duration::from_secs(30)))
    .tcp_keepalive_interval(Some(std::time::Duration::from_secs(15)))
    .tcp_keepalive_retries(Some(9))
    .tcp_user_timeout(None)
    .user_agent("Go-http-client/1.1")
    .no_brotli()
    .no_deflate()
    .no_zstd()
    .no_proxy()
    .redirect(reqwest::redirect::Policy::none())
```

`tcp_user_timeout(None)` is target-specific and must be gated where the method is
available. Explicit TLS/coding/proxy calls are necessary because Cargo features
are additive: another workspace user can enable native TLS, `system-proxy`, or
extra content codings and otherwise change this client's runtime behavior.

The package must implement and executable-test all of the following before this
decision can be reconsidered:

1. A manual redirect loop covering 301/302/303 POST-to-GET, body-header removal,
   307/308 byte replay, relative/missing/malformed Location, Referer, exactly ten
   total requests, credential propagation to the same hostname/subdomains,
   stripping at disallowed destinations, and redirect-body drain/close ordering.
2. A manual Go-compatible proxy selector covering uppercase/lowercase precedence,
   empty-uppercase fallback, ignored `ALL_PROXY`, HTTP versus HTTPS, CGI behavior,
   implicit localhost/loopback bypass, port-qualified and leading-dot NO_PROXY,
   CIDR, IPv4/IPv6, proxy authentication, HTTPS CONNECT, and SOCKS5/5h DNS.
3. HTTP/2 ALPN and protocol-NACK/GOAWAY retry probes, with explicit controller
   acceptance if Go's retry count cannot be matched.
4. Wire-header characterization for User-Agent, unavoidable Accept, gzip, Host,
   Content-Length, Content-Type, and Authorization, with explicit acceptance of
   the Accept divergence.
5. Stable typed mapping for malformed request construction versus execution;
   exact oracle string concatenation for endpoint paths (including trailing- and
   double-slash cases); owned newline-terminated JSON bytes; and full response
   consumption before 401, numeric `200..=300`, status, and JSON handling.
6. Transparent gzip and uncompressed responses, truncated/chunk failures as
   `BodyRead`, transport failures as `Transport`, sequential record creation,
   partial accumulated output, and HTTP/1 connection reuse.
7. No overall/read timeout, characterized dial/TLS timeout mismatch, pool and
   keepalive settings, clean runtime-thread shutdown, and documented global-pool
   limitation.
8. Trusted and untrusted platform roots natively on Linux and macOS, plus Windows
   if supported; workspace-wide reqwest feature inspection; and fresh locked
   RustSec, license, MSRV, all-target, and native-build checks for the SOCKS graph.

Irreducible differences requiring explicit acceptance are reqwest's
`Accept: */*`, HTTP/2 protocol-NACK count/classifier, connect-versus-separate
dial/TLS deadline boundary, missing global idle cap, and non-POST gzip conditions.

## Verification performed

A focused crate outside the repository pinned the rejected four-feature
dependency, edition 2024, and `rust-version = "1.96"`. It verified:

- two oracle-shaped POSTs reused one HTTP/1 TCP connection;
- exact newline-terminated bodies and conditional Authorization;
- request status 200 and 300 availability and full-body reads;
- transparent gzip decompression;
- 301/302/303 conversion, 307/308 byte replay, and same-origin credentials;
- same-host/new-port credential stripping and the redirect-count mismatch;
- request-builder errors for malformed, missing, `file`, and `ftp` URLs; and
- truncated body failure from `Response::bytes()` with
  `is_body() == false` and `is_decode() == true`.

Commands and results:

```text
cargo +1.96.0 run --locked --offline
  pass on Linux outside the network sandbox

cargo +1.96.0 check --locked --offline --all-targets
  pass on Linux

cargo +1.96.0 check --locked --offline --target x86_64-pc-windows-gnu
  pass

cargo audit --no-fetch --deny warnings
  exit 0; 171 locked dependencies scanned
  advisory DB d0861df1eab469d3c58d6b836ce48b5766e5f217

cargo tree --locked -e features -i reqwest
  selected only blocking, gzip, http2, rustls and their internal features

cargo tree --locked -e normal,build
  confirmed permissive expressions and the AWS-LC/CMake native graph
```

Notable exact transitive versions were `hyper 1.11.0`, `hyper-util 0.1.20`,
`h2 0.4.15`, `tower-http 0.6.11`, `tokio 1.53.1`, `rustls 0.23.43`,
`rustls-platform-verifier 0.7.0`, `rustls-native-certs 0.8.4`,
`aws-lc-rs 1.18.0`, and `aws-lc-sys 0.44.0`. The `rustls` feature selects
AWS-LC and therefore compiles C/assembly through `cc` and CMake. Linux did not
link OpenSSL; `openssl-probe` only discovers CA locations. macOS adds
Security.framework/CoreFoundation dependencies and Windows adds `windows-sys`.

The platform verifier uses Linux native CA discovery plus webpki, macOS
Security.framework/keychains, and Windows platform verification. Reqwest's
tagged 0.13.4 CI covers macOS and Windows targets, but that does not replace the
native trust-store tests required above.

## Fresh adversarial review

A separate read-only researcher independently checked the exact base and
selection against the frozen Go 1.26 sources, reqwest/tower-http/hyper-util
0.13.4 graph sources, and executable probe results.

- **Reviewer verdict:** `BLOCK`
- **Reviewer workspace changes:** none
- **Agreement:** proxy behavior, SOCKS omission, redirect credential and
  malformed-Location behavior, retry limits, ambient headers, socket defaults,
  feature unification, native-platform gaps, and response error boundaries all
  prevent approval.
- **Affected package packet:** none exists at research time
- **Affected Go package:** `upstream/uncloud/internal/dns`
- **Direct caller:** `upstream/uncloud/internal/machine/cluster`

The decision remains `blocked` until the controller commissions and accepts a
new adapter design/probe or selects another dependency. No package may treat the
minimum SOCKS-enabled line above as approved.
