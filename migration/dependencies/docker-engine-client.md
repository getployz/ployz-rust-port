# Dependency decision: `docker-engine-client`

| Field | Value |
| --- | --- |
| Status | `blocked` |
| Capability | Async Docker Engine API client for local and configured daemons, including typed container/image operations, API-version compatibility, streaming pull/push responses, cancellation, and daemon error classification |
| Provisional selected dependency | `bollard = { version = "=0.21.0", default-features = false, features = ["pipe", "ssl"] }` |
| License | `Apache-2.0` |
| Research date | `2026-08-11` UTC |
| Request | No request file was present; capability was delegated directly for `upstream/uncloud/internal/docker` |
| Exact blocker | Approval remains blocked on three falsifiable gates: D01 requires explicit human authority for the frozen plain-TCP and unverified-TLS environment behavior; D03 requires native exact-feature/exact-lock evidence on shipped macOS amd64/arm64 and Linux arm64; and D04 requires a declared Docker Engine/API support floor plus the complete caller-operation matrix at that floor. New Go-vs-Rust probes close the prior D02 lazy-construction and D05 stream-cancellation findings for this candidate. |

## Verdict

`bollard` 0.21.0 remains the clear idiomatic and adoption leader and the only
candidate that covers the required Engine API surface without local protocol
reimplementation. It remains the provisional selection. Executable follow-up
probes close the prior lazy-construction and stream-cancellation findings, but
the candidate cannot be approved while D01, D03, and D04 remain open.

Approval must retain all integration requirements in this record. In
particular, Bollard's documented `negotiate_version` mutates its stored client
version, but its 0.21.0 URI implementation emits unversioned request paths. The
probe observed `/version` followed by `/_ping` and `/images/create`, not the
expected `/v1.41/...` paths. Docker documents unversioned endpoints as
deprecated. The package must negotiate first and then use Bollard's public
`with_request_modifier` hook to prefix the negotiated version; the corrected
probe observed `/version` followed by `/v1.41/_ping`,
`/v1.41/images/create`, `/v1.41/images/missing/json`, and
`/v1.41/images/example/push`.

## Primary-source evidence

### Oracle and caller requirements

- The frozen package wraps Docker's Go client, retries only connection failures
  while waiting for the daemon, retries container creation after a missing-image
  pull, polls inspect data until a port binding appears, decodes pull/push JSON
  progress one object at a time, surfaces an embedded stream error, propagates
  cancellation through the stream, and obtains registry credentials from the
  local Docker configuration: [`client.go`](../../upstream/uncloud/internal/docker/client.go),
  [`container.go`](../../upstream/uncloud/internal/docker/container.go), and
  [`image.go`](../../upstream/uncloud/internal/docker/image.go).
- Direct callers create clients from Docker environment variables with API
  negotiation, inspect/tag/remove images, select an OCI platform for a push,
  create/start/remove helper containers, inspect daemon info, and consume every
  pull/push progress item: [`machine.go`](../../upstream/uncloud/internal/machine/machine.go)
  and [`pkg/client/image.go`](../../upstream/uncloud/pkg/client/image.go).
- Downstream machine packages additionally need ping/version/info, typed
  container/image/network lists and inspections, event and log streams, and
  status-code classification. Their log behavior includes both raw TTY output
  and Docker's multiplexed stdout/stderr format:
  [`service.go`](../../upstream/uncloud/internal/machine/docker/service.go),
  [`controller.go`](../../upstream/uncloud/internal/machine/docker/controller.go),
  and [`service_test.go`](../../upstream/uncloud/internal/machine/docker/service_test.go).
- The shipped clients are Linux and macOS on amd64 and arm64; the daemon is
  Linux-only. Windows is commented out in the frozen release definition:
  [`.goreleaser.yaml`](../../upstream/uncloud/.goreleaser.yaml).

### Selected dependency

- Bollard 0.21.0's [published manifest](https://docs.rs/crate/bollard/0.21.0/source/Cargo.toml.orig)
  is Apache-2.0, edition 2021, pure Cargo/Rust at its public boundary, and
  exposes narrowly selectable transports. `pipe` supplies Unix sockets and
  Windows named pipes; `ssl` supplies HTTP plus rustls using Ring, native root
  loading, and Docker client certificates. BuildKit, SSH, WebSocket, date/time,
  and test features remain disabled.
- Its [connection documentation](https://docs.rs/bollard/0.21.0/bollard/struct.Docker.html#method.connect_with_defaults)
  covers `DOCKER_HOST`, Unix sockets, named pipes, plain HTTP, HTTPS, client
  certificates from `DOCKER_CERT_PATH`, and TLS selection through
  `DOCKER_TLS_VERIFY`. The exact [TLS source](https://docs.rs/crate/bollard/0.21.0/source/src/docker.rs)
  uses rustls, native roots plus `ca.pem`, and `cert.pem`/`key.pem` client
  authentication.
- The exact connection source also proves an environment mismatch that generic
  connection documentation obscures. Bollard selects TLS for a `tcp://` host
  whenever `DOCKER_TLS_VERIFY` merely exists, even if it is empty, and otherwise
  ignores `DOCKER_CERT_PATH` for that host. The frozen Go `FromEnv` path instead
  enables TLS whenever `DOCKER_CERT_PATH` is non-empty and disables server
  verification when `DOCKER_TLS_VERIFY` is empty. An adapter cannot reconcile
  these cases by calling `connect_with_defaults`; D01 must explicitly settle
  them before the adapter chooses a constructor.
- The natural API supplies typed create/start/stop/remove/inspect/list
  operations, info/version/ping, network operations, event streams, log streams,
  and status-bearing `DockerResponseServerError`. Image pull and push are
  asynchronous JSON streams, turn Engine `errorDetail.message` into a stream
  error, accept structured registry credentials, and expose the push `platform`
  query: [container API](https://docs.rs/crate/bollard/0.21.0/source/src/container.rs),
  [image API](https://docs.rs/crate/bollard/0.21.0/source/src/image.rs),
  [system API](https://docs.rs/crate/bollard/0.21.0/source/src/system.rs), and
  [errors](https://docs.rs/crate/bollard/0.21.0/source/src/errors.rs).
- Bollard generates models for Engine API 1.53 and can lower its stored version
  after querying `/version`. Docker's official [Engine API versioning documentation](https://docs.docker.com/reference/api/engine/)
  explains why clients must use the highest version supported by both sides and
  that downgraded requests/responses are adjusted for the selected version.
- The exact [URI source](https://github.com/fussybeaver/bollard/blob/v0.21.0/src/uri.rs#L35-L44)
  first formats a versioned base but then calls `url.join(path)` with an
  absolute path, removing the version prefix. The executable probe confirmed
  this behavior. Docker's [versioned API reference](https://docs.docker.com/reference/api/engine/version/v1.46/)
  says omitted prefixes select the daemon's current API and are deprecated.
  Bollard's public [request modifier](https://docs.rs/bollard/0.21.0/bollard/struct.Docker.html#method.with_request_modifier)
  is sufficient to restore the negotiated prefix without forking the crate or
  implementing an HTTP client.
- The official [crates.io API](https://crates.io/api/v1/crates/bollard) reported
  44,416,897 total downloads, 13,615,453 recent downloads, 48 releases, and
  non-yanked 0.21.0 published 2026-05-04. Its
  [reverse-dependency API](https://crates.io/api/v1/crates/bollard/reverse_dependencies?page=1&per_page=10)
  reported 287 dependent crates. The official
  [GitHub repository API](https://api.github.com/repos/fussybeaver/bollard)
  reported 1,346 stars, 185 forks, an unarchived repository, and a push on
  2026-08-11. These are 2026-08-11 snapshots, not stable guarantees.
- Established-project use includes Vector's current
  [Bollard dependency](https://github.com/vectordotdev/vector/blob/master/Cargo.toml),
  configured without defaults and with the exact transport/TLS pattern relevant
  here. Bollard's tagged first-party [Linux CI](https://github.com/fussybeaver/bollard/blob/v0.21.0/.circleci/config.yml)
  exercises Docker 29.3 over Unix, HTTP, and mutual TLS, and its
  [Windows CI](https://github.com/fussybeaver/bollard/blob/v0.21.0/appveyor.yml)
  exercises the Docker named pipe.

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Typed container/image/network/info operations; missing-image status classification; streaming pull/push progress and stream errors; platform-aware push; cancellation; API compatibility | Public APIs cover the required surface. Rust 1.96 loopback probes verified negotiated downgrade state, required version-prefix correction, streamed pull/push objects, platform encoding, empty push auth, typed 404 errors, missing/refused Unix-socket reconstruction, narrow retry classification, cancellation-as-success for daemon startup, prompt pull/push transport EOF, producer termination, and exactly-once output closure. The matching Go probe characterized the frozen post-progress cancellation result as the wrapped text `decode image pull/push message: context canceled`. The oldest supported Engine/API and its complete operation matrix remain undeclared and untested. | **`blocked`: D04; D02 and D05 closed** |
| License and security | Apache-2.0-compatible permissive graph; no known vulnerability; safe local socket and verified mutual-TLS modes | Direct license is Apache-2.0. All 120 external packages in the exact all-target probe graph used Apache/MIT/ISC/BSD/Unicode/Unlicense/Zlib-family compatible terms. RustSec found no vulnerability or warning in the exact lock. No package-owned `unsafe` or raw socket FFI is needed; Ring and platform certificate/socket crates encapsulate their native/unsafe internals. Docker warns that daemon access is effectively host-root access, so endpoints remain trusted configuration. The frozen environment behavior can select plain TCP or unverified TLS, while this record intentionally permits only local sockets or verified TLS. | **`blocked`: D01 requires human authority** |
| Platforms and targets | Shipped Linux/macOS amd64/arm64, Unix-socket local daemon, configured TCP/mTLS | Linux x86_64 Rust 1.96 run/check passed. `pipe`'s Unix connector is target-gated portable Tokio/hyperlocal code; `ssl` uses rustls and native roots. The Linux-to-Intel-macOS check reached Ring's C compilation and failed because no Apple target C compiler/SDK is installed. Bollard has first-party Linux/Windows transport integration CI, but neither cross-compilation nor upstream CI proves the shipped native targets under this exact feature set and lock. | **`blocked`: D03** |
| Maintenance and Rust version | Active current release compatible with workspace Rust 1.96 | 0.21.0 was current and non-yanked; repository activity continued on the research date. The crate declares no `rust-version`, but the exact graph built, ran, and passed warnings-denied Clippy with Rust 1.96.0. | `pass` |
| Architectural constraints | Idiomatic async API; no subprocess or bespoke Docker REST implementation; bounded build/runtime surface | Bollard uses Tokio/futures streams and generated Engine models directly. Exact features exclude BuildKit, SSH, WebSocket, chrono/time, and code generation at consumer build time. The focused all-target lock contains 120 external packages, largely the already-idiomatic Tokio/Hyper/Rustls stack. | `pass` |
| Critical review | Fresh adversarial review for container control | The prior review of `89bddf3` rejected D01-D05. The follow-up probes close D02 and D05; this revised blocked record requires fresh adversarial review to confirm those closures and the narrowed D01/D03/D04 blocker. | `pending fresh review` |

## Candidate comparison

Counts are official crates.io snapshots from 2026-08-11.

| Candidate | Hard-gate fit | Maintenance and adoption | Decision |
| --- | --- | --- | --- |
| [`bollard` 0.21.0](https://crates.io/crates/bollard/0.21.0) | Complete typed async Engine surface, streams, error status, Unix/named-pipe/HTTP/mTLS transports, platform push, and public request modification. Requires the version-prefix policy in this record. | 44.4M total / 13.6M recent downloads; 287 reverse dependents; 48 releases; current release 2026-05-04; active repository. | **Provisional selection:** overwhelmingly most popular idiomatic passing client. |
| [`dockworker` 0.17.0](https://crates.io/crates/dockworker/0.17.0) | Broad async CRUD and current maintenance, but its [source](https://github.com/Idein/dockworker/blob/v0.17.0/src/docker.rs) explicitly defaults Windows to TCP because named pipes are unsupported, has no negotiated/versioned request policy, and `push_image` returns only `Result<()>` rather than the required progress stream. | 261,831 total / 12,449 recent downloads; 4 reverse dependents; released 2026-05-18; active repository. | Rejected at behavior/platform gates and far less adopted. |
| [`docker-api` 0.14.0](https://crates.io/crates/docker-api/0.14.0) | Its [source](https://github.com/vv9k/docker-api-rs/blob/v0.14.0/src/docker.rs) models API 1.42 and supports Unix/TCP plus optional OpenSSL TLS, but not named pipes or the oracle's negotiated client behavior. It is tied to Hyper 0.14 and an older generated schema. | 641,396 total / 64,856 recent downloads; 13 reverse dependents; last release 2023-06-05 and last repository push 2024-05-24. | Rejected at active-maintenance and behavior gates. |
| [`shiplift` 0.7.0](https://crates.io/crates/shiplift/0.7.0) | Async Unix/TCP/TLS client, but an old manually maintained API surface, OpenSSL default, no current Engine version negotiation, and no named-pipe support. | 930,779 total / 98,035 recent downloads; 9 reverse dependents; last release 2021-02-21 and last push 2023-10-02. | Rejected at active-maintenance and behavior gates. |
| [`bollard-next` 0.18.1](https://crates.io/crates/bollard-next/0.18.1) | A temporary alternate publication of the same design, pinned to older Engine 1.45 stubs and lacking current upstream improvements. | 99,923 total / 2,819 recent downloads; 7 reverse dependents; last release 2024-10-19 versus active mainline Bollard 0.21.0. | Rejected: obsolete, much less adopted duplicate of the selected project. |
| Direct Docker REST implementation | Could reproduce only the currently used endpoints. | No ecosystem maintenance, schema generation, cross-platform connector, or security review; would duplicate status, streaming, versioning, TLS, and transport logic already maintained by Bollard. | Rejected by architectural gate. |

## Adversarial gates

D01, D03, and D04 are blocking evidence requirements. D02 and D05 are retained
as closed findings so a fresh reviewer can audit the exact closure evidence
rather than relying on a summary.

### D01 — Human authority for environment and transport parity

The frozen Go `FromEnv` behavior permits both plain remote TCP and TLS with
server verification disabled when `DOCKER_CERT_PATH` is set but
`DOCKER_TLS_VERIFY` is empty. The provisional Bollard policy intentionally
supports the local Unix socket and verified TLS only. A controller or other
human authority must provide an explicit supported-environment matrix stating
whether plain TCP and unverified-TLS parity is required or explicitly
unsupported. If unverified TLS is required, this selection fails the security
gate and becomes `human-decision-required`; it must not be approved by silently
adding a permissive certificate verifier. The matrix must also settle whether
trusted plain TCP, if any, is limited to loopback or another precisely bounded
environment.

### D02 — Closed: lazy construction and daemon-start retry

Bollard 0.21.0's Unix constructor checks that the socket path exists and can
return `SocketNotFoundError` before any Engine request is made. The Rust 1.96
probe proved a viable, dependency-natural policy: retain validated connection
configuration rather than a failed client and reconstruct `Docker` on every
daemon-readiness attempt. It passed:

- a missing socket at startup that subsequently appears;
- connection refusal followed by daemon readiness;
- cancellation returning success, matching frozen `WaitDaemonReady`; and
- a 401 `DockerResponseServerError` remaining permanent.

Only `SocketNotFoundError` and `HyperLegacyError` whose inner error reports
`is_connect()` were retryable. The first reconstruction occurred after the
100 ms initial wait and the next after the 150 ms multiplied wait; the probe did
not busy-loop. Exact randomized exponential scheduling through the frozen
100 ms initial interval, 1.5 multiplier, 0.5 randomization factor, and 1 s cap is
package-owned behavior and needs a separately approved backoff capability; it
is not a reason to reject Bollard. The package must not replace it with the
deterministic probe scheduler.

### D03 — Native shipped-platform evidence

Provide native, exact-feature and exact-lock evidence for macOS amd64, macOS
arm64, and Linux arm64. For each target, preserve the host OS/architecture,
Rust toolchain, dependency line, and lockfile identity, and run build/check plus
warnings-denied Clippy. Runtime evidence must cover Docker Desktop's Unix socket
on both macOS architectures, the Unix socket on Linux arm64, and verified TLS
where that transport is shipped. Linux-to-macOS cross-compilation and Bollard's
upstream Windows CI do not close this gate.

### D04 — Declared Docker Engine/API support floor

A controller or human authority must declare the oldest Docker Engine and API
version that Ployz supports. Against that floor, test the caller-required
operations through the corrected request modifier: ping/version, container
create/inspect/start/remove, image pull/push/inspect/tag/remove including auth
and OCI platform selection, daemon info, and the network/event/log surfaces
used by downstream machine code. The transcript must prove that `/version` is
unprefixed and every subsequent operation uses `/v{selected}/...`; it must also
cover `DOCKER_API_VERSION` override precedence and old-daemon/fallback behavior.
The installed CLI version and the current fake Engine do not establish a
support floor.

### D05 — Closed: pull and push cancellation

Separate slow pull and push probes used controllable TCP fake Engines. After
headers and one progress item, the frozen Go response loop and Bollard adapter
both proved:

- the TCP connection closes promptly, observed as server EOF/reset within one
  second;
- the producer/forwarder task terminates and its output channel closes exactly
  once;
- no progress arrives after cancellation and no task remains orphaned.

The Go probe copied the frozen response-loop function byte-for-byte because the
oracle module declares an unavailable Go 1.26 toolchain; it used the oracle's
exact Docker 28.5.0 client. Both operations produced one `first` progress item,
then the exact wrapped error text
`decode image pull/push message: context canceled`, then channel closure, while
the server observed request cancellation. This is a frozen limitation: cancel
while the decoder is blocked is surfaced as a decode error wrapping
`context.Canceled`, not as a bare cancellation error. The Rust adapter probe
matched that observable category/text, dropped the Bollard stream before
reporting it, closed its receiver once, joined the producer, and observed the
server-side EOF/reset. Package tests must use a typed error whose source remains
cancellation rather than storing only a string.

A gated-reader Go probe also fixed the ordering contract that the transport
probe alone could not distinguish:

- cancellation already ready when a decoded stream error reaches the
  post-decode select produces bare `context canceled`, suppressing that stream
  error;
- a decoded stream error whose unbuffered send is already blocked before
  cancellation still delivers that stream error (`boom` in the probe); and
- EOF observed before cancellation closes the channel without a cancellation
  item.

These are ordered outcomes, not a promise that wall-clock races always pick one
result. The Rust forwarder must preserve them: once an output send is pending it
must not be retroactively replaced; EOF already observed wins; otherwise a
ready cancellation after a decoded item wins, while cancellation of a pending
body read retains the wrapped-decode form proven above. Add deterministic
package tests for all four orderings.

## Selected integration

If D01, D03, and D04 are closed and a fresh re-review accepts the evidence, use exactly:

```toml
bollard = { version = "=0.21.0", default-features = false, features = ["pipe", "ssl"] }
```

The integrator may expose shared versions of Tokio, `futures-util`, and `http`
already required by the natural async implementation; this decision does not
authorize edits to the root manifest. Do not add `bollard-stubs` directly:
Bollard re-exports its generated `models` and `query_parameters`.

### Mandatory API-version policy

1. Construct the transport from trusted Docker configuration. For the shipped
   Unix targets, the default is `unix:///var/run/docker.sock`; honor a non-empty
   `DOCKER_HOST`. Use verified mutual TLS for the supported remote TLS mode.
2. If non-empty `DOCKER_API_VERSION` is present, validate `MAJOR.MINOR`, skip
   negotiation, and use that exact value as the request prefix. This preserves
   the frozen Go client's manual-override precedence.
3. Otherwise query the daemon before normal requests, cap the server version at
   Bollard's `API_DEFAULT_VERSION`, and retain the selected value. The machine
   startup path must still retry connection failures with the oracle's backoff;
   do not make connection construction eagerly turn a not-yet-running daemon
   into a permanent error.
4. Only after selecting the version, install `with_request_modifier`. Preserve
   the original scheme, authority, path, and query while replacing the
   path-and-query with `/v{selected_version}{original_path_and_query}`. Leave
   the negotiation `/version` request unmodified.
5. Port tests must assert the request transcript. A check of
   `client_version()` alone is insufficient because the unmodified 0.21.0 URI
   builder still emitted unversioned paths in the executable probe.

The public modifier shape proven by the probe was:

```rust
let version = docker.client_version().to_string();
let docker = docker.with_request_modifier(move |request| {
    let (mut parts, body) = request.into_parts();
    let original = parts.uri.path_and_query().expect("path").as_str().to_owned();
    let mut uri = parts.uri.into_parts();
    uri.path_and_query = Some(format!("/v{version}{original}").parse().expect("path/query"));
    parts.uri = http::Uri::from_parts(uri).expect("URI parts");
    http::Request::from_parts(parts, body)
});
```

Production code must return typed configuration errors rather than use the
`expect` calls shown in this compact probe excerpt.

### Natural API and cancellation policy

- Use `Docker` directly inside an idiomatic Rust wrapper. Do not recreate the
  Go client's embedded API or method names.
- Consume `create_image` and `push_image` as `Stream<Item = Result<...>>` and
  stop at the first error. Map the generated status/id/progress/current/total
  fields to Ployz progress values. Bollard already promotes
  `errorDetail.message` to `DockerStreamError`; do not decode the byte stream a
  second time.
- Cancellation drops the in-flight future/stream and its response body. Any
  task that forwards progress must select on its cancellation token and drop
  the stream before reporting cancellation. Never detach a pull/push task. When
  cancellation interrupts a blocked progress decode, preserve the frozen
  wrapped `decode image pull/push message: context canceled` presentation and a
  typed cancellation source; do not normalize it to a bare cancellation error.
- Match `DockerResponseServerError { status_code: 404, .. }` for the oracle's
  missing-image retry and other not-found branches. Match transport/connect
  errors narrowly for daemon-start retry; authorization, TLS, JSON, API, and
  other daemon errors remain permanent.
- Keep the oracle's explicit 5-second port-publication deadline and 10 ms poll
  interval in package code. Bollard's transport timeout covers only waiting for
  response headers and is not a substitute.
- Use generated container/image/network/system models and query/body builders.
  Preserve unknown daemon response properties by normal Serde open-struct
  behavior; never reject a response only for extra fields.

### Registry authentication boundary

Bollard accepts and encodes `DockerCredentials` and correctly sends an encoded
empty object for unauthenticated pushes, matching the oracle's Docker workaround.
It does **not** read `~/.docker/config.json`, invoke credential helpers, or apply
Docker Hub registry-name normalization. `RetrieveLocalDockerRegistryAuth` is a
separate local-credential-resolution capability. The controller must either
reference an approved decision for that capability or create a dependency
request before implementing that function; this record does not approve a
credential-helper executable or a hand-written credential-store protocol.

## License and security notes

- Bollard and its generated stubs are Apache-2.0, compatible with the frozen
  project's Apache-2.0 license. The exact all-target probe graph had no
  copyleft-only or unknown third-party license.
- `cargo audit --no-fetch --deny warnings` found no vulnerability or
  informational warning using 1,211 RustSec advisories from advisory-db commit
  `d0861df1eab469d3c58d6b836ce48b5766e5f217` dated 2026-08-11. Re-run against
  the integrated lock.
- Docker's official [security documentation](https://docs.docker.com/engine/security/)
  states that daemon control is effectively host-root access and warns against
  unauthenticated TCP exposure. Default to the permission-protected Unix socket;
  remote endpoints are trusted operator configuration and require verified TLS.
- The frozen Go `FromEnv` path can combine `DOCKER_CERT_PATH` with an empty
  `DOCKER_TLS_VERIFY` to use TLS without verifying the server. Bollard's selected
  rustls connector always verifies. Do not add a permissive certificate
  verifier to imitate that insecure test-only mode. If exact compatibility with
  that mode is declared required, this decision becomes
  `human-decision-required` because parity conflicts with the security gate.

## Known limitations

- The version-prefix defect and mandatory correction above are exact to 0.21.0.
  Any Bollard upgrade must rerun the transcript probe and remove the workaround
  only after upstream emits versioned paths itself.
- Bollard 0.21.0 generates Engine 1.53 types. Negotiation controls server
  behavior, but package tests must exercise the oldest Docker version Ployz
  supports for every field it treats as required.
- Bollard does not declare an MSRV. Rust 1.96 compatibility is established only
  for the exact lock generated during this research; an upgrade needs a new
  toolchain check.
- The selected `ssl` feature uses Ring and therefore a C/assembly build script.
  Native Linux passed. Linux-to-macOS cross-checking failed because this VM has
  no Apple target compiler/SDK; native macOS amd64/arm64 and Linux arm64
  exact-feature/exact-lock build and runtime checks remain required by D03.
- The dependency's default request-header timeout is 120 seconds. It does not
  cover continued streaming after headers arrive, but package code must still
  own every oracle deadline and cancellation boundary.
- SSH Docker hosts, Docker contexts, BuildKit, WebSockets, and Podman discovery
  are not approved by this feature set. Return to the dependency gate before
  enabling their features or advertising those modes.
- Bollard's legacy tar build, container upload/download archive, and image
  import/export APIs are available without the `buildkit` feature. The frozen
  `internal/docker` package and its direct callers do not own Compose builds;
  frozen `internal/cli.BuildServices` delegates those to Docker Compose. This
  record therefore does not authorize substituting Bollard's legacy build API
  for Compose/BuildKit behavior. A later package requiring BuildKit must return
  to the dependency gate instead of selecting the disabled builder variant,
  whose source deliberately reaches `unimplemented!` without the feature.

## Verification command and probe result

The first focused probe used Rust 1.96.0 with the exact dependency line, Tokio
1.53, `futures-util` 0.3, and `http` 1. A local TCP fake Engine returned API
1.41, two pull progress objects, a 404 JSON error, and one push progress object.
It asserted:

- version negotiation selected 1.41;
- unmodified Bollard emitted unversioned paths (the characterization failure);
- the required modifier emitted `/v1.41/...` while preserving encoded pull and
  OCI-platform push queries;
- pull and push streams retained ordered typed messages;
- missing-image response became status 404 with message
  `No such image: missing`;
- unauthenticated push still sent a non-empty encoded `X-Registry-Auth` header.

The follow-up Rust probe used the same exact Bollard line and a fresh 121-crate
lock. Its fake Unix daemon and slow TCP Engines asserted:

- missing socket followed by daemon readiness reconstructs rather than
  retaining `SocketNotFoundError`;
- connection refusal followed by readiness reconstructs and succeeds;
- only socket absence and an inner `is_connect()` legacy error are retryable;
- daemon-start cancellation returns success and 401 remains permanent;
- both pull and push deliver one progress item, then cancellation drops the
  stream, reports the frozen wrapped cancellation, closes output exactly once,
  joins the producer, and causes server EOF/reset within one second.

The matching Go probe used Docker client 28.5.0 and an exact copy of frozen
`processPullPushImageResp`. Pull and push each printed:

```text
first="first" cancel="decode image pull/push message: context canceled" channel_closed=true server_canceled=true
race cancel-before-decoded-error second_open=true error="context canceled" channel_closed=true
race decoded-error-before-cancel second_open=true error="boom" channel_closed=true
race eof-before-cancel second_open=false error="<nil>" channel_closed=true
```

Commands and observed results:

```sh
cargo +1.96.0 run --locked --offline \
  --manifest-path /tmp/ployz-bollard-probe/Cargo.toml
# pass with the mandatory modifier

cargo +1.96.0 clippy --locked --offline --all-targets \
  --manifest-path /tmp/ployz-bollard-probe/Cargo.toml -- -D warnings
# pass

cargo +1.96.0 check --locked --offline --all-targets \
  --manifest-path /tmp/ployz-bollard-probe/Cargo.toml \
  --target x86_64-unknown-linux-gnu
# pass

cargo +1.96.0 check --locked --offline --all-targets \
  --manifest-path /tmp/ployz-bollard-probe/Cargo.toml \
  --target x86_64-apple-darwin
# infrastructure failure in ring's build: no Apple C compiler/SDK on this Linux VM

cargo audit --no-fetch --deny warnings \
  --file /tmp/ployz-bollard-probe/Cargo.lock
# no vulnerabilities or warnings in 121 locked packages including the probe root

go run .
# from /tmp/ployz-docker-go-probe: both pull and push pass the exact cancellation characterization

cargo +1.96.0 run --locked \
  --manifest-path /tmp/ployz-bollard-unblock-probe/Cargo.toml
# D02 and D05 cases pass

cargo +1.96.0 clippy --locked --all-targets \
  --manifest-path /tmp/ployz-bollard-unblock-probe/Cargo.toml -- -D warnings
# pass

cargo audit --no-fetch --deny warnings \
  --file /tmp/ployz-bollard-unblock-probe/Cargo.lock
# no vulnerabilities or warnings in 121 locked packages including the probe root
```

The installed Docker CLI and now-accessible daemon are 29.1.3/API 1.52 with a
minimum API of 1.44. That current Linux amd64 daemon does not establish the
oldest supported floor or close D03/D04, so no current-daemon success is used as
a substitute for those gates. The loopback probes are fully executable and do
not mutate daemon resources.

## Review

This is critical container-control functionality. The prior adversarial review
of `89bddf3` returned **REJECT** on D01-D05. Follow-up executable evidence closes
D02 and D05, leaving D01, D03, and D04 as the exact blocker. A fresh adversarial
review of this revised record is required before integration; a clean review of
the record would validate the narrowed blocked verdict, not approve dependency
use while those three gates remain open.

Affected package: `upstream/uncloud/internal/docker` / future
`crates/ployz-internal-docker`. Direct dependents that informed the decision are
`upstream/uncloud/internal/machine`,
`upstream/uncloud/internal/machine/docker`, and `upstream/uncloud/pkg/client`.
