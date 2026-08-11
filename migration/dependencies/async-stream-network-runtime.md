# Dependency decision: `async-stream-network-runtime`

| Field | Value |
| --- | --- |
| Status | `blocked` pending the mandatory fresh adversarial review below |
| Capability | Bounded, cancellation-safe TCP/Unix stream acceptance, deadline-bounded TCP dialing, supervised bidirectional copying, and write-half shutdown for `internal/proxy` |
| Selected dependency | Provisional: Tokio `1.53.1` and tokio-util `0.7.19` |
| License | `MIT` (both direct crates) |
| Research date | `2026-08-11` UTC |
| Request | Delegated capability request; no request file exists at the assigned integration base |
| Integration base | `689686e18be585b3d4233f8fa6df36486315f440` |
| Blocked package evidence | `/tmp/ployz-proxy-writer` commit `1040794b0e5696afd4ee1145b64902033217a0fa` |
| Affected package | `ployz-internal-proxy` (`upstream/uncloud/internal/proxy`) |

The selected stack is the popular, idiomatic passing candidate. It replaces
the rejected implementation's blocking listener clones and per-connection OS
threads with cancellation-safe readiness futures and supervised runtime tasks.
This record approves runtime primitives, not a private runtime or a Go-shaped
threading adapter.

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
  hidden tasks.
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
- Tokio's
  [platform policy](https://docs.rs/tokio/1.53.1/src/tokio/lib.rs.html#397-421)
  lists Linux, Windows 10+, and macOS 10.15+ among supported platforms. Unix
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
| Behavior | TCP and Unix acceptance must cancel even after Unix unlink; ten-second/parent cancellation must drop the dial operation; no late connection; bounded multiplexing; two-way copy with EOF half-close; exact outer/per-connection error ownership | Tokio documents cancel-safe TCP/Unix accept. The direct `TcpSocket::connect(SocketAddr)` future owns its socket. `copy_bidirectional` supplies the exact clean-EOF half-close state machine. `Semaphore`, `JoinSet`, and `CancellationToken` provide an explicit bound and drain. The local probe exercised all of these. The string DNS overload is forbidden because it introduces unabortable blocking resolution. | `pass` for numeric default targets and async custom dialers under the contract below; hostname limitation requires explicit scope treatment |
| License and security | Permissive, no TLS/crypto/C service, no known advisory, bounded untrusted connection work | Both direct crates are MIT. The exact 18-package lock used only MIT, Apache-2.0, Unicode-3.0, and LLVM-exception combinations. A 1,211-advisory RustSec scan found no vulnerability. No TLS, resolver, signal, filesystem, process, or `unsafe` application code is selected. The application must enforce a 256-handler permit bound before accept. | `pass` |
| Platforms and targets | Linux/macOS/Windows TCP; Linux/macOS Unix sockets; honest Windows exclusion | Tokio officially supports the three OS families. Rust 1.96 all-target checks passed for `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`, and `x86_64-pc-windows-gnu`. The native Linux probe covered loopback TCP and Unix cancellation after unlink. Cross-checks are compile evidence, so native macOS/Windows runtime tests remain package-acceptance obligations. | `pass` for dependency selection |
| Maintenance and Rust version | Active current release, compatible with Rust 1.96, strong production adoption | Both exact releases were less than one month old at research time, declare MSRV 1.71, and have hundreds of millions of downloads. Tokio also publishes a rolling MSRV and LTS policy. | `pass` |
| Architectural constraints | Caller-owned runtime; async tasks rather than OS threads; bounded accepted work; no detached work; natural APIs | `run` becomes an async function on the caller's Tokio runtime. Each handler is one tracked task. The dial and copy operations remain futures inside that task. The proxy uses no private runtime, `spawn_blocking`, raw socket FFI, clone-and-wake listener trick, or compatibility facade for a Go dependency. | `pass` |

## Candidate comparison

Point-in-time release/download/dependent evidence comes from the official
crates.io API on `2026-08-11`.

| Candidate | Behavior and platform fit | Adoption and maintenance | Integration cost | Disposition |
| --- | --- | --- | --- | --- |
| **Tokio `1.53.1` + tokio-util `0.7.19`** | First-party cancel-safe TCP/Unix accept, TCP socket futures, fixed-buffer bidirectional copy with half-close, semaphore, task set, timers, and cancellation token; Linux/macOS/Windows support. | 868.4M/703.9M downloads; current July 2026 releases; 67,828/6,612 reverse-dependency rows; used across Hyper/Axum/Tonic and already the selected runtime family in the repository's HTTP-server research. | Two direct crates; 18-package cross-target probe lock including the probe; one proc macro dependency for `select!`/tests. | **Selected: most idiomatic and widely adopted passing stack.** |
| smol `2.0.2` family | Async TCP/Unix is possible, but equivalent cancellation tokens, task supervision, bounded lifecycle, and exact bidirectional half-close must be assembled from additional sibling crates; its facade does not document this complete lifecycle contract. | 20.4M downloads, 679 reverse-dependency rows; latest release 2024-09-07; MSRV 1.63. | Smaller pieces individually, more direct crates and more custom orchestration for this capability. | Rejected: materially less adopted and maintained for no behavior advantage. |
| async-std `1.13.2` | Similar high-level sockets but no first-party equivalent complete supervision/cancellation/copy stack. | Its official package metadata says "Deprecated in favor of `smol`"; 88.2M historical downloads and 1,918 reverse-dependency rows; latest release 2025-08-15. | Would choose a deprecated facade and still add lifecycle utilities. | Rejected by maintenance gate. |
| Mio/socket2 directly | Can express nonblocking TCP and platform readiness; Unix support is lower-level. | Tokio itself is the dominant maintained abstraction built on Mio/socket2. | Reimplements an executor, timers, cancellation, wake registration, task supervision, and copy state machines. | Rejected: unnecessary low-level networking/runtime implementation. |
| Standard-library threads (rejected candidate) | Blocking cloned accept cannot be cancelled reliably after Unix unlink; connect/copy workers can outlive timeout; two copy threads plus a dial thread per connection. | Standard library is stable, but this architecture has no maintained cancellation/runtime layer. | Unbounded OS threads and application-owned wakeup tricks. | Rejected by behavior and resource gates; this is the reviewed failing design. |
| Hickory Resolver `0.26.1` added to Tokio | Fully async DNS and system resolver configuration, but lookup methods document background task spawning and do not reproduce every libc/NSS hostname source. | Active, MSRV 1.88, permissive, and credible for applications that require DNS. | Large DNS protocol/cache/platform graph and a second lifecycle to supervise for a proxy whose known callers use numeric IPs/custom dialers. | Rejected for this capability: does not improve the required no-detached-work guarantee enough to justify semantic/build cost. |

## Selected integration

### Exact versions and features

Only the integrator may add these workspace dependencies:

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
   `JoinSet` containing every accepted handler, and a `Semaphore` with
   `DEFAULT_MAX_CONNECTIONS = 256`. Acquire an owned permit before accepting;
   the listener backlog, rather than application memory or OS threads, absorbs
   excess connections. The permit stays in the handler task through dial and
   copy.
2. Race permit acquisition, the concrete listener's cancellation-safe `accept`,
   completed handlers, and parent cancellation with unbiased `tokio::select!`.
   Reap every completed task. On cancellation, cancel the child token and stop
   accepting without a listener self-connect. This remains prompt if a Unix
   socket pathname has already been removed.
3. On fatal accept failure, retain the source in `ProxyError::Accept`, cancel
   the child token, stop accepting, then drain the entire `JoinSet` before
   returning the accept error. On parent cancellation do the same drain and
   return `Ok(())`. Never detach a handler. Resume any handler panic after
   cleanup; Tokio task panics must not become silently successful proxy runs.
4. The default dialer must parse the target to `SocketAddr`, build the matching
   v4/v6 `TcpSocket`, and race `socket.connect(address)` against child
   cancellation and `sleep_until(start + 10s)`. Construct one fresh absolute
   deadline per accepted stream. Do not spawn the connect future. The losing
   future is dropped in the same handler before it can return or create a late
   connection.
5. A custom dialer is an async factory returning a `Send + 'static` boxed
   future and receives an owned child `CancellationToken`, network text exactly
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
7. After dial success, keep both streams in the handler and race
   `copy_bidirectional(&mut local, &mut remote)` against child cancellation.
   Clean EOF is handled by `copy_bidirectional`'s `shutdown` state. On copy
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
  task. To satisfy the no-orphan-operation hard gate, the approved default
  dialer accepts numeric `SocketAddr` targets only. All frozen direct callers
  provide numeric addresses or custom tunnel dialers. If parity authority says
  the otherwise-unused implicit Go hostname behavior is required, reopen this
  decision rather than silently enabling Tokio string resolution.
- `CancellationToken::cancel` is documented as not atomic across a token tree.
  After `cancel()` returns every child is cancelled; the run loop must always
  perform the final join-set drain and must not infer completion from one
  `is_cancelled` sample.
- The 256-connection limit is an intentional resource bound required by this
  capability. Pending kernel-backlog connections can time out or be refused
  under load, unlike the oracle's unbounded goroutine creation. Tests must prove
  the bound, continued service after a permit is released, and prompt
  cancellation while saturated.
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

- five Linux runtime probes passed: ten-second/parent racing drops the custom
  future, direct socket connect, bounded task multiplexing and drain,
  bidirectional copy with both EOF half-closes, and Unix accept cancellation
  after unlink;
- warnings-denied Clippy passed;
- Linux, Intel macOS, and Windows GNU all-target checks passed;
- the exact graph contained 18 registry packages plus the probe, with no extra
  async runtime, DNS resolver, TLS, crypto, signal, process, or filesystem
  feature;
- the local 1,211-entry RustSec database reported no vulnerability. Re-run
  license and advisory checks against the integrated lock.

## Review

This networking/runtime decision requires a fresh adversarial dependency
review. The review is in progress; this record remains blocked and must not be
entered as approved until the reviewer result is incorporated here.

