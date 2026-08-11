# Dependency decision: `http-server-runtime`

| Field | Value |
| --- | --- |
| Status | `blocked` — the Hyper/Tokio family remains the best conditional candidate, but three behavior-parity authority gates and the cross-record compression conflict below remain unresolved |
| Capability | Production plain-HTTP server/runtime and gzip response compression for `ployz-internal-machine-metrics` |
| Selected dependency | Conditional stack: Hyper `1.11.0`, hyper-util `0.1.20`, http-body-util `0.1.4`, Tokio `1.53.1`, tokio-util `0.7.19`, and flate2 `1.1.9` |
| License | `MIT` for the Hyper/Tokio crates; `MIT OR Apache-2.0` for flate2 |
| Research date | `2026-08-11` UTC |
| Request | Delegated capability request; no request file exists at integration base |
| Integration base | `a47272fab9ecef37b513d0ad8a47c81c75f86dc4` |
| Prior proposal | `1ab24e45c0d97f16d1f51a2f8db1281db089fa2e`, inspected with `git show`; its mandatory adversarial review rejected the decision |
| Fresh adversarial review base | `f927d1bf224142754bc2f818b88fdb46d7d70686` |
| Affected package | `ployz-internal-machine-metrics` (`upstream/uncloud/internal/machine/metrics`) |

This revision does **not** approve dependencies for implementation. A fresh
adversarial review confirmed that direct Hyper/Tokio is the best family and
that the earlier fixes for connection isolation, blocking work, features, and
task ownership are directionally correct. It also found that the proposed
shutdown model still interrupted Go's accept backoff and closed never-active
connections too early. The corrected requirements and the new blocker are
recorded below.

## Oracle and primary-source evidence

- [`server.go`](../../upstream/uncloud/internal/machine/metrics/server.go) joins
  the supplied concrete IPv4 or IPv6 address with fixed port `51090`, registers
  only the methodless `/metrics` ServeMux pattern, sets `ReadTimeout` to five
  seconds, binds before observing cancellation, serves in a goroutine, and on
  cancellation returns a fresh five-second `Shutdown` result.
- [`cluster.go`](../../upstream/uncloud/internal/machine/cluster.go) runs this
  server inside the controller's `errgroup`; an outer server error cancels
  sibling work. Individual connection failures do not enter that error path.
- Go 1.26.1 [`net/http.Server`](https://github.com/golang/go/blob/go1.26.1/src/net/http/server.go)
  is the runtime oracle. `readRequest` installs one absolute `ReadTimeout`
  deadline for headers plus body; `IdleTimeout` defaults to `ReadTimeout`; the
  keep-alive loop grants the idle period, waits for four bytes, clears that
  deadline, and starts a fresh request deadline. `Serve` retries temporary
  accepts with exponential delay and runs every connection in its own recovered
  goroutine. Its backoff sleep is not cancellation-aware. `Shutdown` closes
  listeners, waits for every `Serve` listener goroutine, closes idle
  connections, and waits for active handlers without cancelling them. A
  never-active connection remains `StateNew` until the five-second age/read
  boundary instead of being closed as idle immediately.
- Go 1.26.1 [`ServeMux`](https://github.com/golang/go/blob/go1.26.1/src/net/http/server.go)
  cleans ordinary request paths, preserves queries in redirects, matches path
  segments after unescaping, and does not impose a method restriction on a
  methodless pattern. [`promhttp.Handler` 1.22.0](https://github.com/prometheus/client_golang/blob/v1.22.0/prometheus/promhttp/http.go)
  gathers, encodes, and compresses synchronously and has no request-method gate.
- Hyper's HTTP/1 [`Builder`](https://docs.rs/hyper/1.11.0/hyper/server/conn/http1/struct.Builder.html)
  documents `header_read_timeout` as header-only. Its source starts the timer
  when it polls a new request head and clears it when that head completes; it
  does not extend the timer through the request body or reproduce Go's separate
  keep-alive idle/reset phases.
- Hyper HTTP/1 [`Connection::graceful_shutdown`](https://docs.rs/hyper/1.11.0/hyper/server/conn/http1/struct.Connection.html#method.graceful_shutdown)
  disables keep-alive. The exact
  [dispatcher source](https://docs.rs/hyper/1.11.0/src/hyper/proto/h1/dispatch.rs.html#91-99)
  closes a connection in the initial read/write state immediately, unlike Go's
  `StateNew` shutdown treatment.
- Tokio documents that [`spawn_blocking`](https://docs.rs/tokio/1.53.1/tokio/task/fn.spawn_blocking.html)
  uses additional blocking threads even on a current-thread runtime and that a
  started blocking task cannot be aborted. [`JoinSet`](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinSet.html)
  aborts remaining tasks on drop and provides `detach_all` when tasks must
  continue independently.
- Tokio's [`select!`](https://docs.rs/tokio/1.53.1/tokio/macro.select.html)
  checks ready branches in pseudo-random order by default, and
  `TcpListener::accept` is cancellation-safe. An unbiased accept/cancellation
  race can therefore preserve Go `select`'s lack of fixed precedence without
  losing an accepted stream.
- [`CancellationToken`](https://docs.rs/tokio-util/0.7.19/tokio_util/sync/struct.CancellationToken.html)
  is in tokio-util's unconditional `sync` module. The published
  [0.7.19 manifest](https://docs.rs/crate/tokio-util/0.7.19/source/Cargo.toml)
  confirms that its `rt` feature is for task utilities and is unnecessary for
  this type.
- flate2 [`1.1.9`](https://docs.rs/crate/flate2/1.1.9) supplies gzip with its
  pure-Rust `miniz_oxide` backend. The frozen tree never imports
  `prometheus/promhttp/zstd`, so `promhttp`'s optional zstd writer remains unset
  and the actual oracle offers identity and gzip, not zstd.
- Official crates.io records for
  [Hyper](https://crates.io/api/v1/crates/hyper),
  [hyper-util](https://crates.io/api/v1/crates/hyper-util),
  [http-body-util](https://crates.io/api/v1/crates/http-body-util),
  [Tokio](https://crates.io/api/v1/crates/tokio),
  [tokio-util](https://crates.io/api/v1/crates/tokio-util), and
  [flate2](https://crates.io/api/v1/crates/flate2) on 2026-08-11 reported all
  six exact versions as current releases. Their declared MSRVs are 1.61 through
  1.71, and their aggregate adoption is large (Hyper 829.2M, Tokio 868.4M, and
  flate2 609.5M total downloads), supporting the selected family's maintenance
  and adoption position.

## Go 1.26.1 characterization

Focused tests ran with `/opt/go1.26.1/bin/go` rather than the VM's older default
Go. Durations below scale the oracle's five seconds to 200 ms without changing
the state machine.

| Behavior | Observed result |
| --- | --- |
| Slow request head | Connection closed at about 200 ms. |
| Complete head plus incomplete body | Handler output was not received until the same absolute 200 ms read deadline expired. |
| Head delayed 120 ms, then incomplete body | Response completed at about 201 ms from request start, proving head and body share one budget rather than receiving independent budgets. |
| Idle keep-alive | Connection closed at about 200 ms with no next request. |
| Next request after 150 ms idle | Sending the first four bytes at 150 ms and completing the head another 150 ms later succeeded at about 302 ms, proving the fresh post-idle request budget. |
| Temporary accepts | Injected `ECONNRESET`/`ECONNABORTED` attempts occurred after `5, 10, 20, 40, 80, 160, 320, 640, 1000` ms; the next fatal sentinel propagated. |
| Cancellation during accept backoff | During the 320 ms sleep, `Shutdown` with a 50 ms context returned success only after about 320 ms. `listenerGroup.Wait` is before the context-aware drain loop, so backoff is not interrupted and the context is not a strict wall-clock bound for this phase. |
| Never-active connection at shutdown | With a 300 ms `ReadTimeout`, a 150 ms shutdown context returned `DeadlineExceeded`; the silent `StateNew` connection remained open until about the original 300 ms read boundary. Hyper's direct graceful call closed the corresponding initial connection immediately. |
| Bind families | Concrete IPv4 and IPv6 addresses bound without wildcard substitution. |

The real Go 1.26.1 ServeMux/promhttp socket matrix produced:

| Request | Result |
| --- | --- |
| `GET`, `HEAD`, `POST`, `PUT`, or `OPTIONS /metrics` | `200`; HEAD has no response body |
| any method on `/metrics?x=1` | same `/metrics` handler; query is ignored for routing |
| `/metrics/` or unrelated path | `404` |
| `/foo/../metrics?x=1`, `//metrics`, or `/metrics/.` | `307 Temporary Redirect` to cleaned `/metrics`, preserving the query |
| `/m%65trics` | `200`; ServeMux unescapes within a path segment before matching |
| `/metrics%2F` or `/metrics%3Fx` | `404`; escaped separators/data do not become an exact `/metrics` path |
| `CONNECT /metrics` or `CONNECT /m%65trics` | matched; a methodless pattern also accepts CONNECT and segment unescaping still applies |
| `CONNECT /foo/../metrics?x=1` | `404`; CONNECT paths are not canonicalized |
| `OPTIONS *` | server-level `200` with an empty body, before ServeMux |

The Rust router must reproduce this matrix. A raw
`request.uri().path() == "/metrics"` test is insufficient because it misses
canonical redirects and segment unescaping.

The frozen promhttp compression probe additionally observed identity for an
empty header, explicit `identity`, zstd-only, wildcard, and even an
all-recognized-encodings-`q=0` request; gzip for `gzip` and for `zstd, gzip`;
and identity when it outranked gzip. This compatibility fallback is deliberate:
the Rust adapter must not add zstd or return `406` when promhttp falls back to
identity.

## Hard gates

| Gate | Requirement | Evidence and disposition | Result |
| --- | --- | --- | --- |
| Behavior | Exact bind/accept/route/concurrency/cancellation/drain/error behavior | Direct Hyper/Tokio exposes the required low-level controls, and the ownership model below fixes accept retry and connection isolation. Public Hyper does **not** expose Go's full read-deadline or new/idle connection state machines, and Tokio listener drop cannot report close failure. | `fail` pending the three explicit authority gates |
| License and security | Permissive licenses; no unnecessary TLS/auth/C runtime; current vulnerability scan | Direct manifests declare MIT or MIT/Apache-2.0; every resolved transitive license is permissive. Only HTTP/1 and flate2's exact pure-Rust `miniz_oxide` backend are enabled. A 1,211-advisory RustSec scan of the exact 35-package cross-target probe lock found no vulnerabilities or informational warnings. Re-run against the integrated lock. | `pass` |
| Platforms and targets | Linux, macOS, and Windows; concrete IPv4/IPv6; no wildcard substitution | Tokio officially supports all three platforms; `SocketAddr` and `TcpListener::bind` cover both families. Rust 1.96 target checks passed for Linux, Intel macOS, and Windows GNU. Native bind/runtime probes exercised IPv4/IPv6 only on Linux, so macOS/Windows runtime claims remain package-acceptance obligations rather than research-time results. | `pass` for the dependency; native non-Linux verification still required |
| Maintenance and Rust version | Popular, maintained current versions compatible with Rust 1.96 | Current releases were revalidated on 2026-08-11. Declared rust-version values are Hyper 1.63, hyper-util 1.64, http-body-util 1.61, Tokio/tokio-util 1.71, and flate2 1.67; the maximum is below the project's Rust 1.96. | `pass` |
| Architectural constraints | Natural dependency APIs, bounded private accept loop, no Go-shaped compatibility server | Hyper is deliberately a low-level building block. One HTTP/1 connection builder, Tokio listener, per-connection tasks, and full response bodies are the smallest stack that keeps every relevant control point visible. | `pass`, conditional on the three behavior gates |

## Candidate comparison

Exact latest versions were revalidated from the projects' published docs.rs
records on `2026-08-11`. Adoption figures from the official crates.io API are
point-in-time comparison evidence.

| Candidate | Disposition | Reason |
| --- | --- | --- |
| **Hyper `1.11.0` + Tokio `1.53.1`** | **Selected family, blocked** | The most widely adopted low-level family and the only credible option that leaves bind, accept classification, per-connection supervision, graceful signaling, and fatal exit visible. It still needs the explicit read-deadline, new-connection shutdown, and listener-close resolutions below. |
| Axum `0.8.9` | Rejected | `axum::serve` intentionally offers minimal configuration, retries accept failures internally, and has unbounded graceful shutdown. Axum routing adds Tower/framework cost but cannot repair the low-level parity gaps. |
| Actix Web `4.14.1` | Rejected | Its first-request-head timeout and worker shutdown/force-drop result do not match Go's request/body/keep-alive deadlines or returned shutdown error, and it brings a separate runtime/server stack. |
| Warp `0.4.3` | Rejected | Its high-level server does not expose the required bounded drain, accept error classification, or read-deadline model. Supplying a custom listener and dropping to Hyper recreates the selected stack with an extra filter layer. |
| axum-server `0.8.0` | Rejected | Published server code retries accept errors and converts forced graceful expiry to normal completion, losing required fatal/shutdown results. |
| Poem `3.1.12` | Rejected | Its idle/graceful controls do not reproduce the required absolute request-read deadline and fatal/error ownership, with materially lower adoption. |
| flate2 `1.1.9` | Selected for gzip | Current, highly adopted, no async runtime, supports gzip directly, and `rust_backend` avoids a C/system-library dependency. Compression still runs in `spawn_blocking`. |

The selected family remains preferable to a high-level server even while the
decision is blocked: none of the rejected servers fixes the explicit parity
gate, and each hides additional lifecycle behavior.

## Selected integration

### Exact versions and minimal features

These declarations are conditional guidance for the integrator after approval;
this decision does not edit a manifest:

```toml
flate2 = { version = "=1.1.9", default-features = false, features = ["rust_backend"] }
http-body-util = { version = "=0.1.4", default-features = false }
hyper = { version = "=1.11.0", default-features = false, features = ["http1", "server"] }
hyper-util = { version = "=0.1.20", default-features = false, features = ["tokio"] }
tokio = { version = "=1.53.1", default-features = false, features = ["macros", "net", "rt", "time"] }
tokio-util = { version = "=0.7.19", default-features = false }
```

Do not enable `full`, HTTP/2, TLS, authentication, signals, a C compression
backend, hyper-util `server-graceful`, or tokio-util `rt`. `CancellationToken`
needs no tokio-util feature. Tokio `sync` is already activated transitively by
tokio-util's published Tokio dependency; the probe also proved it need not be a
direct feature. hyper-util needs only `tokio` for `TokioIo` and `TokioTimer`;
Hyper itself owns the HTTP/1 server and graceful-connection API. Tokio `macros`
is required by the unbiased `select!` accept/cancellation race and by the
current-thread test; it is not an accidental convenience feature.

The minimized pre-compression lock contained 29 packages including the probe;
adding flate2's pure-Rust gzip path produced 35. The prior proposal's
tokio-util `rt` and redundant hyper-util server features enlarged the graph
without enabling a required API.

### Required accept and connection model

1. Construct `SocketAddr::new(supplied_ip, 51090)` and await
   `TcpListener::bind` **before** reading the cancellation token. An already
   cancelled caller must still receive `AddrInUse` or another bind error.
2. Configure Hyper HTTP/1 only with `TokioIo` and `TokioTimer`. Do not enable
   h2c. The five-second Hyper header timer is only a partial mitigation and may
   be enabled only after the deadline exception gate is explicitly accepted or
   a faithful replacement is proved.
3. The accept loop classifies only `io::ErrorKind::ConnectionReset` and
   `ConnectionAborted` as the Go accept-side transient cases. Log each failure;
   sleep 5 ms then double to 10, 20, 40, 80, 160, 320, 640 ms and cap at one
   second; reset the delay after every successful accept. Any other accept
   error is returned with its source intact. Cancellation must close/drop the
   listener promptly but must **not** erase the remaining Go-equivalent backoff
   wait before server completion; the shutdown deadline starts when cancellation
   is selected, not after that wait.
4. For each accepted stream, create a child/clone of the server
   `CancellationToken`, build a Hyper connection future, and spawn an **inner**
   connection task. Spawn a small supervisor for that inner task into a
   `JoinSet<()>`. The supervisor logs `Ok(Err(hyper_error))` and
   `Err(JoinError)` (including panics) and returns normally. The accept loop
   reaps supervisors but treats every supervisor outcome as per-connection and
   nonfatal. No Hyper protocol error or connection/service panic is returned
   from the server.
5. Pin the inner Hyper connection. Once the new/idle-state gate below is
   resolved, apply its authorized state-aware shutdown action and continue
   awaiting the connection. Calling `graceful_shutdown()` indiscriminately is
   rejected because it closes a never-active connection immediately. If the
   outer server exits for a fatal listener/control error, do **not** cancel any
   connection token; existing connections continue as they do after Go `Serve`
   returns a fatal accept error.

The nested inner task is deliberate: after a shutdown timeout or fatal accept,
the supervisor can be detached yet still await and log an inner connection
panic. A bare detached Hyper task would silently discard its `JoinError`.

### Routing and synchronous metrics work

- Implement the Go 1.26 ServeMux matrix above, including methodless matching,
  query-independent `/metrics`, segment unescaping, `307` path cleaning with
  query preservation, the CONNECT no-clean exception, `OPTIONS *`, exact
  trailing-slash behavior, and HEAD body suppression. Do not substitute an Axum
  method router with different redirects.
- For a matched scrape, clone only the owned registry/request negotiation data
  needed by a `'static` closure and run **registry gather, selected encoder,
  gzip compression, and final byte-buffer construction together** inside
  `tokio::task::spawn_blocking`. Await that handle asynchronously, map/log its
  `JoinError` as a request/connection failure, and return a full in-memory body.
  Do not gather on the runtime thread and offload only compression afterward.
- The required package test must use `#[tokio::test(flavor = "current_thread")]`:
  hold one synthetic gather/encode/gzip closure for at least 250 ms and prove a
  concurrent fast request completes within 75 ms. The focused dependency probe
  passed exactly this test. Also test two concurrent scrapes, not just a 404,
  to catch accidental serialization outside the Prometheus registry itself.
- Started `spawn_blocking` work cannot be aborted. This matches Go's active
  handler continuation on shutdown timeout; detached supervisors must remain
  able to log its eventual outcome.

### Every server exit and task-ownership path

| Path | Required ownership and result |
| --- | --- |
| Bind failure, including pre-cancelled token | No tasks exist; return the bind error with source intact. |
| Successful connection | Supervisor observes `Ok(Ok(()))`; accept loop reaps it; no server result. |
| Connection protocol/service error | Supervisor logs `Ok(Err(_))`; isolate and continue accepting. |
| Connection panic/cancellation `JoinError` | Supervisor logs it; isolate and continue accepting. A supervisor panic is also logged/reaped as per-connection, never server-fatal. |
| Transient accept error | Log, retain all connections, perform bounded exponential backoff, then retry. |
| Fatal accept/control error | Drop the listener, call `JoinSet::detach_all` **without cancelling connection tokens**, and return the fatal error. This prevents `JoinSet` drop from aborting active connections. |
| Cancellation during accept | Stop accepting, drop the listener, signal connection shutdown according to the finally authorized state model, and begin the five-second deadline. |
| Cancellation during backoff | Drop the listener and begin that same deadline, but preserve the remaining backoff wait before server completion. Go's pre-drain listener wait can outlive the context, and quiescence checked afterward can still return success. |
| Fatal accept and cancellation simultaneously ready | Use unbiased selection and permit either Go-observable branch: fatal error with detached connections, or cancellation followed by Shutdown semantics. Do not impose an undocumented fixed precedence. |
| Graceful completion inside five seconds | Drain `JoinSet::join_next` until empty, logging all per-connection outcomes, then return success subject to the new/idle-state and listener-close gates. |
| Five-second drain expiry | Call `JoinSet::detach_all` before drop and return a typed shutdown deadline error. Already-signalled connections and started blocking work continue in the background, matching Go `Shutdown`. |

Do not use `JoinSet::shutdown` on the graceful path: it aborts tasks. Do not
drop a populated set: its `Drop` implementation aborts tasks. Do not make any
connection `JoinError` server-fatal.

## Explicit behavior-parity authority gates

### 1. Whole-request and keep-alive deadlines — unresolved

Go's five-second `ReadTimeout` is observably stronger and differently phased
than Hyper's header timer:

- one absolute budget covers the first byte of a request head through any body
  reads/drain;
- an idle keep-alive receives up to five seconds before its next request;
- after the first four bytes of that request arrive, it receives a fresh
  five-second header-plus-body budget.

Hyper `header_read_timeout(5s)` covers only the head and begins while polling
the next head, so idle time consumes the header budget and body reads are
unbounded by it. The metrics handler ignores request bodies, but Go drains a
small ignored body under the same deadline before connection reuse/response
completion; the focused probe made that observable.

No faithful deadline I/O layer using only the selected public APIs was proved.
Before this decision can be approved, one of these must be recorded:

1. a separately reviewed implementation/probe that reproduces all three phases,
   including ignored bodies and keep-alive reset; or
2. a human parity exception that explicitly accepts header-only timing, earlier
   keep-alive closure, and unbounded ignored-body read/drain differences.

A slow-header test alone cannot close this gate.

### 2. Never-active connection shutdown — unresolved

Go distinguishes a newly accepted connection from an idle keep-alive
connection during `Shutdown`. It closes an idle connection promptly, but a
connection that has not completed its first request remains `StateNew` until it
ages past five seconds or its five-second read deadline fires. The scaled Go
probe kept such a connection open through a 150 ms shutdown deadline and closed
it at about the 300 ms read boundary.

Hyper's public HTTP/1 graceful API exposes no new/active/idle state. The exact
Rust probe called `graceful_shutdown()` on a polled, never-active connection and
observed immediate EOF. Blanket per-connection signalling therefore fails
shutdown parity independently of the normal header/body/keep-alive deadline
gate.

Before approval, either prove and separately review a state-aware public-API
model that closes idle connections, drains active handlers, and preserves the
first-request window for new connections, or obtain a human parity exception
accepting immediate closure of never-active connections. Tests must distinguish
all three states; an in-flight-handler-only drain test cannot close this gate.

### 3. Listener-close errors — unresolved

Go `Shutdown` returns the first listener-close error after a successful drain;
Tokio's standard `TcpListener` closes by RAII and exposes no fallible close
result. Go's precise precedence is subtler than the public summary: it records
the close error, waits for listener-serving goroutines without consulting the
context, then checks quiescence before selecting the context. Thus an already
expired context may still yield the close result (including success) when the
server is quiescent, while a non-quiescent drain returns the context error. A
fatal `Serve` return ignores Go's deferred listener-close error, so only the
cancellation/Shutdown path is affected.

Before approval, either prove a safe owned-listener abstraction that reports a
real close failure without double-close/file-descriptor reuse hazards, or record
a human parity exception allowing Tokio's unreportable RAII close. The
implementation must not silently claim exact shutdown error parity.

Under [`PORTING.md`](../../PORTING.md), these are authority gates rather than
researcher-discretion exceptions: observable parity is required, and conflicts
that cannot be adapted safely require human escalation. This adversarial review
therefore cannot accept any of the three exceptions on the project's behalf.

## Known limitations and cross-record conflict

- The approved [`prometheus-metrics.md`](prometheus-metrics.md) currently says
  the HTTP layer must negotiate zstd. Go 1.26.1 source plus a whole-tree import
  scan show that the frozen program does not activate `promhttp/zstd`; actual
  behavior is identity/gzip. This record therefore selects flate2 for gzip and
  no zstd crate. The controller must reconcile that conflicting obligation
  before creating a ready package packet; this researcher does not own the
  other record.
- Hyper and Tokio contain maintained internal unsafe code. The selected package
  integration uses safe public APIs; this is not a claim that the transitive
  graph is unsafe-free.
- There is deliberately no TLS, authentication, HTTP/2, wildcard bind, signal
  handling, or framework router. Those absences match the oracle and must not be
  treated as missing dependency features.
- Compression and Prometheus collection are CPU/synchronous work. `spawn_blocking`
  prevents scheduler starvation but does not impose a scrape concurrency limit;
  preserve Go's concurrent handler behavior and do not add an unobserved limit.
- The server crate must run inside the caller-owned Tokio runtime; it must not
  create or drop a private runtime. A started `spawn_blocking` task is
  unabortable, and dropping a runtime can wait for such work rather than merely
  detaching it.
- Tokio 1.53.1 is current but is not one of Tokio's listed LTS minor lines at
  review time. The exact pin passes maintenance and security gates; re-review an
  upgrade instead of floating it.

## Verification probes and required package checks

Research-time commands passed on Rust/Cargo `1.96.0`:

```sh
cargo test --offline --all-targets --manifest-path /tmp/http-runtime-rust-probe/Cargo.toml
cargo clippy --offline --all-targets --manifest-path /tmp/http-runtime-rust-probe/Cargo.toml -- -D warnings
cargo tree --offline -e features --manifest-path /tmp/http-runtime-rust-probe/Cargo.toml
cargo check --offline --target x86_64-unknown-linux-gnu --all-targets --manifest-path /tmp/http-runtime-rust-probe/Cargo.toml
cargo check --offline --target x86_64-apple-darwin --all-targets --manifest-path /tmp/http-runtime-rust-probe/Cargo.toml
cargo check --offline --target x86_64-pc-windows-gnu --all-targets --manifest-path /tmp/http-runtime-rust-probe/Cargo.toml
cargo audit --no-fetch --file /tmp/http-runtime-rust-probe/Cargo.lock
```

The exact API probe compiled `http1::Builder`, `TokioTimer`, featureless
`CancellationToken`, and pure-Rust gzip. Its current-thread test proved a 250 ms
gather/encode/gzip closure did not prevent a concurrent request from completing
inside 75 ms; its shutdown test proved Hyper graceful close immediately ends a
never-active connection. Linux IPv4/IPv6 bind-before-cancel tests and Linux,
Intel-macOS, and Windows-GNU all-target checks passed. The cross-target lock
contained 35 packages including the probe (34 third-party); the native normal
graph resolves 31 third-party packages on Linux/macOS and 32 on Windows. Cargo
metadata found only permissive licenses. The cached RustSec database contained
1,211 advisories at commit `d0861df`, with no vulnerability or informational
finding.

Go characterization passed with:

```sh
env GOCACHE=/tmp/http-runtime-go126-cache \
  GOMODCACHE=/tmp/http-runtime-go126-modcache \
  GOPROXY=off GOSUMDB=off \
  /opt/go1.26.1/bin/go test -mod=mod -v ./...
```

The expanded Go suite also probed compression fallback, CONNECT routing,
never-active shutdown, and cancellation during a 320 ms accept backoff. After
all three authority gates are resolved, the package implementation still
must add deterministic tests for:

- concrete IPv4 and IPv6 bind/serve and fixed port `51090`;
- occupied-port propagation with an already-cancelled token;
- the full ServeMux matrix above, including CONNECT and `OPTIONS *`;
- the exact gzip/identity quality matrix, zstd-only fallback, wildcard fallback,
  and the oracle's identity fallback when every recognized encoding has `q=0`;
- 5 ms-through-1 s transient accept retry, reset after success, fatal accept
  propagation, and cancellation during backoff;
- malformed/disconnected connection errors plus handler and blocking-task
  panics remaining logged/nonfatal;
- current-thread concurrent request and concurrent scrape progress;
- cancellation refusing new connections, distinct new/active/idle treatment,
  clean in-flight drain, typed five-second expiry, post-timeout task
  continuation, simultaneous fatal/cancel selection, and every JoinSet path in
  the ownership table;
- the finally authorized deadline behavior and listener-close behavior on
  Linux, macOS, and Windows CI.

## Review

| Item | Result |
| --- | --- |
| Critical capability | Networking/runtime/compression; a second fresh adversarial dependency review is mandatory |
| Prior reviewer | **REJECTED** proposal `1ab24e4`; all six stated findings are addressed in this revision |
| Fresh reviewer | Completed independently on `f927d1bf224142754bc2f818b88fdb46d7d70686` with primary sources and executable Go/Rust probes |
| Challenge coverage | All three parity gates; accept classification/backoff; connection panic isolation; current-thread blocking; exact feature/lock graph; method/path/query/cleaning/escaping/307 routing; every task/exit path; cancellation/error precedence; gzip/identity/zstd fallback; IPv4/IPv6; Linux/macOS/Windows targets; Rust 1.96; licenses; RustSec |
| Final reviewer result | **REJECTED for approval**. The family selection survives, but the proposed shutdown model was not clean: cancellation incorrectly interrupted accept backoff, and blanket Hyper graceful signalling failed never-active connection parity. The record now corrects the former and makes the latter a third explicit authority gate. |
| Affected package packets | `ployz-internal-machine-metrics`; no prose package packet exists at this integration base |

The controller must not mark this dependency `approved` until all three
authority gates and the Prometheus compression conflict are resolved in records
owned by the authorized roles. A future clean technical re-review alone cannot
grant the required human parity exceptions.
