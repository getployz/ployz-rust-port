# Dependency decision: `async-stream-network-runtime`

| Field | Value |
| --- | --- |
| Status | `human-decision-required` — Tokio is the best conditional runtime, but exact hostname/no-orphan behavior and the accepted-connection bound conflict with the oracle and lack scope authority |
| Capability | Bounded, cancellation-safe TCP/Unix stream acceptance, deadline-bounded TCP dialing, supervised bidirectional copying, and write-half shutdown for `internal/proxy` |
| Selected dependency | No approved selection; conditional candidate is Tokio `1.53.1` and tokio-util `0.7.19` |
| License | `MIT` (both direct crates) |
| Research date | `2026-08-11` UTC |
| Request | Delegated capability request; no request file exists at the assigned integration base |
| Integration base | `689686e18be585b3d4233f8fa6df36486315f440` |
| Blocked package evidence | `/tmp/ployz-proxy-writer` commit `1040794b0e5696afd4ee1145b64902033217a0fa` |
| Affected package | `ployz-internal-proxy` (`upstream/uncloud/internal/proxy`) |

The conditional stack is the popular, idiomatic candidate. It replaces
the rejected implementation's blocking listener clones and per-connection OS
threads with cancellation-safe readiness futures and supervised runtime tasks.
It is **not approved**: the mandatory fresh adversarial review found unresolved
behavior authority and verification gaps. The controller must not add it to the
dependency registry as approved.

## Oracle and blocked-implementation evidence

- [`proxy.go`](../../upstream/uncloud/internal/proxy/proxy.go) accepts until its
  context is cancelled or the listener fails. Cancellation closes the listener,
  cancels in-flight handlers, waits for every accepted handler, and returns
  `nil`. A non-cancellation accept failure cancels all handlers, waits for them,
  and returns `accept local connection: %w`.
- Every accepted connection gets a ten-second child dial context. The dialer is
  called with network `tcp` and the unchanged remote-address text. Dial failure
  is reported as `connect remote address '<address>': %w` only while the parent
  context is live. A custom Go dialer is responsible for obeying the supplied
  context; `Run` waits for it through the handler wait group.
- Once connected, two copies run concurrently. Clean EOF in one direction
  calls `CloseWrite` when supported while the other direction continues. The
  first copy failure closes both streams; after both directions finish it is
  reported as `data copy: %w` only while the run context remains live.
- [`proxy_test.go`](../../upstream/uncloud/internal/proxy/proxy_test.go) requires
  a non-cancellation listener error to retain its source, individual connection
  failures not to stop acceptance, a custom dialer to receive the exact network
  and address, and wrapped closed-connection recognition.
- Direct callers in [`pkg/client/image.go`](../../upstream/uncloud/pkg/client/image.go)
  use both loopback TCP and filesystem Unix listeners, pass numeric-IP remote
  addresses through a custom tunnel dialer, remove a Unix socket during
  cleanup, suppress routine closed-connection errors, and consume other proxy
  errors. [`cmd/uc/proxy.go`](../../upstream/uncloud/cmd/uc/proxy.go) uses a TCP
  listener, a numeric-IP remote address, and a custom cluster dialer.
- The rejected Rust candidate's fresh reviews found six concrete problems:
  a cloned blocking Unix listener can remain in `accept` after its path is
  unlinked (P01); the custom dialer's ten-second deadline is data rather than an
  active cancellation source (P02); a cancelled default dial returns while its
  connect thread can later create a connection (P03); closed-error matching
  wrongly includes every `NotConnected` and Windows raw code `10058` (P04);
  every connection consumes unbounded handler/dial/copy OS threads (R01); and a
  synchronous custom dialer that ignores its context can prevent `Run` from
  terminating forever (R02).

## Primary-source evidence

- Tokio `1.53.1` documents both
  [`TcpListener::accept`](https://docs.rs/tokio/1.53.1/tokio/net/struct.TcpListener.html#method.accept)
  and Unix-only
  [`UnixListener::accept`](https://docs.rs/tokio/1.53.1/tokio/net/struct.UnixListener.html#method.accept)
  as cancellation-safe: if another `select!` branch wins, no connection was
  accepted. This removes the need to clone, close, or reconnect to a blocking
  listener and therefore still cancels after another caller unlinks the Unix
  socket path.
- [`TcpSocket::connect`](https://docs.rs/tokio/1.53.1/tokio/net/struct.TcpSocket.html#method.connect)
  consumes the socket and its source awaits readiness on a stream owned by that
  future. Racing this future against cancellation/deadline and dropping the
  losing branch drops the not-yet-connected socket; no detached connect task
  exists to create a connection later.
- Tokio's string-address [`ToSocketAddrs` source](https://docs.rs/tokio/1.53.1/src/tokio/net/addr.rs.html)
  is materially different: non-numeric strings use `spawn_blocking` for the OS
  resolver. Tokio documents that a started
  [`spawn_blocking`](https://docs.rs/tokio/1.53.1/tokio/task/fn.spawn_blocking.html)
  task cannot be aborted. The implementation must therefore parse the default
  dial target to `SocketAddr` before `TcpSocket::connect`; it must not pass a
  hostname string to `TcpStream::connect` or `lookup_host`. All known direct
  callers already supply numeric IP addresses. Hostname behavior is discussed
  under known limitations.
- [`copy_bidirectional`](https://docs.rs/tokio/1.53.1/tokio/io/fn.copy_bidirectional.html)
  polls both directions concurrently, uses two fixed 8 KiB buffers, calls
  `AsyncWrite::shutdown` on the opposing writer after clean EOF, continues the
  other direction, and returns the first I/O error immediately. Its source
  contains both transfer state machines in the caller's future and spawns no
  hidden tasks. Unlike Go, however, it propagates `AsyncWrite::shutdown`
  failures. It is therefore not an exact drop-in: a small `AsyncWrite` adapter
  would have to poll the real shutdown to completion but deliberately convert
  its result to `Ok(())`, preserving Go's ignored `CloseWrite` result.
- [`Semaphore`](https://docs.rs/tokio/1.53.1/tokio/sync/struct.Semaphore.html)
  supplies the explicit accepted-handler bound.
  [`JoinSet`](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinSet.html)
  owns spawned tasks, offers cancellation-safe `join_next`, aborts tasks on
  drop, and documents that `shutdown` aborts and then waits until the set is
  empty. The proxy should normally cancel and drain cooperatively; it must never
  call `detach_all`.
- [`CancellationToken`](https://docs.rs/tokio-util/0.7.19/tokio_util/sync/struct.CancellationToken.html)
  wakes all waiters, supports child tokens, and documents `cancelled()` as
  cancellation-safe. Its
  [published manifest](https://docs.rs/crate/tokio-util/0.7.19/source/Cargo.toml)
  shows that the type is available without an optional feature; `rt` is for
  separate task utilities and is unnecessary here.
- Tokio's published
  [platform-policy source](https://docs.rs/crate/tokio/1.53.1/source/src/lib.rs)
  lists Linux, Windows, and macOS among supported platforms. Unix
  sockets are compiled only on Unix; Windows uses TCP for the oracle's shipped
  proxy path and must not expose a fake filesystem-Unix implementation.
- The published Tokio and tokio-util manifests declare MSRV `1.71` and MIT.
  Their official crates.io records on 2026-08-11 reported current releases
  `1.53.1` (2026-07-20) and `0.7.19` (2026-07-21), respectively. Tokio reported
  868.4 million total downloads and 67,828 reverse-dependency API rows;
  tokio-util reported 703.9 million and 6,612 rows. Both are below this
  workspace's Rust `1.96` and overwhelmingly lead the credible alternatives.

## Hard gates

| Gate | Requirement | Evidence and disposition | Result |
| --- | --- | --- | --- |
| Behavior | TCP and Unix acceptance must cancel even after Unix unlink; ten-second/parent cancellation must drop the dial operation; no late connection; bounded multiplexing; two-way copy with EOF half-close; exact outer/per-connection error ownership | Tokio documents cancel-safe TCP/Unix accept and a direct `TcpSocket::connect(SocketAddr)` future owns its socket. But the oracle's default dialer also accepts hostnames, while Tokio resolves string targets in unabortable blocking work. The proposed numeric-only restriction lacks parity authority. The proposed fixed bound also changes overload behavior without an authorized value. Raw `copy_bidirectional` propagates shutdown errors that Go ignores. | **`fail`** pending the two authority decisions and corrected/proved shutdown-error handling |
| License and security | Permissive, no TLS/crypto/C service, no known advisory, bounded untrusted connection work | Both direct crates are MIT. The exact 18-package lock used only MIT, Apache-2.0, Unicode-3.0, and LLVM-exception combinations. A 1,211-advisory RustSec scan found no vulnerability. No TLS, resolver, signal, filesystem, process, or `unsafe` application code is selected. The application must enforce an explicitly authorized finite handler bound before accept. | `pass` conditional on that bound; behavior authority remains failed |
| Platforms and targets | Linux/macOS/Windows TCP; Linux/macOS Unix sockets; honest Windows exclusion | Tokio's published source names the three OS families. Rust 1.96 all-target checks passed for `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`, and `x86_64-pc-windows-gnu`. The native Linux probe covered loopback TCP and Unix cancellation after unlink. Cross-checks are compile evidence only; this record does not claim native non-Linux runtime parity or unverified OS-version floors. | `pass` for conditional dependency compatibility; does not override behavior failure |
| Maintenance and Rust version | Active current release, compatible with Rust 1.96, strong production adoption | Both exact releases were less than one month old at research time, declare MSRV 1.71, and have hundreds of millions of downloads. Tokio also publishes a rolling MSRV and LTS policy. | `pass` |
| Architectural constraints | Caller-owned runtime; async tasks rather than OS threads; bounded accepted work; no detached work; natural APIs; custom tunnel streams, not TCP-only results | `run` can become an async function on the caller's Tokio runtime and each handler can be one tracked task. The proposed probe and dial alias were TCP-only even though frozen SSH/tunnel callers return non-TCP `net.Conn` values. The Rust contract must use an object-safe `AsyncRead + AsyncWrite + Unpin + Send` stream abstraction and prove it with a non-TCP stream. | **`fail`** for the reviewed proposal; Tokio can support the corrected design |

## Candidate comparison

Point-in-time release/download/dependent evidence comes from the official
crates.io API on `2026-08-11`.

| Candidate | Behavior and platform fit | Adoption and maintenance | Integration cost | Disposition |
| --- | --- | --- | --- | --- |
| **Tokio `1.53.1` + tokio-util `0.7.19`** | First-party cancel-safe TCP/Unix accept, TCP socket futures, fixed-buffer bidirectional copy, semaphore, task set, timers, and cancellation token; Linux/macOS/Windows support. Raw copy shutdown errors and default hostname resolution need deliberate handling. | 868.4M/703.9M downloads; current July 2026 releases; 67,828/6,612 reverse-dependency rows; used across Hyper/Axum/Tonic and already the conditional runtime family in the repository's HTTP-server research. | Two direct crates; 17 registry packages in the cross-target probe lock; one proc macro dependency for `select!`/tests. | **Best conditional candidate, not approved while behavior gates fail.** |
| smol `2.0.2` family | Async TCP/Unix is possible, but equivalent cancellation tokens, task supervision, bounded lifecycle, and exact bidirectional half-close must be assembled from additional sibling crates; its facade does not document this complete lifecycle contract. | 20.4M downloads, 679 reverse-dependency rows; latest release 2024-09-07; MSRV 1.63. | Smaller pieces individually, more direct crates and more custom orchestration for this capability. | Rejected: materially less adopted and maintained for no behavior advantage. |
| async-std `1.13.2` | Similar high-level sockets but no first-party equivalent complete supervision/cancellation/copy stack. | Its official package metadata says "Deprecated in favor of `smol`"; 88.2M historical downloads and 1,918 reverse-dependency rows; latest release 2025-08-15. | Would choose a deprecated facade and still add lifecycle utilities. | Rejected by maintenance gate. |
| Mio/socket2 directly | Can express nonblocking TCP and platform readiness; Unix support is lower-level. | Tokio itself is the dominant maintained abstraction built on Mio/socket2. | Reimplements an executor, timers, cancellation, wake registration, task supervision, and copy state machines. | Rejected: unnecessary low-level networking/runtime implementation. |
| Standard-library threads (rejected candidate) | Blocking cloned accept cannot be cancelled reliably after Unix unlink; connect/copy workers can outlive timeout; two copy threads plus a dial thread per connection. | Standard library is stable, but this architecture has no maintained cancellation/runtime layer. | Unbounded OS threads and application-owned wakeup tricks. | Rejected by behavior and resource gates; this is the reviewed failing design. |
| Hickory Resolver `0.26.1` added to Tokio | Fully async DNS and system resolver configuration, but lookup methods document background task spawning and do not reproduce every libc/NSS hostname source. | Active, MSRV 1.88, permissive, and credible for applications that require DNS. | Large DNS protocol/cache/platform graph and a second lifecycle to supervise for a proxy whose known callers use numeric IPs/custom dialers. | Rejected for this capability: does not improve the required no-detached-work guarantee enough to justify semantic/build cost. |

## Conditional integration after authority and fresh review

### Exact versions and features

If and only if the blockers are resolved and a fresh reviewer approves the
corrected record, the integrator would add:

```toml
tokio = { version = "=1.53.1", default-features = false, features = ["io-util", "macros", "net", "rt", "sync", "time"] }
tokio-util = { version = "=0.7.19", default-features = false }
```

- `net` supplies TCP and cfg-Unix sockets; `io-util` supplies
  `copy_bidirectional` and `AsyncWriteExt::shutdown`; `rt` supplies spawned
  tasks and `JoinSet`; `sync` supplies `Semaphore`; `time` supplies the absolute
  dial deadline; and `macros` supplies unbiased `select!` and focused async test
  attributes. Do not enable `full`, `rt-multi-thread`, signal, process, fs,
  io-uring, tracing, or tokio-util `rt`.
- `CancellationToken` is in tokio-util's unconditional `sync` module. Keep
  tokio-util's default feature set empty.
- The library runs inside a caller-owned Tokio runtime. It must not build, enter,
  or drop a private runtime and must not use `block_on` or `spawn_blocking`.

### Required ownership and cancellation model

1. `Proxy::run` is async. It creates one child `CancellationToken`, one
   `JoinSet` containing every accepted handler, and a `Semaphore` with an
   explicitly authorized capacity. `256` was only the rejected proposal, not a
   derived or approved value. Acquire an owned permit before accepting;
   the listener backlog, rather than application memory or OS threads, absorbs
   excess connections. The permit stays in the handler task through dial and
   copy.
2. Use a two-stage unbiased selection: first race permit acquisition, completed
   handlers, and parent cancellation; after owning a permit, race the concrete
   listener's cancellation-safe `accept`, completed handlers, and cancellation.
   Do not accept before owning a permit. Reap every completed task. On
   cancellation, cancel the child token and stop accepting without a listener
   self-connect. This remains prompt if a Unix socket pathname has been removed.
3. On fatal accept failure, retain the source in `ProxyError::Accept`, cancel
   the child token, stop accepting, then drain the entire `JoinSet` before
   returning the accept error. On parent cancellation do the same drain and
   return `Ok(())`. Never detach a handler. Resume any handler panic after
   cleanup; Tokio task panics must not become silently successful proxy runs.
4. Under the proposed numeric-only exception, the default dialer would parse
   the target to `SocketAddr`, build the matching
   v4/v6 `TcpSocket`, and race `socket.connect(address)` against child
   cancellation and `sleep_until(start + 10s)`. Construct one fresh absolute
   deadline per accepted stream. Do not spawn the connect future. The losing
   future is dropped in the same handler before it can return or create a late
   connection.
5. Define an object-safe `AsyncStream` supertrait over `AsyncRead + AsyncWrite +
   Unpin + Send`, with a blanket implementation. A custom dialer is an async
   factory returning `Pin<Box<dyn Future<Output = io::Result<Box<dyn
   AsyncStream>>> + Send + 'static>>` and receives an owned child
   `CancellationToken`, network text exactly
   `"tcp"`, unchanged remote-address text, and the absolute ten-second
   `tokio::time::Instant`. The proxy, not the callback, races that future against
   both cancellation sources and drops it on loss.
6. Custom dialer contract: polling must be nonblocking and cooperative; all
   per-dial work and resources must be owned by the returned future; dropping
   it must prevent any later connection or side effect. It must not use
   `spawn_blocking`, detach Tokio tasks, start unmanaged threads, or retain a
   second connection-capable owner. The implementor must document this on the
   public API and test it with a pending future carrying a drop guard. A future
   whose `poll` blocks forever violates the contract just as a Go dialer that
   ignores its context violates `DialContext`.
7. After dial success, keep both arbitrary async streams in the handler. Wrap
   their write sides in a small adapter whose `poll_shutdown` polls the
   underlying shutdown but converts both success and failure to `Ok(())`; then
   race `copy_bidirectional` against child cancellation. This preserves clean
   EOF half-close while matching Go's ignored `CloseWrite` errors. On copy
   error or cancellation, leaving the handler drops both streams and the copy
   future; no separate copy task remains. Report a copy error only after both
   streams have been dropped and only if the child is not cancelled.

### Exact error rules

- Cancellation is control flow, not an error callback: parent cancellation or
  cancellation induced by a fatal accept error suppresses dial/copy errors.
  Parent cancellation returns `Ok(())` after the join set is empty.
- A fatal listener error returns `accept local connection: {source}` after
  handler drain and retains `source()`.
- A live-run dial error reports
  `connect remote address '<unchanged address>': {source}`. A ten-second expiry
  uses `io::ErrorKind::TimedOut` and source text `context deadline exceeded`;
  parent cancellation uses `Interrupted` internally but is suppressed.
- A live-run copy failure reports `data copy: {source}` and does not stop the
  accept loop. Tokio's single copy future returns the first polled I/O failure;
  the oracle's simultaneous two-goroutine failure order is scheduling-dependent
  and supplies no stronger deterministic ordering.
- Closed-error recognition traverses `Error::source()` and returns true only
  for explicit crate closed/closed-pipe sentinels and `io::ErrorKind::BrokenPipe`
  or `ConnectionReset` (the portable mappings for EPIPE/ECONNRESET). It must
  return false for generic `NotConnected`, `ConnectionAborted`, `TimedOut`,
  `Interrupted`, and Windows `WSAESHUTDOWN` raw code `10058`. Do not maintain an
  ad hoc cross-platform raw-number list when `ErrorKind` carries the exact two
  syscall classes.

### Known limitations and package acceptance obligations

- Tokio's system-hostname overload runs OS resolution in an unabortable blocking
  task. To satisfy the no-orphan-operation hard gate, the proposed default
  dialer accepts numeric `SocketAddr` targets only. All frozen direct callers
  provide numeric addresses or custom tunnel dialers. If parity authority says
  the otherwise-unused implicit Go hostname behavior is required, reopen this
  decision rather than silently enabling Tokio string resolution.
- `CancellationToken::cancel` is documented as not atomic across a token tree.
  After `cancel()` returns every child is cancelled; the run loop must always
  perform the final join-set drain and must not infer completion from one
  `is_cancelled` sample.
- A finite connection limit is required by this capability but changes the
  oracle's unbounded goroutine behavior. The controller/human must authorize the
  capacity and overload/backpressure behavior. Tests must then prove the bound,
  continued service after a permit is released, and prompt cancellation while
  saturated.
- Native non-Linux runtime evidence was unavailable in this research worktree.
  Package acceptance must run TCP cancellation/deadline/half-close tests on
  Windows and macOS and Unix unlink cancellation on macOS. The cross-target
  checks prove compile compatibility only.
- Tokio does not normalize OS I/O text or raw codes. Tests should assert stable
  wrapper text, `ErrorKind`, and source identity; exact platform error strings
  remain platform-defined just as Go wraps platform errors.

## Verification

The probe is intentionally outside the repository because this role owns only
this decision record: `/tmp/ployz-async-runtime-probe`.

```sh
cargo test --offline --all-targets --manifest-path /tmp/ployz-async-runtime-probe/Cargo.toml
cargo clippy --offline --all-targets --manifest-path /tmp/ployz-async-runtime-probe/Cargo.toml -- -D warnings
cargo tree --offline -e features --manifest-path /tmp/ployz-async-runtime-probe/Cargo.toml
cargo check --offline --all-targets --target x86_64-unknown-linux-gnu --manifest-path /tmp/ployz-async-runtime-probe/Cargo.toml
cargo check --offline --all-targets --target x86_64-apple-darwin --manifest-path /tmp/ployz-async-runtime-probe/Cargo.toml
cargo check --offline --all-targets --target x86_64-pc-windows-gnu --manifest-path /tmp/ployz-async-runtime-probe/Cargo.toml
cargo audit --no-fetch --file /tmp/ployz-async-runtime-probe/Cargo.lock
```

Results on Rust/Cargo `1.96.0`:

- five narrow Linux runtime probes passed: deadline/parent racing drops a
  pending TCP-only custom future, direct socket connect succeeds, a standalone
  semaphore/task batch respects its bound, raw bidirectional copy half-closes on
  success, and Unix accept cancellation works after unlink. They did **not**
  prove cancellation of an in-flight connect/no late accept, a saturated real
  accept loop, fatal-accept drain, task/callback panic cleanup, copy-error
  timing, shutdown-error suppression, or a non-TCP custom dialer;
- warnings-denied Clippy passed;
- Linux, Intel macOS, and Windows GNU all-target checks passed;
- the exact graph contained 17 registry packages plus the probe, with no extra
  async runtime, DNS resolver, TLS, crypto, signal, process, or filesystem
  feature;
- the local 1,211-entry RustSec database reported no vulnerability. Re-run
  license and advisory checks against the integrated lock.

## Review

Fresh adversarial dependency reviewer:
`/root/async_stream_runtime_research/adversarial_commit_review`.

Reviewed exact candidate `3173cbf32d21f1a5f7a85f2a6143a7d75e2b8c72`
with parent `689686e18be585b3d4233f8fa6df36486315f440`.
Verdict: **REJECT**.

Blocking findings:

1. Numeric-only default dialing and a fixed 256-handler cap change observable
   oracle behavior without recorded controller/human parity authority. The
   record itself left hostname scope unresolved while marking the behavior gate
   partially passing.
2. Raw Tokio `copy_bidirectional` propagates `AsyncWrite::shutdown` errors, but
   Go ignores `CloseWrite` errors. The proposed direct use was not exact parity.
3. The custom dialer/result/probe used `TcpStream`, while frozen SSH/tunnel
   dialers can return non-TCP stream implementations. Arbitrary async stream
   compatibility was neither specified nor tested.
4. The probe did not exercise the claimed in-flight connect cancellation/no
   late accept, real saturated accept behavior, fatal-accept drain,
   task/callback panic cleanup, copy-error/shutdown-error semantics, or non-TCP
   custom streams. It also overstated the registry-package count and platform
   link evidence.

Tokio remains the strongest candidate, and the shutdown adapter/arbitrary-stream
corrections above are technically straightforward. Approval still requires:

- a human/controller decision either preserving hostname behavior with a
  separately researched cancellable resolver or explicitly accepting
  numeric-only default dialing;
- an authorized finite connection limit and overload behavior;
- corrected executable probes for every rejected claim; and
- a fresh adversarial review of a new exact committed candidate.
