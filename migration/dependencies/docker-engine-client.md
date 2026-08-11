# Dependency decision: `docker-engine-client`

| Field | Value |
| --- | --- |
| Status | `blocked` |
| Capability | Async Docker Engine API client for local and configured daemons, including typed container/image operations, API-version compatibility, streaming pull/push responses, cancellation, and daemon error classification |
| Provisional selected dependency | `bollard = { version = "=0.21.0", default-features = false, features = ["pipe", "ssl"] }` |
| License | `Apache-2.0` |
| Research date | `2026-08-11` UTC |
| Request | No request file was present; capability was delegated directly for `upstream/uncloud/internal/docker` |
| Exact blocker | Approval is blocked on D01-D06: explicit authority for plain TCP/unverified TLS; cancellable lazy construction plus the frozen broad connection-failure retry class and backoff; native exact-lock evidence on shipped macOS amd64/arm64 and Linux arm64; a declared Engine/API floor with the complete caller-operation and exact negotiation matrix; durable typed pull/push cancellation and race proofs; and resolution of Bollard's unconditional request-header timeout versus the frozen context-only timeout policy. The current `/tmp` probes are useful exploratory evidence but do not close a gate. |

## Verdict

`bollard` 0.21.0 remains the clear idiomatic and adoption leader and appears to
cover the broad required Engine API surface without local protocol
reimplementation. It remains the provisional selection, not an approved
dependency. Fresh adversarial review rejected the attempted D02/D05 closures:
the exploratory probes do not cover in-flight ping cancellation, the frozen
broad retry classifier, typed stream-error forwarding and ordered races, and
their source/locks are not durable. D01-D06 below are the exact open gates.

The initial probe found an additional candidate defect: Bollard's documented
`negotiate_version` mutates its stored version, but its 0.21.0 URI builder emits
unversioned operation paths. A public request modifier can add a version prefix,
but the attempted policy was itself non-parity: Bollard negotiates with GET
`/version` and pings with GET, whereas the frozen client negotiates lazily via
unversioned HEAD `/_ping`, falls back to GET, reads the API version header, and
uses API 1.24 when that header is absent. D04 must resolve the complete request
transcript and fallback semantics before any modifier policy becomes mandatory.

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
  implementing an HTTP client, but is not sufficient to reproduce lazy
  HEAD-ping negotiation, direct unversioned Ping, or header-timeout policy.
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
| Behavior | Complete caller surface; exact errors/retries; streaming pull/push; cancellation; API negotiation and timeout compatibility | Public APIs appear to cover most operations, and exploratory probes establish some feasibility, but do not prove parity. Missing evidence includes in-flight ping cancellation, the frozen broad connection-failure class, typed stream errors and ordered cancellation races, complete volume/archive/exec/container/image/network/event/log behavior at the support floor, lazy HEAD-ping negotiation and malformed override behavior, and context-only header waits. | **`blocked`: D02, D04, D05, D06** |
| License and security | Apache-2.0-compatible permissive graph; no known vulnerability; safe local socket and verified mutual-TLS modes | Direct license is Apache-2.0. All 120 external packages in the exploratory all-target probe graph used Apache/MIT/ISC/BSD/Unicode/Unlicense/Zlib-family compatible terms. RustSec found no vulnerability or warning in that ephemeral exact lock. No package-owned `unsafe` or raw socket FFI is needed; Ring and platform certificate/socket crates encapsulate their native/unsafe internals. Docker warns that daemon access is effectively host-root access, so endpoints remain trusted configuration. The frozen environment behavior can select plain TCP or unverified TLS, while this record intentionally permits only local sockets or verified TLS. | **`blocked`: D01 requires human authority** |
| Platforms and targets | Shipped Linux/macOS amd64/arm64, Unix-socket local daemon, configured TCP/mTLS | An exploratory Linux x86_64 Rust 1.96 run/check passed. `pipe`'s Unix connector is target-gated portable Tokio/hyperlocal code; `ssl` uses rustls and native roots. The Linux-to-Intel-macOS check reached Ring's C compilation and failed because no Apple target C compiler/SDK is installed. Bollard has first-party Linux/Windows transport integration CI, but neither cross-compilation nor upstream CI proves the shipped native targets under this exact feature set and lock. | **`blocked`: D03** |
| Maintenance and Rust version | Active current release compatible with workspace Rust 1.96 | 0.21.0 was current and non-yanked; repository activity continued on the research date. The crate declares no `rust-version`, but the exploratory exact graph built, ran, and passed warnings-denied Clippy with Rust 1.96.0. | `pass` |
| Architectural constraints | Idiomatic async API; no subprocess or bespoke Docker REST implementation; bounded build/runtime surface | Bollard uses Tokio/futures streams and generated Engine models directly, with 120 external packages in the exploratory exact lock. However its public GET-only Ping/version negotiation and mandatory header timeout may require a new low-level transport or upstream change to meet D04/D06; that feasibility is unresolved. | **`blocked`: D04, D06** |
| Critical review | Fresh adversarial review for container control | Fresh review of `024193d` reran the exploratory probes successfully but rejected their gate-closing claims and found the D04 matrix/version policy and request timeout incomplete. This record incorporates those findings and requires re-review as a truthful blocked decision. | `pending re-review` |

## Candidate comparison

Counts are official crates.io snapshots from 2026-08-11.

| Candidate | Hard-gate fit | Maintenance and adoption | Decision |
| --- | --- | --- | --- |
| [`bollard` 0.21.0](https://crates.io/crates/bollard/0.21.0) | Broad typed async Engine surface, streams, error status, Unix/named-pipe/HTTP/mTLS transports, platform push, and public request modification. Requires a D04-approved version and negotiation policy; the current modifier is exploratory only. | 44.4M total / 13.6M recent downloads; 287 reverse dependents; 48 releases; current release 2026-05-04; active repository. | **Provisional selection:** overwhelmingly most popular idiomatic candidate, with unresolved hard gates. |
| [`dockworker` 0.17.0](https://crates.io/crates/dockworker/0.17.0) | Broad async CRUD and current maintenance, but its [source](https://github.com/Idein/dockworker/blob/v0.17.0/src/docker.rs) explicitly defaults Windows to TCP because named pipes are unsupported, has no negotiated/versioned request policy, and `push_image` returns only `Result<()>` rather than the required progress stream. | 261,831 total / 12,449 recent downloads; 4 reverse dependents; released 2026-05-18; active repository. | Rejected at behavior/platform gates and far less adopted. |
| [`docker-api` 0.14.0](https://crates.io/crates/docker-api/0.14.0) | Its [source](https://github.com/vv9k/docker-api-rs/blob/v0.14.0/src/docker.rs) models API 1.42 and supports Unix/TCP plus optional OpenSSL TLS, but not named pipes or the oracle's negotiated client behavior. It is tied to Hyper 0.14 and an older generated schema. | 641,396 total / 64,856 recent downloads; 13 reverse dependents; last release 2023-06-05 and last repository push 2024-05-24. | Rejected at active-maintenance and behavior gates. |
| [`shiplift` 0.7.0](https://crates.io/crates/shiplift/0.7.0) | Async Unix/TCP/TLS client, but an old manually maintained API surface, OpenSSL default, no current Engine version negotiation, and no named-pipe support. | 930,779 total / 98,035 recent downloads; 9 reverse dependents; last release 2021-02-21 and last push 2023-10-02. | Rejected at active-maintenance and behavior gates. |
| [`bollard-next` 0.18.1](https://crates.io/crates/bollard-next/0.18.1) | A temporary alternate publication of the same design, pinned to older Engine 1.45 stubs and lacking current upstream improvements. | 99,923 total / 2,819 recent downloads; 7 reverse dependents; last release 2024-10-19 versus active mainline Bollard 0.21.0. | Rejected: obsolete, much less adopted duplicate of the selected project. |
| Direct Docker REST implementation | Could reproduce only the currently used endpoints. | No ecosystem maintenance, schema generation, cross-platform connector, or security review; would duplicate status, streaming, versioning, TLS, and transport logic already maintained by Bollard. | Rejected by architectural gate. |

## Adversarial gates

D01-D06 are blocking evidence requirements. The exploratory probes under
`/tmp` are not tracked and cannot survive a restart; they are reported only to
avoid repeating disproven approaches. Gate-closing probes, exact lockfiles, and
fixtures must be committed under a controller-authorized durable research path.

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

### D02 — Cancellable lazy construction and exact daemon-start retry class

Bollard 0.21.0's Unix constructor checks that the socket path exists and can
return `SocketNotFoundError` before any Engine request. Retaining validated
connection configuration and reconstructing `Docker` on each readiness attempt
is feasible, and the exploratory probe passed missing-socket and
connection-refused-then-ready cases. It does **not** close this gate:

- `ping_with_reconstruction` awaits `docker.ping()` outside its cancellation
  select, so a daemon that accepts and stalls response headers delays
  cancellation until Bollard's request timeout instead of cancelling the
  in-flight ping through the frozen context;
- it retries only `SocketNotFoundError` and `HyperLegacyError::is_connect()`,
  while Docker 28.5 wraps essentially every non-context `http.Client.Do` error
  as connection-failed, including permission/dial/refusal, DNS/network errors,
  timeouts, malformed HTTP, TLS bad-certificate diagnostics, redirects rejected
  by the non-GET redirect policy, and remaining transport errors; and
- its deterministic scheduler does not reproduce the frozen randomized
  exponential backoff.

Durable Go-vs-Rust tests must cover missing and permission-denied sockets,
refusal, DNS/network failure, stalled and elapsed timeouts, malformed HTTP,
TLS-certificate failure, non-GET redirect rejection, arbitrary transport error,
context cancellation/deadline, HTTP 401/404/500, and success. They must select
cancellation against the in-flight Bollard ping and prove the frozen result that
outer cancellation makes `WaitDaemonReady` return success. They must also prove
the exact 100 ms initial interval, 1.5 multiplier, 0.5 randomization factor,
1 s cap, no maximum elapsed time, and no busy loop. That scheduling needs a
separately approved randomized-backoff capability; it must not use the
deterministic exploratory scheduler.

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
version that Ployz supports. The matrix at that floor must exercise every actual
caller operation, not a representative subset:

- ping and server version/info;
- container create, inspect, start, stop, restart, list, remove, logs, and port
  binding data;
- image pull, push, inspect, tag, list with manifests, and remove, including
  registry auth, encoded empty push auth, stream errors, and OCI platform;
- network create, inspect, and remove with IPAM, labels, options, and attached
  container data;
- volume create, list, inspect, and remove;
- event streams and raw-TTY versus multiplexed log streams;
- `CopyToContainer` archive upload; and
- exec create, start, attach/hijack, resize, and inspect, including cancellation
  and exit status.

Each case must cover query/body/model conversion, unknown response fields,
status classification, exact caller-visible context, and cancellation. Legacy
tar build, image import/export, and archive download need capability probes but
are not substitutes for frozen Compose/BuildKit behavior.

The same durable transcript must characterize the frozen version policy before
any request modifier is accepted. Docker 28.5 negotiates lazily on the first
normal request using unversioned HEAD `/_ping`, falls back to GET when HEAD is
not supported, reads the `Api-Version` header, and falls back to API 1.24 when
the header is absent. Every direct Ping remains unversioned. A non-empty
`DOCKER_API_VERSION` strips an optional leading `v`, accepts any remaining
string without `MAJOR.MINOR` validation, and disables negotiation; malformed
values affect later request-path or server behavior rather than failing during
client construction. Bollard instead negotiates with
GET `/version`, requires a parseable response field, and its Ping is GET-only.
The exploratory modifier also incorrectly versioned Ping. Its handling of
redirects differs: the Go non-GET redirect policy returns an error which becomes
connection-failed/retryable, while Bollard does not follow redirects and treats
3xx as a permanent server response error.

The installed 29.1.3/API 1.52 daemon (minimum 1.44), Docker's published API
matrix, and a fake API 1.41 response do not declare Ployz's floor or resolve
these observable differences. Exact negotiation may require a new low-level
transport capability; accepting a different transcript requires explicit
authority.

### D05 — Durable typed pull/push cancellation and ordering parity

The exploratory fake-Engine probes establish only that dropping a pending
Bollard pull/push stream closes the TCP connection and can terminate one simple
forwarder. They do **not** close this gate:

- the Rust forwarder discards `Some(Err(_))`, so it never proves
  `DockerStreamError` propagation;
- it models cancellation as a string constant rather than a typed error with a
  cancellation source;
- it has no Rust cases matching the Go blocked-read, decoded-error,
  send-already-pending, and EOF orderings; and
- it lacks the post-decode cancellation check needed to suppress a decoded item
  when cancellation becomes ready before its send, as the frozen loop does.

Durable Go and Rust probes must prove, for both pull and push: connection
EOF/reset after blocked-read cancellation; exact wrapped decode cancellation;
typed embedded stream errors; bare cancellation winning after decode but before
send; an already-pending unbuffered send retaining its decoded error; EOF before
cancel closing without an item; channel closure exactly once; no later progress;
and producer join/no orphan task. The probe sources, fixtures, manifests, exact
locks, and output assertions must be committed under a controller-authorized
research path before this gate can close.

### D06 — Request-header timeout parity

The frozen Docker client's default `http.Client` has no whole-request timeout;
every request waits according to its supplied context. Bollard wraps every
request-to-response-headers future in `tokio::time::timeout` and defaults to
120 seconds. That adds an error and deadline to background-context requests and
interacts with daemon-readiness cancellation.

Declare a parity-safe constructor timeout policy and prove it with durable
stalled-header tests for background context, finite deadline before and after
the Bollard timeout, cancellation races, and a response arriving near the
boundary. Passing a merely large finite timeout is not exact. If Bollard cannot
disable its timeout, the candidate needs an upstream change, alternate
transport, or explicit authority to accept the new limit.

## Selected integration

If D01-D06 are closed and a fresh re-review accepts the evidence, use exactly:

```toml
bollard = { version = "=0.21.0", default-features = false, features = ["pipe", "ssl"] }
```

The integrator may expose shared versions of Tokio, `futures-util`, and `http`
already required by the natural async implementation; this decision does not
authorize edits to the root manifest. Do not add `bollard-stubs` directly:
Bollard re-exports its generated `models` and `query_parameters`.

### Provisional API-version feasibility, not an approved policy

The exploratory probe proved that `with_request_modifier` can repair Bollard's
unversioned normal-operation paths while preserving scheme, authority, path,
and query. Its compact shape was:

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
`expect` calls shown in this compact probe excerpt. More importantly, this hook
does not solve D04 by itself. A global modifier must never version direct Ping,
and Bollard exposes neither the frozen HEAD-then-GET Ping behavior nor its
header-based lazy negotiation through the natural `ping()`/`negotiate_version()`
APIs. Do not implement the exploratory upfront GET `/version` policy, eagerly
validate `DOCKER_API_VERSION`, or install this modifier in production until D04
has a clean reviewed transcript and any required transport capability is
approved.

### Natural API and cancellation policy

- If D04 and D06 close, use `Docker` directly for Engine operations inside an
  idiomatic Rust wrapper. Do not recreate the Go client's embedded API or method
  names. Any narrow low-level negotiation primitive requires separate approval.
- Consume `create_image` and `push_image` as `Stream<Item = Result<...>>` and
  stop at the first error. Map the generated status/id/progress/current/total
  fields to Ployz progress values. Bollard already promotes
  `errorDetail.message` to `DockerStreamError`; do not decode the byte stream a
  second time.
- Cancellation drops the in-flight future/stream and its response body. Any
  task that forwards progress must select on its cancellation token and drop
  the stream before reporting cancellation. Never detach a pull/push task. The
  exact post-decode/send/EOF ordering remains gated by D05; do not infer it from
  the simple exploratory forwarder.
- Match `DockerResponseServerError { status_code: 404, .. }` for the oracle's
  missing-image retry and other not-found branches. Do not use only
  `HyperLegacyError::is_connect()` for daemon-start retry: D02 must reproduce
  Docker Go's broader connection-failed classification, including some TLS,
  malformed-response, redirect, and timeout errors that a cleaner Rust policy
  would otherwise make permanent.
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
  project's Apache-2.0 license. The exploratory all-target probe graph had no
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

- The version-prefix defect is exact to 0.21.0. Any eventual D04-approved
  correction must be rechecked on a Bollard upgrade and removed only after
  upstream emits versioned paths itself.
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
- The dependency's mandatory request-header timeout is not an accepted known
  limitation. D06 must resolve its divergence from the frozen context-only
  policy before approval.
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
- the exploratory modifier emitted `/v1.41/...` while preserving encoded pull
  and OCI-platform push queries, but incorrectly versioned Ping;
- pull and push streams retained ordered typed messages;
- missing-image response became status 404 with message
  `No such image: missing`;
- unauthenticated push still sent a non-empty encoded `X-Registry-Auth` header.

The follow-up Rust probe used the same exact Bollard line and a fresh 121-crate
lock. Its fake Unix daemon and slow TCP Engines demonstrated only:

- missing socket followed by daemon readiness reconstructs rather than
  retaining `SocketNotFoundError`;
- connection refusal followed by readiness reconstructs and succeeds;
- the exploratory classifier chose only socket absence and an inner
  `is_connect()` legacy error, which is too narrow for parity;
- retry-sleep cancellation returns success and 401 remains permanent, without
  testing cancellation of an in-flight stalled ping;
- both pull and push deliver one progress item, then cancellation drops the
  stream, closes output once, joins the producer, and causes server EOF/reset
  within one second. The forwarder discards stream errors and is not a parity
  implementation.

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
# exploratory modifier probe passes; D04 remains open

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
# exploratory subset passes; D02 and D05 remain open

cargo +1.96.0 clippy --locked --all-targets \
  --manifest-path /tmp/ployz-bollard-unblock-probe/Cargo.toml -- -D warnings
# pass

cargo audit --no-fetch --deny warnings \
  --file /tmp/ployz-bollard-unblock-probe/Cargo.lock
# no vulnerabilities or warnings in 121 locked packages including the probe root
```

None of the `/tmp` probe trees above is durable repository evidence. At review
time their SHA-256 hashes were:

```text
Rust Cargo.toml 140101b0…  Cargo.lock a8e3020f…  src/main.rs 342ec58a…
Go   go.mod     a2a31fc3…  go.sum     4dc7d1d…  main.go     e23ce319…
```

The abbreviated hashes identify the reviewed ephemeral files but are not enough
to reconstruct them and therefore cannot close a gate. The assigned dependency
researcher may commit only this decision record. The controller must authorize
a durable research-artifact owner/path and commit complete probe sources,
fixtures, manifests, sums, exact locks, and assertions before relying on any of
these results after restart.

The installed Docker CLI and now-accessible daemon are 29.1.3/API 1.52 with a
minimum API of 1.44. That current Linux amd64 daemon does not establish the
oldest supported floor or close D03/D04, so no current-daemon success is used as
a substitute for those gates. The exploratory loopback probes did not mutate
daemon resources.

## Review

This is critical container-control functionality. The prior adversarial review
of `89bddf3` rejected D01-D05. Fresh review of `024193d` reran all exploratory
checks successfully but rejected D02/D05 closure, expanded D04, and identified
D06. This corrected record intentionally remains blocked on D01-D06. A clean
re-review validates only that the record states the smallest known falsifiable
blockers; it does not authorize Bollard use.

Affected package: `upstream/uncloud/internal/docker` / future
`crates/ployz-internal-docker`. Direct dependents that informed the decision are
`upstream/uncloud/internal/machine`,
`upstream/uncloud/internal/machine/docker`, and `upstream/uncloud/pkg/client`.
