# Dependency decision: `http-server-runtime`

| Field | Value |
| --- | --- |
| Status | `blocked` — Hyper-private pipelined bytes and response-write timing break Go's idle/fresh-request deadline transitions, Hyper's request-target/aggregate-head limits differ, and Tokio cannot report the listener-close result |
| Capability | Production plain-HTTP server/runtime and gzip response compression for `ployz-internal-machine-metrics` |
| Selected dependency | Conditional stack: Hyper `1.11.0`, hyper-util `0.1.20`, http-body-util `0.1.4`, Tokio `1.53.1`, tokio-util `0.7.19`, and flate2 `1.1.9` |
| License | `MIT` for the Hyper/Tokio crates; `MIT OR Apache-2.0` for flate2 |
| Research date | `2026-08-11` UTC |
| Request | Delegated capability request; no request file exists at integration base |
| Integration base | `4c2cefbcf61f51d3a4faa8dd68d8a7f4f2d95717` |
| Prior proposal | `1ab24e45c0d97f16d1f51a2f8db1281db089fa2e`, inspected with `git show`; its mandatory adversarial review rejected the decision |
| Previous adversarial review base | `f927d1bf224142754bc2f818b88fdb46d7d70686` |
| Affected package | `ployz-internal-machine-metrics` (`upstream/uncloud/internal/machine/metrics`) |

This revision does **not** approve dependencies for implementation. Executable
Go-versus-Rust probes show that a small state-aware transport/service adapter
can reproduce the ordinary whole-request deadline and never-active shutdown
cases, but cannot see bytes Hyper privately read past a request boundary. In
the differential matrix, first reads that include one, two, or three bytes of
the next request all contribute to Go's four-byte idle peek and earn a fresh
request budget, while Hyper hides them from the transport adapter and it closes
the connection. A second differential probe held a 32 MiB response write
backpressured beyond the scaled timeout: Go began the idle period only after
the write completed, while the adapter began it as soon as the service returned
the response and closed the subsequent request early. The probes also exposed
deterministic protocol-limit mismatches: Go accepts a valid
70 KiB request target within its one-megabyte header budget while Hyper rejects
it at a private 65,534-byte cap, and configured Hyper accepts a head beyond
Go's aggregate limit. A standard Tokio listener also cannot report Go's
fallible close result. The previously reported Prometheus compression-record
conflict is already resolved at this integration base.

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
- Go's [`DefaultMaxHeaderBytes`](https://github.com/golang/go/blob/go1.26.1/src/net/http/server.go#L892-L905)
  is one MiB with 4096 bytes of read-buffer slop, and no independent
  request-target or 100-field cap applies. Hyper's
  [parser source](https://docs.rs/hyper/1.11.0/src/hyper/proto/h1/role.rs.html#34)
  fixes `MAX_URI_LEN` at `u16::MAX - 1`, while its configurable defaults are
  100 fields and an approximately 408 KiB read buffer. `max_headers` can remove
  the field-count default, but `max_buf_size` is not an exact aggregate-head
  bound and neither option changes the URI cap.
- Go's net/http response writer buffers up to 2048 bytes before choosing
  framing. Promhttp therefore uses an exact content length when the final
  identity or gzip payload is at most 2048 bytes and chunked framing above that
  threshold. Hyper can reproduce this without hand-writing HTTP by selecting an
  exact-size or unknown-size one-frame body after encoding/compression.
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
| Privately buffered partial pipeline | Separate cases placed one, two, or three bytes of `GET ` after a complete request in the first socket write. After 150 ms the rest of those first four bytes arrived, and the request remainder arrived after another 150 ms. Go counted the private prefix toward its idle peek and served all three second requests. Hyper hid every prefix inside its parser; the transport adapter served none of them. |
| Temporary accepts | Injected `ECONNRESET`/`ECONNABORTED` attempts occurred after `5, 10, 20, 40, 80, 160, 320, 640, 1000` ms; the next fatal sentinel propagated. |
| Cancellation during accept backoff | During the 320 ms sleep, `Shutdown` with a 50 ms context returned success only after about 320 ms. `listenerGroup.Wait` is before the context-aware drain loop, so backoff is not interrupted and the context is not a strict wall-clock bound for this phase. |
| Never-active connection at shutdown | With a 300 ms `ReadTimeout`, a 150 ms shutdown context returned `DeadlineExceeded`; the silent `StateNew` connection remained open until about the original 300 ms read boundary. Hyper's direct graceful call closed the corresponding initial connection immediately. |
| Bind families | Concrete IPv4 and IPv6 addresses bound without wildcard substitution. |
| Request limits | 101 small fields and a 900 KiB request head reached the Go handler; a head 8192 bytes above one MiB returned `431`; a valid 70 KiB target reached the handler. Configured Hyper passed the first two, incorrectly accepted the Go-invalid oversized head, and returned `414` for the 70 KiB target. |
| Handler plus incomplete ignored body | A handler delayed 120 ms, prepared `prepared`, and returned without reading a five-byte body of which the client sent one byte. Go retained that response while its original absolute read deadline expired and emitted it at about 201 ms. The Rust adapter reproduced this only after changing its deadline state to a true disarmed `None`, not the earlier 24-hour surrogate. |
| Write and backpressure timing | A handler that waited 350 ms still returned its response with a 200 ms `ReadTimeout`. A 32 MiB response held backpressured for 350 ms was then read fully; after 150 ms idle the client sent four request bytes, waited another 150 ms, and Go served the request. Thus the deadline is truly disarmed through handler/write work and the fresh idle period begins after response completion. The current adapter instead arms idle when the service returns the response and fails this case. |
| Ignored-body drain | For known `Content-Length`, 262,143 bytes were drained and reused while exactly 262,144 and 262,145 forced close. For chunked bodies with a declared and supplied trailer, 262,143 **and exactly 262,144** decoded bytes were drained and reused; 262,145 forced close. Exact-cap chunked EOF is observably different from a known length at the cap. The Rust `Limited` probe reproduced all six outcomes. |
| Listener close precedence | With no active connection, an injected listener-close error won even when the shutdown context was already expired. |

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

On a real socket, the current identity exposition exceeded 2048 bytes and used
`Transfer-Encoding: chunked` with no `Content-Encoding`. Repeated gzip probes
straddled the threshold: a 2042-byte gzip payload used an exact content length,
while a 2066-byte payload was chunked. Both carried
`text/plain; version=0.0.4; charset=utf-8; escaping=underscores`, a `Date`
header, and payloads that decoded to the same exposition. The Rust wire probe
reproduced that framing and header matrix using public Hyper body APIs.

## Hard gates

| Gate | Requirement | Evidence and disposition | Result |
| --- | --- | --- | --- |
| Behavior | Exact bind/accept/route/limits/concurrency/cancellation/drain/error behavior | The state-aware adapter passes ordinary whole-request, slow-handler/incomplete-body, known/chunked drain, and shutdown cases. It cannot observe next-request bytes in Hyper's private buffer and arms idle before a backpressured response finishes, causing two idle/fresh-budget mismatches. Hyper also rejects a Go-valid 70 KiB target and accepts a Go-invalid over-limit head; Tokio listener drop cannot report close failure. | `fail` on five exact scenario classes |
| License and security | Permissive licenses; no unnecessary TLS/auth/C runtime; current vulnerability scan | Direct manifests declare MIT or MIT/Apache-2.0; every resolved transitive license is permissive. Only HTTP/1 and flate2's exact pure-Rust `miniz_oxide` backend are enabled. A 1,211-advisory RustSec scan of the exact 35-package cross-target probe lock found no vulnerabilities or informational warnings. Re-run against the integrated lock. | `pass` |
| Platforms and targets | Linux, macOS, and Windows; concrete IPv4/IPv6; no wildcard substitution | Tokio officially supports all three platforms; `SocketAddr` and `TcpListener::bind` cover both families. Rust 1.96 target checks passed for Linux, Intel macOS, and Windows GNU. Native bind/runtime probes exercised IPv4/IPv6 only on Linux, so macOS/Windows runtime claims remain package-acceptance obligations rather than research-time results. | `pass` for the dependency; native non-Linux verification still required |
| Maintenance and Rust version | Popular, maintained current versions compatible with Rust 1.96 | Current releases were revalidated on 2026-08-11. Declared rust-version values are Hyper 1.63, hyper-util 1.64, http-body-util 1.61, Tokio/tokio-util 1.71, and flate2 1.67; the maximum is below the project's Rust 1.96. | `pass` |
| Architectural constraints | Natural dependency APIs, bounded private accept loop, no Go-shaped compatibility server | Hyper remains the HTTP parser/encoder. The extra adapter owns only transport deadline state and service lifecycle notifications; it neither parses HTTP nor imitates `net/http.Server`. | `pass` |

## Candidate comparison

Exact latest versions were revalidated from the projects' published docs.rs
records on `2026-08-11`. Adoption figures from the official crates.io API are
point-in-time comparison evidence.

| Candidate | Disposition | Reason |
| --- | --- | --- |
| **Hyper `1.11.0` + Tokio `1.53.1`** | **Selected family, blocked** | The most widely adopted low-level family and the only credible option that leaves bind, accept classification, per-connection supervision, graceful signaling, and fatal exit visible. The adapter passes the ordinary timeout/state cases but cannot observe Hyper-private bytes across a pipelined request boundary. Its request-target/aggregate-head limits and infallible listener drop also fail parity. |
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
tokio = { version = "=1.53.1", default-features = false, features = ["io-util", "macros", "net", "rt", "sync", "time"] }
tokio-util = { version = "=0.7.19", default-features = false }
```

Do not enable `full`, HTTP/2, TLS, authentication, signals, a C compression
backend, hyper-util `server-graceful`, or tokio-util `rt`. `CancellationToken`
needs no tokio-util feature. Tokio `io-util` is required by the safe
`AsyncRead` deadline adapter, and `sync` is explicit because the adapter uses
Tokio lifecycle notification rather than depending on feature unification.
hyper-util needs only `tokio` for `TokioIo`;
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
2. Configure Hyper HTTP/1 only with `TokioIo`; do not enable h2c or Hyper's
   header timer. Set `max_headers(262_144)`, the maximum count implied by Go's
   byte budget and a four-byte minimum field line, and
   `max_buf_size(1_052_672)`, the closest public aggregate-head setting.
   The probe proves it is not exact: Hyper still accepted a head that Go
   rejected. The separate 65,534-byte request-target cap is private and cannot
   be configured.
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
5. Pin the inner Hyper connection and poll lifecycle notifications beside it.
   On cancellation, leave a never-active connection alone until its first-read
   deadline; if it becomes active later, or is already active/idle, call
   `graceful_shutdown()`. Continue awaiting the connection. Calling it
   indiscriminately is rejected because it closes a never-active connection
   immediately. If the
   outer server exits for a fatal listener/control error, do **not** cancel any
   connection token; existing connections continue as they do after Go `Serve`
   returns a fatal accept error.

The nested inner task is deliberate: after a shutdown timeout or fatal accept,
the supervisor can be detached yet still await and log an inner connection
panic. A bare detached Hyper task would silently discard its `JoinError`.

### Whole-request deadline adapter

- Wrap the Tokio stream in a safe `AsyncRead`/`AsyncWrite` adapter and then in
  `TokioIo`; Hyper continues to own all HTTP parsing and encoding. Start the
  initial absolute read deadline when the connection begins reading.
- When the service receives a request, mark it active and construct the handler
  response before the post-handler drain. Preserve that buffered response even
  when an incomplete body reaches the read deadline. For a known length, drain
  below Go's 256 KiB tolerance and force close at exactly 256 KiB or above. For
  unknown/chunked bodies, use `http_body_util::Limited` to detect a decoded byte
  beyond the tolerance: exact-cap EOF plus trailers is reusable, while 256 KiB
  plus one byte closes. Once body handling completes, represent the read
  deadline as genuinely absent; the corrected probe uses `Option<Instant>` and
  does not substitute a 24-hour timer. Handler and write work remain unbounded.
- Go arms idle only after `finishRequest` completes response transmission, then
  grants a fresh whole-request deadline once its four-byte peek completes. The
  executable probe passed slow-head, delayed-head plus incomplete-body,
  slow-handler plus incomplete-body, idle expiry, idle-plus-fresh-request, and
  all six known/chunked drain boundaries with a scaled 200 ms budget. It also
  proves two unresolved adapter failures: Hyper hides one to three prefetched
  bytes, and a service-return notification occurs before a backpressured write
  completes. No implementation may treat "response object ready" as the idle
  transition or claim the public adapter described here is sufficient.
- Keep lifecycle state in a small shared state object used only by the transport,
  service, and pinned connection driver. Focused one-connection probes
  distinguished never-active, active, and idle cancellation, drained one active
  request, closed one idle connection, retained one new connection through its
  read deadline, and observed those connection tasks reach zero. They do not
  prove the accept-loop supervisors, fatal paths, panic logging, or full
  `JoinSet` ownership table; those remain package acceptance obligations.

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
  `JoinError` as a request/connection failure, and return an owned in-memory
  buffer. For either identity or gzip, expose payloads of at most 2048 bytes
  through an exact-size body and larger payloads through an unknown-size
  one-frame body. This reproduces Go's content-length/chunked threshold.
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
| Cancellation during accept | Stop accepting, drop the listener, apply the probed state-aware connection shutdown model, and begin the five-second deadline. |
| Cancellation during backoff | Drop the listener and begin that same deadline, but preserve the remaining backoff wait before server completion. Go's pre-drain listener wait can outlive the context, and quiescence checked afterward can still return success. |
| Fatal accept and cancellation simultaneously ready | Use unbiased selection and permit either Go-observable branch: fatal error with detached connections, or cancellation followed by Shutdown semantics. Do not impose an undocumented fixed precedence. |
| Graceful completion inside five seconds | Drain `JoinSet::join_next` until empty, logging all per-connection outcomes, then return success subject to the listener-close gate. This path must leave zero owned tasks. |
| Five-second drain expiry | Call `JoinSet::detach_all` before drop and return a typed shutdown deadline error. Already-signalled connections and started blocking work continue after return, matching Go `Shutdown`; tests must prove they terminate eventually and are not lost ownership leaks. |

Do not use `JoinSet::shutdown` on the graceful path: it aborts tasks. Do not
drop a populated set: its `Drop` implementation aborts tasks. Do not make any
connection `JoinError` server-fatal.

## Remaining behavior-parity authority gates

### 1. Whole-request deadline across Hyper-private pipelining — unresolved

Go's keep-alive loop performs a four-byte peek before clearing its idle deadline
and starting a fresh whole-request deadline. The exact differential probe put a
complete first request and `GET`—three bytes of the next request—in one socket
write. After the first response, it waited 150 ms, wrote the fourth byte, waited
another 150 ms, and completed the request. With a scaled 200 ms timeout, Go
served the second request: its buffered three bytes contributed to the peek, so
the fourth byte started a fresh 200 ms budget.

Hyper may read those three bytes into its private parser buffer while completing
the first request. The safe transport adapter cannot observe that request
boundary or recover the buffered-byte count. It therefore saw only the fourth
byte after entering idle state, retained the original idle deadline, and closed
before the remainder arrived. The Rust characterization test asserts this
failure rather than treating its passing test process as behavior parity.

No public Hyper API exposes the unread byte count at this boundary. Parsing HTTP
inside the transport adapter would duplicate Hyper's parser; forcing every head
read to one byte would impose a pathological I/O policy merely to prevent
over-read. Neither is an idiomatic adapter boundary. Approval requires a public
parser-state hook, a different parser/server that passes every lifecycle gate,
or an explicit human parity exception for this deadline divergence.

### 2. Response completion before idle timing — unresolved

Go clears the request read deadline before handler/write work and does not arm
the keep-alive idle deadline until `finishRequest` has completed response
transmission. The differential probe returned a 32 MiB response and withheld
client reads for 350 ms, longer than the scaled 200 ms timeout. Go transmitted
the full response, then allowed 150 ms idle plus another 150 ms to finish a
request after its first four bytes; it served that request successfully.

The public adapter used the service future's return as "response ready." Hyper
then polled transport reads while its large response was still backpressured,
which armed and exhausted the idle timer before response transmission
completed. The next request was not served. A service-layer notification is
therefore too early, and public `AsyncWrite` calls do not identify the semantic
end of a particular Hyper response. Approval requires a public connection or
encoder completion hook, a different server/parser that passes the lifecycle
matrix, or an explicit human parity exception. The record does not assume that
`poll_flush` is an exact response-boundary signal without a separate proof.

### 3. Request limits — unresolved and technically blocked

Go applies one shared one-MiB-plus-slop limit to the request line and headers.
The socket probe sent a valid 70 KiB target and reached the Go handler. Hyper
returned `414` before service invocation because `MAX_URI_LEN` is privately
fixed at `u16::MAX - 1`. Conversely, with `max_buf_size(1_052_672)`, Hyper
accepted a header value 8192 bytes above one MiB that Go rejected with `431`;
its adaptive read buffer can expose capacity beyond the requested next read, and
there is no public HTTP/1 aggregate-header-size setting. `max_headers` fixes only
the independent 100-field default and preallocates proportional scratch space.

These are deterministic selected-stack failures. Approval requires either a
Hyper release/public configuration that enforces the oracle range, a
different server/parser that passes all lifecycle gates, or a human parity
exception explicitly accepting the narrower request-target limit. Reimplementing
HTTP parsing in the package is not an idiomatic adapter boundary.

### 4. Listener-close errors — unresolved

Go `Shutdown` returns the first listener-close error after a successful drain;
Tokio's standard `TcpListener` closes by RAII and exposes no fallible close
result. The Go probe also confirmed the precise precedence: with a quiescent
server, the close error wins even when the context is already expired. A fatal
`Serve` return still ignores its deferred listener-close error.

The frozen package creates its own concrete TCP listener, so test injection is
not a production solution. Approval requires either a separately approved safe
owned-listener abstraction that reports the real platform close result without
double-close/file-descriptor reuse hazards, or a human parity exception allowing
Tokio's unreportable RAII close. The implementation must not claim exact
shutdown error parity.

Under [`PORTING.md`](../../PORTING.md), the researcher cannot grant these
exceptions. The never-active shutdown question, true deadline disarming, the
slow-handler/incomplete-body ordering, and ordinary non-prefetched deadline
cases are technically closed by the public-API probe. The private-prefetch and
response-completion idle transitions remain blocked.

## Known limitations

- The approved [`prometheus-metrics.md`](prometheus-metrics.md) now agrees with
  the frozen oracle: identity/gzip negotiation with identity fallback and no
  active zstd encoder. No cross-record compression conflict remains.
- Hyper's `max_headers` can remove its 100-field default, but a very large value
  allocates correspondingly large parser scratch space. The specified
  `262_144` value must be stress-tested under concurrent scrapes; it repairs
  neither aggregate head size nor the URI cap.
- Hyper's private read buffer can contain part of the next request before the
  service/transport adapter learns that the current request is complete. The
  adapter instructions above do not authorize parser duplication or one-byte
  request-head reads as workarounds.
- Returning a Hyper `Response` from the service is not evidence that its bytes
  have reached the socket. Arming idle at that point fails the 32 MiB
  backpressure probe; `AsyncWrite::poll_flush` is not claimed as the semantic
  response boundary without executable proof.
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
cargo audit --file /tmp/http-runtime-rust-probe/Cargo.lock
```

The exact API probe compiled the safe Tokio `AsyncRead` adapter, `TokioIo`,
HTTP/1 `Builder`, featureless `CancellationToken`, and pure-Rust gzip. Nine
Rust tests passed. Seven exercise matching or required behavior: ordinary
shared head/body and pipelined deadlines; preservation of the
already-constructed response after a 120 ms handler and incomplete-body
timeout with the read deadline represented as `None`; all 262,143/262,144/
262,145-byte known-length and chunked-plus-trailer boundaries; idle expiry and
a fresh request budget; no handler/write timeout;
configured header-count/size behavior and the 70 KiB URI result;
new/active/idle cancellation; current-thread progress during 250 ms
`spawn_blocking`; isolated `JoinSet` drain/detach primitives; gzip/identity
negotiation including ties, repeated fields, and malformed quality values; and
the 2048/2049-byte socket framing boundary. Two negative characterizations pass
by proving divergences: Hyper's private buffer causes the adapter to close each
one-, two-, and three-byte prefetched request that Go serves, and service-return
idle arming closes the post-backpressure request that Go serves. The JoinSet
test is deliberately not evidence for the unimplemented accept-loop ownership
table.
Linux native tests plus Linux, Intel-macOS, and Windows-GNU all-target checks
passed; macOS and Windows runtime behavior is not claimed without native CI.
The cross-target lock contained 35 packages including the probe. Cargo metadata
found only permissive licenses. A fresh RustSec fetch contained 1,211 advisories at commit
`d0861df1eab469d3c58d6b836ce48b5766e5f217`, with no vulnerability or
informational finding.

Go characterization passed with:

```sh
env GOCACHE=/tmp/http-runtime-go126-cache \
  GOMODCACHE=/tmp/http-runtime-go126-modcache \
  GOPROXY=off GOSUMDB=off \
  /opt/go1.26.1/bin/go test -mod=mod -v ./...
```

The expanded Go suite also probed compression fallback and raw framing, CONNECT
routing, request limits, the distinct known-length and chunked-plus-trailer
drain boundaries, slow handler plus incomplete body, backpressured response
completion, lack of a write timeout, listener-close precedence, never-active
shutdown, one/two/three-byte privately buffered partial pipelining, and
cancellation during a 320 ms accept backoff. If the four remaining authority
gates are resolved, the package implementation still must
add deterministic tests for:

- concrete IPv4 and IPv6 bind/serve and fixed port `51090`;
- occupied-port propagation with an already-cancelled token;
- the full ServeMux matrix above, including CONNECT and `OPTIONS *`;
- 101 fields, 900 KiB/over-one-MiB request heads, and the finally authorized
  request-target behavior;
- the exact gzip/identity quality matrix, zstd-only fallback, wildcard fallback,
  ties, repeated fields, malformed quality values, the oracle's identity
  fallback when every recognized encoding has `q=0`, and
  the 2048/2049-byte content-length versus chunked framing boundary for both
  identity and gzip;
- 5 ms-through-1 s transient accept retry, reset after success, fatal accept
  propagation, and cancellation during backoff;
- malformed/disconnected connection errors plus handler and blocking-task
  panics remaining logged/nonfatal;
- current-thread concurrent request and concurrent scrape progress;
- cancellation refusing new connections, distinct new/active/idle treatment,
  clean in-flight drain, typed five-second expiry, post-timeout task
  continuation, simultaneous fatal/cancel selection, and every JoinSet path in
  the ownership table;
- the probed deadline behavior and finally authorized listener-close behavior
  on Linux, macOS, and Windows CI.

## Review

| Item | Result |
| --- | --- |
| Critical capability | Networking/runtime/compression; a second fresh adversarial dependency review is mandatory |
| Prior reviewers | **REJECTED** proposal `1ab24e4`; the next fresh review rejected candidate `7dc92fa` because slow-handler/incomplete-body ordering, 1/2/3-byte prefetched pipelining, chunked drain boundaries, response-write/idle timing, and bounded task-reclamation evidence were incomplete. This revision addresses those record-evidence findings and preserves every demonstrated dependency blocker. |
| Fresh reviewer | Read-only adversarial review of exact corrected commit `87a2c7cb2d9fc47bca07a78eff1e98fb003d2a9c`; **ACCEPT committing this blocked decision**, with no actionable finding. The reviewer independently reran the full Go suite, all nine Rust tests, warnings-denied Clippy, three target checks, and RustSec. |
| Challenge coverage | Hyper-private one/two/three-byte partial-pipeline buffering; backpressured response completion versus idle arming; known-length and chunked-plus-trailer drain boundaries; request limits; truly disarmed whole-request/idle timeouts; new/active/idle shutdown; accept/cancellation/error precedence; bounded task-reclamation claims; gzip/identity negotiation and 2048-byte framing boundary; platforms; versions/features; Rust 1.96; licenses; RustSec |
| Final reviewer result | **ACCEPT blocked record; REJECT dependency approval.** Five exact scenario classes fail: private partial-pipeline bytes, response-completion/idle timing, 70 KiB request-target rejection, oversized aggregate-head acceptance, and unreportable listener-close result/precedence. No parity exception was granted. |
| Affected package packets | `ployz-internal-machine-metrics`; no prose package packet exists at this integration base |

The controller must not mark this dependency `approved` until the private-byte
and response-completion deadline transitions, both request-limit divergences,
and listener-close authority gate are resolved. A clean technical re-review
alone cannot grant the required human parity exceptions.
