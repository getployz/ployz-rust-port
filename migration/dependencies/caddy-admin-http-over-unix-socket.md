# Dependency decision: `caddy-admin-http-over-unix-socket`

| Field | Value |
| --- | --- |
| Status | `approved` after a clean fresh adversarial re-review |
| Capability | Cancellable HTTP/1.1 access to Caddy's admin API over a filesystem Unix-domain socket |
| Selected | Reqwest `=0.13.4`; Async Compression `=0.4.43`; Futures Util `=0.3.34`; Rustls `=0.23.43`; Serde `=1.0.229`; Serde JSON `=1.0.151`; Tokio `=1.53.1`; Tokio Util `=0.7.19` |
| License | All selected crates are `MIT` or `MIT OR Apache-2.0`; the conservative capability/Reqwest-union scratch graph contains only permissive/compatible expressions |
| Research date | `2026-08-12` UTC |
| Base | `886327c72d1db25ca56a6c76c65a2cdc6c4ee2b9` (parent `b9454dfd94bf2c858a03813b3b9009661c6e0b04`) |
| Affected package | Future `crates/ployz-internal-machine-caddyconfig`, porting `upstream/uncloud/internal/machine/caddyconfig` |

## Verdict

Select Reqwest's asynchronous fixed-Unix-socket client, stream its raw response
chunks through Tokio Util into Async Compression's multi-member gzip decoder,
and use small custom Serde visitors for the two Go JSON envelopes. Install or
validate the workspace-required Rustls ring provider before constructing the
Reqwest client.

This is the strongest idiomatic ecosystem solution that preserves the frozen
observable contract. Reqwest owns HTTP/1.1, the fixed connector, pooling,
redirects, and request lifetime. Tokio owns availability and per-operation
deadlines. Async Compression performs yielding multi-member decoding rather
than blocking the runtime. The package owns only policy that a general client
cannot know: routes, headers, exact deadlines, response/error precedence, two
small JSON envelopes, and Go-compatible gzip behavior. It must expose domain
operations and typed errors, not Reqwest or a Go-shaped HTTP facade.

No caddyconfig owner result was available. `migration/TASKS.tsv` still marks
the package dependency-blocked, and the owner worktree contains no result
artifact. This decision therefore derives the contract directly from the
frozen oracle, reachable callers, current migration state, and runnable probes.
The controller—not this researcher—owns registries and routine approval.

## Exact observable contract

### Reachability and caller consequences

Repository-wide symbol tracing (excluding out-of-scope
`upstream/uncloud/experiment/**`) found one production constructor and these
operations:

- [`NewCaddyAdminClient`](../../upstream/uncloud/internal/machine/caddyconfig/client.go)
  is constructed by
  [`NewController`](../../upstream/uncloud/internal/machine/caddyconfig/controller.go).
- `IsAvailable` gates custom snippet validation and live loading. Unavailable
  Caddy still permits writing the generated base Caddyfile.
- `Validate` is `Adapt` with the successful result discarded. The generator
  calls it serially for global and service snippets and includes failures in a
  skipped-config comment.
- `Load` follows generation. Failure preserves the previous on-disk file and
  clears the successful fingerprint; success updates the fingerprint and file.
- There are no frozen direct client tests; generator/controller tests mock the
  validator. Thus [`client.go`](../../upstream/uncloud/internal/machine/caddyconfig/client.go)
  is the exact protocol oracle and callers establish reachability.

Caddy independently defines `POST /adapt` as adapting without running and
`POST /load` as replacing active configuration; `Content-Type` selects the
adapter/native JSON: [Caddy admin API](https://caddyserver.com/docs/api#post-adapt)
and [`POST /load`](https://caddyserver.com/docs/api#post-load).

### Availability

1. Independently connect to the configured **filesystem** Unix socket; do not
   consult or disturb the HTTP pool.
2. Bound `tokio::net::UnixStream::connect(path)` with exactly one second using
   `tokio::time::timeout`.
3. `Ok(stream)` is `true`; immediately drop the stream. Timeout and every I/O
   error are `false`, including missing path, permission denial, and a stale
   socket returning `ECONNREFUSED`.
4. The Go method has no context and exposes no error. The Rust future may be
   canceled by dropping its owner; no task may survive it.

Tokio documents that expiry cancels and drops the inner future without extra
cleanup: [`timeout`](https://docs.rs/tokio/1.53.1/tokio/time/fn.timeout.html).
Its Unix connector is supported on Unix:
[`UnixStream`](https://docs.rs/tokio/1.53.1/tokio/net/struct.UnixStream.html).
Abstract Linux addresses are not in scope: the shipped caller uses
`/run/uncloud/caddy/admin.sock`, and macOS has no abstract namespace.

The Linux probe covers available, missing, stale, and permission-denied paths
and verifies a one-second timer. It does **not** claim a reproducible stalled
pathname connect: attempts to synthesize backlog saturation produced immediate
`EAGAIN` on this kernel rather than a pending Tokio connect. The exact
`timeout(connect)` composition is source-backed and must remain a package test;
native release testing remains responsible for any OS-specific pending-connect
case.

### Shared HTTP policy, timeout, and lifecycle

Build one reusable client:

```rust
reqwest::Client::builder()
    .unix_socket(socket_path)
    .http1_only()
    .http1_title_case_headers()
    .no_proxy()
    .no_gzip()
    .no_brotli()
    .no_deflate()
    .no_zstd()
    .retry(reqwest::retry::never())
    .timeout(Duration::from_secs(5))
    .user_agent("Go-http-client/1.1")
    .build()
```

- `unix_socket` fixes every URL to the owned path and bypasses DNS/proxies;
  `http1_only` forbids HTTP/2/TLS negotiation:
  [Reqwest `ClientBuilder`](https://docs.rs/reqwest/0.13.4/reqwest/struct.ClientBuilder.html).
- Disable Reqwest's configurable Tower response/application retry policy.
  Reqwest warns that side-effecting requests must not be broadly retried and
  supplies `retry::never()`:
  [retry policy](https://docs.rs/reqwest/0.13.4/reqwest/retry/index.html).
- Retain Hyper-util's separate default stale pooled-connection recovery. It
  retries only a canceled request on a reused connection; Go's transport also
  retries a replayable POST on a reused connection when nothing was written,
  and `strings.Reader` supplies `GetBody`:
  [Go transport](https://go.dev/src/net/http/transport.go) and
  [Go request](https://go.dev/src/net/http/request.go). The probe closes a
  kept-alive Adapt connection before Load and confirms recovery with exactly
  one observed `/adapt` and one `/load`; no status/body retry is permitted.
- Preserve the default ten-hop redirect policy. Go also stops after ten and
  changes 301/302/303 to GET while retaining replayable POST bodies for 307/308:
  [Go client source](https://go.dev/src/net/http/client.go) and
  [Reqwest redirect policy](https://docs.rs/reqwest/0.13.4/reqwest/redirect/struct.Policy.html).
- `ClientBuilder::timeout(5s)` is required, but is not alone sufficient because
  manual decoding happens outside Reqwest. At the start of **each** `/adapt` or
  `/load` request, also capture `deadline = Instant::now() + 5s`; wrap `send`
  and every response/decode read in `timeout_at(deadline, ...)`. This makes the
  single per-request window cover connect, request, headers, raw body, and
  decoding, matching Go `http.Client.Timeout`:
  [Go timeout contract](https://go.dev/src/net/http/client.go#L90-L105).
  `Load` first runs `Adapt`, so it has two sequential five-second request
  windows, not a single ten-second operation wrapper.
- Convert `Response::bytes_stream()` with
  `tokio_util::io::StreamReader`; if `Content-Encoding` is exactly `gzip`, wrap
  it in `async_compression::tokio::bufread::GzipDecoder` and call
  `multiple_members(true)`. The decoder documents that it expects EOF or a
  further valid member after each member:
  [GzipDecoder](https://docs.rs/async-compression/0.4.43/async_compression/tokio/bufread/struct.GzipDecoder.html)
  and [StreamReader](https://docs.rs/tokio-util/0.7.19/tokio_util/io/struct.StreamReader.html).
- Read into a bounded buffer (probe: 8 KiB), explicitly yield between reads,
  and apply the same absolute deadline to each read. This prevents buffered
  decompression from monopolizing the executor and keeps timeout/drop
  cancellation effective. Do not collect compressed bytes and synchronously
  decode afterward; do not use `spawn_blocking` or a detached producer.
- The operation future owns request, response stream, decoder, and partial
  bytes. Dropping it cancels connect/header/body/decode work and closes the
  active connection. The reusable Reqwest pool needs no package worker or
  shutdown API. `Client` is internally pooled and cheaply cloneable:
  [`reqwest::Client`](https://docs.rs/reqwest/0.13.4/reqwest/struct.Client.html).

The actual workspace Reqwest union includes `rustls-no-provider`. Reqwest says
that building **any** client with this feature and no global crypto provider
panics, including an HTTP-only UDS client:
[Reqwest TLS backends](https://docs.rs/reqwest/0.13.4/reqwest/tls/).
Before the builder, run a small fallible package helper:

1. if a provider exists, accept it only when its secure-random and key-provider
   function pointers equal Rustls ring's provider;
2. otherwise install `rustls::crypto::ring::default_provider()`;
3. if installation loses a race, re-read and accept only a ring-compatible
   winner; and
4. return a typed construction error for an incompatible provider—never panic.

Docker uses the same policy privately, but caddyconfig cannot depend on Docker
construction order or its private helper. A direct Rustls dependency is
required. Fresh-process union probes build both caddy-first and Docker-style-
first successfully. Ring is already the workspace provider, so this introduces
no provider-feature or policy conflict.

For `Adapt`, any raw-read, decode, or deadline error is
`read response body: <source>` and discards the prefix. For non-200 `Load`,
retain decoded bytes produced before a raw-read, decode, or deadline error and
ignore that error, exactly matching `body, _ := io.ReadAll(resp.Body)`.
Caller drop still cancels the Rust future; no result can then be observed.

### Go wire defaults and gzip

The frozen transport automatically sets `Accept-Encoding: gzip` and
decompresses a gzip response because `DisableCompression` is false:
[Go transport](https://go.dev/src/net/http/transport.go). The Go wire oracle
observed:

```text
method=POST target=/adapt proto=HTTP/1.1 host=localhost body="example.test"
header User-Agent=["Go-http-client/1.1"]
header Content-Length=["12"]
header Content-Type=["text/caddyfile"]
header Accept-Encoding=["gzip"]
```

Set `Accept-Encoding: gzip` explicitly for both operations and decode only a
single `Content-Encoding` value ASCII-case-insensitively equal to `gzip`, as
Go's transport does. Call every Reqwest
`no_*` method to prevent automatic advertisement/double decoding under feature
unification. Do not enable Reqwest's `gzip` feature: the already-integrated
Docker raw client uses the same Reqwest package ID and its approved
[`lossless-docker-engine-progress-streaming`](lossless-docker-engine-progress-streaming.md)
decision forbids automatic compression.

Go's `gzip.Reader` defaults to multistream. It concatenates valid members and
reports trailing junk or a truncated later member **after** returning the valid
decoded prefix: [Go gzip reader](https://go.dev/src/compress/gzip/gunzip.go).
The Go oracle probe and Rust gate cover concatenated members, trailing junk,
and a truncated second member. Therefore `Adapt` rejects both malformed cases;
non-200 `Load` retains the valid prefix and continues status/error handling.

### `Adapt`

1. Send `POST http://localhost/adapt` through the fixed socket, with the
   Caddyfile bytes unchanged and exactly `Content-Type: text/caddyfile`.
2. Read/decode the complete body before status handling. Always release the
   response on success, error, timeout, or cancellation.
3. Exactly status 200 is success. Decode the top-level Go envelope
   `struct { Result json.RawMessage \`json:"result"\` }` and return the original
   byte spelling of the selected value. Borrowed Serde JSON `RawValue` retains
   the spelling: [`RawValue`](https://docs.rs/serde_json/1.0.151/serde_json/value/struct.RawValue.html).
4. Match Go 1.26 `encoding/json` for supported Caddy envelopes: top-level `null` is
   valid and yields empty result; object keys match exact spelling first and
   then folded spelling; folding is ASCII case-insensitive plus Unicode
   SimpleFold equivalence (`ſ` with `S/s`, `K` with `K/k`); later matching
   duplicate values win; missing is empty; explicit result `null` returns the
   raw bytes `null`; unknown fields are ignored. Any other top-level type or
   malformed JSON is `parse adapt response: <source>`.
5. On exactly 400, decode Caddy v2.8.4 `APIError`, whose member is `error`:
   [Caddy source](https://github.com/caddyserver/caddy/blob/v2.8.4/admin.go#L1306-L1318).
   Top-level `null` and missing member successfully yield an empty message.
   Matching is folded as above. Each string overwrites the previous string,
   while `null` leaves it unchanged; therefore
   `{"error":"first","error":null}` yields `first`. A matching non-string,
   non-null value fails envelope parsing even if an earlier value was valid.
6. Successful 400 extraction returns the bare message. If extraction fails,
   or for every other status, return the raw decoded body without status text.

Custom visitors are required because Serde derive rejects top-level `null`,
does not implement Go's field folding, and treats duplicate/null string fields
differently. Implement the two known folded comparisons locally; no general
Go-JSON abstraction or Unicode dependency is warranted. Typed errors should
retain status and raw body even when compatibility `Display` shows only body.

### `Load` and `Validate`

`Validate(caddyfile)` is `Adapt(caddyfile)` with success discarded and the same
error returned.

`Load(caddyfile)` must:

1. call `Adapt` first;
2. on failure return `adapt Caddyfile to JSON config: <error>` and send no load;
3. send `POST http://localhost/load`, body exactly Adapt's raw result bytes,
   and exactly `Content-Type: application/json`;
4. on status 200 return immediately without reading/interpreting its body;
5. otherwise read/decode while preserving partial bytes and ignoring any later
   raw-read/decode/deadline failure;
6. on status 400 and successful API-error extraction return
   `caddy responded with error: <message>`; and
7. otherwise return
   `caddy responded with error: HTTP <numeric-status>: <raw-decoded-body>`.

Typed variants must retain operation, status, raw bytes, and available source.
Reqwest and Go may phrase dependency-native transport/parser tails differently;
callers do not compare them. Stable prefixes, Caddy message/body bytes, status,
and precedence above may not change.

## Primary-source dependency evidence

### Reqwest `=0.13.4`

- Its exact manifest declares Rust 1.85 and `MIT OR Apache-2.0`; gzip, TLS,
  HTTP/2, system proxy, JSON, and stream are separately gated:
  [published manifest](https://docs.rs/crate/reqwest/0.13.4/source/Cargo.toml).
- Fixed Unix socket, total timeout, HTTP/1-only, title-case headers, proxy and
  compression controls, retry, and user agent are public builder APIs:
  [`ClientBuilder`](https://docs.rs/reqwest/0.13.4/reqwest/struct.ClientBuilder.html).
- Tag `v0.13.4` resolves to immutable commit
  `11489b34eda6d32b15ad4033e62beba2ee401350`, released 2026-05-25.
- Official crates.io data on 2026-08-12 reported 636,833,289 total and
  159,926,022 recent downloads with 28,013 reverse-dependency rows:
  [crate](https://crates.io/crates/reqwest). The repository remained active on
  2026-08-10.

### Async/stream/JSON stack

- Async Compression `=0.4.43` (`gzip,tokio`) supplies the Tokio `AsyncRead`
  gzip decoder and explicit multi-member mode, is `MIT OR Apache-2.0`, has
  MSRV 1.83, and tag commit
  `6fd95cff631d19157c4533b717939f25971d50b8c`. Crates.io reported 194,526,058
  total/44,097,114 recent downloads and 335 reverse dependencies. The release
  was 2026-07-29 and repository remained active:
  [project](https://github.com/Nullus157/async-compression) and
  [features/docs](https://docs.rs/async-compression/0.4.43/async_compression/).
  The resolved gzip backend is Flate2/Miniz's pure-Rust `rust_backend`; no
  system zlib or package-owned unsafe/FFI is introduced.
- Tokio Util `=0.7.19` (`io`) supplies `StreamReader`, is MIT/MSRV 1.71,
  released 2026-07-21, and crates.io reported 706,081,069 total/149,266,487
  recent downloads and 6,635 reverse dependencies:
  [Tokio Util](https://docs.rs/crate/tokio-util/0.7.19).
- Futures Util `=0.3.34` (`std`) supplies `TryStreamExt::map_err`, is
  `MIT OR Apache-2.0`/MSRV 1.71, tag commit
  `705e6b5c0f06535b1aac1cb1989a172b3d45be8c`, released 2026-08-11, and
  crates.io reported 809,410,000 total/196,042,293 recent downloads and 8,897
  reverse dependencies: [Futures](https://github.com/rust-lang/futures-rs).
- Rustls `=0.23.43` (`ring,std`) supplies the process-global provider API
  required by the workspace's providerless Reqwest build. It is
  `Apache-2.0 OR ISC OR MIT`, MSRV 1.71, already locked/feature-enabled by the
  workspace, and its provider installation is one-time and race-safe:
  [Rustls cryptography providers](https://docs.rs/rustls/0.23.43/rustls/crypto/struct.CryptoProvider.html).
- Tokio `=1.53.1` (`net,time,io-util`) is the workspace runtime and supports
  Linux/macOS through Mio's epoll/kqueue backends:
  [Tokio](https://github.com/tokio-rs/tokio) and
  [Mio platforms](https://github.com/tokio-rs/mio#platforms).
- Serde `=1.0.229` (`std`) and Serde JSON `=1.0.151` (`raw_value,std`) are
  current, permissive, heavily adopted, and keep the wire JSON private to two
  visitors. Exact MSRVs are 1.56 and 1.71 respectively.

All selected crate manifests and source/license files were inspected from the
registry. The maximum selected MSRV is Reqwest's 1.85, below workspace Rust
1.96.

## Candidate comparison

Official adoption values are snapshots from 2026-08-12.

| Candidate | Hard-gate result | Decision |
| --- | --- | --- |
| **Reqwest 0.13.4 + selected async stream stack** | Built-in fixed UDS, pooled cancellable async HTTP, explicit HTTP/timeout/retry policy, raw streaming, current MSRV. Async multi-member decode preserves complete deadline/cancellation and partial-prefix rules. | **Selected.** Strongest idiomatic facade and narrowest passing seam. |
| Reqwest automatic `gzip` | Its decoder is idiomatic and streaming, but enabling its Cargo feature would unify automatic compression into the Docker raw client whose approved contract forbids it. It also hides the `Load` partial-error seam. | Rejected for workspace behavior leakage. |
| Reqwest + synchronous Flate2 | Popular and permissive, but `GzDecoder` is single-member and post-collection synchronous decoding falls outside Reqwest's deadline/cancellation. `MultiGzDecoder` fixes only the first problem. | Rejected at gzip and lifecycle gates after adversarial finding. |
| Hyper 1.11 + hyper-util 0.1.20 + hyperlocal 0.9.1 + body utilities | Behavior-capable, but package would own client/pool/redirect/body execution and more protocol error surface. Hyperlocal 0.9.1 was last released 2024-07-22 and had only 55 reverse-dependency rows. | Rejected on integration complexity and privileged surface; fallback if Reqwest drops fixed UDS. |
| Ureq 3.4.0 custom transport | Maintained/popular (176.2M downloads, 3,247 reverse rows), but Unix transport uses its explicitly unversioned API and blocking cancellation would require supervised threads/socket interruption. | Rejected at cancellation/lifecycle and architecture gates. |
| `hyper_socket` 0.2.0 | 2019 pre-Hyper-1.0 connector with 5,956 total/54 recent downloads. | Rejected obsolete/unmaintained/incompatible. |
| Raw Tokio HTTP parser or `curl --unix-socket` | Could reach the socket, but respectively owns unsafe protocol surface or adds an external executable and changes errors, pooling, cancellation, and portability. | Rejected at architecture/security/behavior gates. |

## Exact selected integration and feature union

Root manifests/lockfiles remain integrator-owned. Production dependency lines:

```toml
async-compression = { version = "=0.4.43", default-features = false, features = ["gzip", "tokio"] }
futures-util = { version = "=0.3.34", default-features = false, features = ["std"] }
reqwest = { version = "=0.13.4", default-features = false, features = ["stream"] }
rustls = { version = "=0.23.43", default-features = false, features = ["ring", "std"] }
serde = { version = "=1.0.229", default-features = false, features = ["std"] }
serde_json = { version = "=1.0.151", default-features = false, features = ["raw_value", "std"] }
tokio = { version = "=1.53.1", default-features = false, features = ["io-util", "net", "time"] }
tokio-util = { version = "=0.7.19", default-features = false, features = ["io"] }
```

Probe/test-only Tokio features are `macros,rt,sync`. Do not enable Reqwest
`gzip`, `json`, defaults, `http2`, or `system-proxy`.

The base workspace already resolves Reqwest 0.13.4 with effective features
`query,rustls-no-provider,stream` through `ployz-internal-docker`. Selecting
`stream` adds **no new Reqwest feature** to that union. `query` only exposes
request query serialization and is unused here. `rustls-no-provider` compiles
TLS support but does not route this client through TLS: fixed Unix socket,
`http://localhost`, and `http1_only()` remain explicit. The `no_*` calls prevent
future compression feature union from changing wire/decode behavior.

The base also already resolves Async Compression 0.4.43 with exactly
`gzip,tokio` through `ployz-internal-corrosion`, Tokio Util 0.7.19 with `io`
among its effective workspace features, and Futures Util 0.3.34 with `std`.
Thus this selection adds no new version and no new effective feature for those
shared package IDs; it authorizes direct use only within the caddyconfig seam.
Rustls 0.23.43 is also already resolved with `ring,std,tls12` among its
effective features. The direct caddy request adds no version or feature.

Workspace `cargo tree -e features -i reqwest@0.13.4` established the actual
Reqwest union. The separate audit scratch modeled exactly that Reqwest union
plus the selected caddy dependencies and the probe/test Tokio feature superset.
Its lock contains 149 registry packages (150 locked dependencies), all with
permissive/compatible license expressions, and RustSec reported no
vulnerability. Whole-workspace `cargo audit` separately
reports pre-existing `RUSTSEC-2023-0071` in `rsa` via `russh` and an allowed
unmaintained warning for `paste` via netlink. Neither appears in the capability
union scratch graph or is introduced/reached by this decision.

## Hard gates and accepted limitations

| Gate | Evidence | Result |
| --- | --- | --- |
| Behavior | Frozen oracle/callers, Go wire/JSON/gzip characterization, and real-socket Rust harness cover exact requests, result/error extraction, status/body precedence, multi-member gzip, malformed later members, and partial reads. | `pass` |
| Timeout/cancellation/lifecycle | Absolute per-request deadline spans send/raw/decode; bounded async decode yields; header/plain-body/gzip-body timeout and future-drop tests observe peer EOF while the client stays alive; no package worker. | `pass` |
| Platforms | Behavior seam compiles for Linux/macOS x86_64/aarch64; the exact selected graph resolves all four; native Linux x86_64 real UDS/provider runs; Reqwest/Tokio/Rustls-ring source supports macOS/Linux. | `pass`; exact-union macOS compile/runtime and Linux arm64 runtime remain native release acceptance |
| License/security | Direct manifests permissive; capability/actual-Reqwest-union graph permissive and RustSec-clean; fixed UDS/no proxy/DNS/TLS/application retry; ring provider validated without panic; no package unsafe. | `pass` |
| Maintenance/Rust | Current releases; very high adoption for HTTP/runtime/stream stack; Async Compression actively maintained; max MSRV 1.85 vs Rust 1.96. | `pass` |
| Architecture | Domain-only facade; established crates own UDS/HTTP/stream/gzip/JSON; package policy remains narrowly oracle-specific. | `pass` |

Accepted limitations:

- Native runtime was Linux x86_64. The behavior seam compiles and the exact
  selected graph resolves for all four targets. This Linux host cannot fully
  cross-link Rustls ring for Darwin without a macOS SDK, so exact-union macOS
  amd64/arm64 compile/runtime and Linux arm64 runtime remain native
  package/release acceptance.
- The one-second policy and real connect outcomes were tested, but a genuinely
  pending pathname connect was not reproducible on this kernel; backlog
  saturation returned immediate `EAGAIN`. The production composition still
  wraps the real `UnixStream::connect`, not `pending()`.
- Reqwest/Hyper and Go can phrase transport/parser errors differently and have
  different malformed-peer limits. Preserve typed sources and the stable
  operation/status/body/prefix precedence. Supported peer is Caddy.
- Go replaces invalid UTF-8/unpaired JSON surrogates and caps nesting at
  10,000, while Serde JSON requires UTF-8 and has different depth behavior.
  Caddy emits valid, shallow UTF-8 JSON. Exact supported envelope parity is the
  null/folding/duplicate/type/raw-value contract above; malformed-Unicode/depth
  diagnostics are an accepted malformed-peer limitation.
- Rust error display is UTF-8; retain fallback bodies as `Vec<u8>`. Valid Caddy
  JSON displays byte-for-byte; invalid UTF-8 display may be escaped/lossy while
  the accessor retains exact bytes.
- Bodies remain unbounded because the oracle uses `io.ReadAll`. The peer is the
  local permissioned admin socket; adding a bound requires a new behavior decision.

Package acceptance must turn the probe into durable tests for available,
missing, stale, permission-denied sockets; exact timeout composition; exact
wire headers/body; JSON missing/null/folding/duplicates/wrong types; all status
paths; multi-member/trailing-junk/truncated gzip; Adapt read precedence; Load
partial-prefix behavior and no-load-on-adapt-error; 200 unread load body;
header/plain/gzip cancellation and five-second deadlines; peer EOF/no orphan;
client reuse; all four behavior target checks; and native exact-union checks on
each release platform. If a deterministic stalled UDS
connect can be constructed in package CI, add it without changing policy.
Also test fresh-process caddy-first/Docker-first provider order and incompatible
provider construction failure without panic.

## Runnable verification

Research scratch is outside the repository because this task may commit only
this record:

```text
/tmp/caddy-admin-http-unix-probe-886327c7/
/tmp/caddy-admin-http-unix-union-audit-886327c7/
/tmp/caddy-admin-go-wire-probe-886327c7/main.go
/tmp/caddy-admin-go-json-probe-886327c7/main.go
/tmp/caddy-admin-go-gzip-probe-886327c7/main.go
```

Rust real-socket gate:

```sh
cd /tmp/caddy-admin-http-unix-probe-886327c7
cargo generate-lockfile
cargo fmt --all -- --check
cargo run --locked
cargo clippy --locked --all-targets -- -D warnings
cargo check --locked --target x86_64-unknown-linux-gnu
cargo check --locked --target aarch64-unknown-linux-gnu
cargo check --locked --target x86_64-apple-darwin
cargo check --locked --target aarch64-apple-darwin
cargo audit --file Cargo.lock
```

Confirmed output:

```text
confirmed: UDS outcomes/permission, exact wire, Go JSON/gzip edges, partial-body precedence, header/plain/gzip cancellation, header/plain/gzip five-second deadlines
```

The harness covers real filesystem UDS and permission denial; exact `/adapt`
and `/load` HTTP/1.1 wire; raw result spelling; Go top-level null, folded keys,
duplicates/null/wrong types; all status/error paths; concatenated/malformed
gzip; raw/gzip partial precedence; cancellation with peer EOF; and five-second
header/plain-body/gzip-body deadlines. The client remains alive until peer EOF,
and stale pooled-connection recovery is separately covered. Connect timeout is
source-backed but not reported as runtime-probed for the reason above.

Effective feature-union audit:

```sh
cd /tmp/caddy-admin-http-unix-union-audit-886327c7
cargo generate-lockfile
cargo tree -e features -i reqwest@0.13.4
cargo run --locked -- caddy-first
cargo run --locked -- docker-first
cargo run --locked -- incompatible
cargo audit --file Cargo.lock
cargo metadata --locked --format-version 1
```

The Go 1.26.1 wire/JSON/gzip probes run with `mise exec -- go run main.go` in
their directories. The untouched oracle package passed:

```sh
cd upstream/uncloud
mise exec -- go test ./internal/machine/caddyconfig
# ok github.com/psviderski/uncloud/internal/machine/caddyconfig
```

Final verified scratch SHA-256 values:

```text
0b079c13e9bf3b47537689fb2e96e0de9c117a7def6f47758565fa517b64e5f6  Rust probe Cargo.toml
693ebfb3dba645cf111253f35b506594238b7835f6e1494dabaa8691d3633628  Rust probe Cargo.lock
fbb2898b304e573ec59b58eeab4b3921fc395b50fb4e5d3a8bb4159d8e1447eb  Rust probe src/main.rs
651edf67bd89ee1d271c4d81db4593feeb15774d0b14b151cdc43adc75f36feb  union audit Cargo.toml
39f563e48ad402a4cad8e119f68ca649b79d5cb844989565bc31c57615178c51  union audit Cargo.lock
2e4f6a1239ded01a76d86558cd089892d2e79a83a44623e834212285dcd5dac4  union audit src/main.rs
5be18b7b83d24d0962b81551540441c747b69ebe55bb42bbc772badfb978e05d  Go wire main.go
48c2f8601f4857f4e79c6001ddc3e05ee0ea6fbf938d8777e15ca8f5a86fd837  Go JSON main.go
9db8699f33580b68517e3641631bf19185110d29fa687d3dfb3436d65100f121  Go gzip main.go
```

## Review

The initial fresh adversarial reviewer found five issues: single-member gzip;
synchronous decoding outside timeout/cancellation; incomplete Go JSON null,
Unicode-fold, and duplicate-null rules; missing actual Reqwest feature-union
analysis; and overstated probe coverage. All were fixed by the async
multi-member design, Go characterization, union audit, added gates, and narrowed
connect claim.

A different fresh read-only adversarial reviewer then challenged behavior,
platforms, security, licensing, maintenance, provider ordering, feature-union
safety, and the narrow seam against the revised record and scratch evidence.
Final result: **CLEAN — no findings**. The reviewer reproduced the behavior
run/format/Clippy and four behavior target checks; fresh-process caddy-first,
Docker-first, and incompatible-provider union cases; Linux exact-union checks;
the 150-dependency clean RustSec audit; all Go probes; and the frozen package
test. It confirmed only this record was untracked, upstream had no diff, and it
made no edits. Exact-union native macOS and Linux arm64 acceptance remains
explicit above. This satisfies the required networking/privileged-API review;
routine approval is controller-delegated and registries remain untouched.
