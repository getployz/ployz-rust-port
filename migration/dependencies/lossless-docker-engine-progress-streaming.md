# Dependency decision: `lossless-docker-engine-progress-streaming`

| Field | Value |
| --- | --- |
| Status | `approved` |
| Capability | Lossless, ordered Docker Engine image pull/push JSON progress streaming |
| Selected dependency | Narrow raw Engine seam using `reqwest = 0.13.4`, `rustls = 0.23.43`, `base64 = 0.22.1`, `futures-util = 0.3.34`, `serde = 1.0.229`, `serde_json = 1.0.151`, and the already selected `tokio = 1.53.1`; Bollard remains the client for all other Engine methods |
| License | Direct crates: `MIT OR Apache-2.0`; `Apache-2.0 OR ISC OR MIT` (Rustls); `MIT` (Tokio). The resolved probe graph contains only permissive/compatible expressions. |
| Research date | `2026-08-11` UTC |
| Request | [`migration/dependencies/requests/lossless-docker-engine-progress-streaming.md`](requests/lossless-docker-engine-progress-streaming.md) |

## Verdict

Approve Reqwest 0.13.4 as a deliberately narrow, pull/push-only Docker Engine
HTTP/JSON seam. It is the most popular maintained candidate that passes every
gate. Its public response body is an ordered raw-byte stream, so a local open
Serde model can retain `id`, `status`, textual `progress`, all required
`progressDetail` members, deprecated textual `error`, the complete
`errorDetail`, and future fields. Reqwest 0.13.4 also provides the three
required connector families directly: Unix sockets, Windows named pipes, and
TCP with verified Rustls/mTLS. No custom Hyper connector is necessary.

This approval is intentionally smaller than a second general Docker client.
Use the seam only for `POST /images/create` and
`POST /images/{validated-name}/push`. It must consume the same canonical daemon
configuration, selected Engine API version, registry-auth value, TLS material,
and security policy as the approved `docker-engine-client` capability. Bollard
0.21.0 remains selected for every other Engine operation. Do not add Ping,
version negotiation, credential discovery, image-name parsing, redirect
following, automatic retries, or another Engine method to this seam without
returning to the dependency gate.

Bollard 0.21.0 cannot implement this capability: its public pull model drops
textual `progress`; its public push model also drops `id`; both methods replace
an embedded `errorDetail` item with a message-only error; and its raw response
and transport are private. An upstream Bollard correction would eventually be
preferable because it could collapse the two maintained clients, but no exact
corrected release or commit exists. Approval cannot be based on a hypothetical
artifact.

## Primary-source evidence

### Oracle and direct callers

- The frozen decoder performs sequential `json.Decoder.Decode`, treats clean
  EOF as success, emits one wrapped decode error on malformed/transport input,
  retains the full decoded message when `errorDetail` is present, checks
  cancellation after each decode, and closes the response and channel exactly
  once: [`internal/docker/image.go`](../../upstream/uncloud/internal/docker/image.go).
  If cancellation is ready after a decode, it emits a bare cancellation error
  instead of the decoded or embedded-error item.
- Moby's `JSONMessage` contains `id`, `status`, textual `progress`,
  `progressDetail`, textual `error`, `errorDetail`, stream metadata, and raw
  auxiliary data. `JSONProgress` includes `current`, `total`, `start`,
  `hidecounts`, and `units`: [Moby 28.5.0 JSON message
  source](https://github.com/moby/moby/blob/v28.5.0/pkg/jsonmessage/jsonmessage.go#L21-L43)
  and [message
  definition](https://github.com/moby/moby/blob/v28.5.0/pkg/jsonmessage/jsonmessage.go#L144-L170).
- Push rendering uses message `id`, status, textual progress, and structured
  current/total values. Pull callers drain messages in order and stop on the
  first item error: [`pkg/client/image.go`](../../upstream/uncloud/pkg/client/image.go),
  [`pkg/client/container.go`](../../upstream/uncloud/pkg/client/container.go),
  and [`internal/docker/container.go`](../../upstream/uncloud/internal/docker/container.go).
- The remote pull bridge forwards each decoded raw JSON object in order before
  its client reconstructs the same message model:
  [`internal/machine/docker/server.go`](../../upstream/uncloud/internal/machine/docker/server.go)
  and [`internal/machine/docker/client.go`](../../upstream/uncloud/internal/machine/docker/client.go).

### Docker Engine and registry authentication

- Docker defines a versioned REST API and directs clients to use the highest
  mutually supported API version: [Engine API
  overview](https://docs.docker.com/reference/api/engine/). The pull and push
  endpoints are long-standing JSON streams; push `platform` requires API 1.46+
  and closing the HTTP connection cancels the push: [Engine v1.47 image
  push](https://docs.docker.com/reference/api/engine/version/v1.47/#tag/Image/operation/ImagePush).
- Docker's first-party raw pull example contains ordered `status`, `id`,
  `progressDetail`, and textual `progress` objects: [SDK/API
  example](https://docs.docker.com/reference/api/engine/sdk/examples/#pull-an-image).
- The Go client validates/normalizes a push reference, sends its name in the
  path, sends tag/platform as query parameters, and always includes
  `X-Registry-Auth`. Push passes `http.NoBody` through its JSON encoder, which
  emits the exact three-byte body `{}\n` with `Content-Type: application/json`;
  pull passes nil and has no body. Pull sends `fromImage`, tag/digest, and
  platform as query parameters: [Moby push client
  source](https://github.com/moby/moby/blob/v28.5.0/client/image_push.go#L25-L81)
  and [pull client
  source](https://github.com/moby/moby/blob/v28.5.0/client/image_pull.go#L20-L73),
  plus [request body
  encoding](https://github.com/moby/moby/blob/v28.5.0/client/request.go#L61-L90).
- Docker auth uses padded URL-safe RFC 4648 Base64, not ordinary Base64:
  [Moby auth
  source](https://github.com/moby/moby/blob/v28.5.0/api/types/registry/authconfig.go#L49-L73).
  The frozen wrapper also requires an encoded empty `{}` auth config for a push
  when no credentials exist:
  [`internal/docker/image.go`](../../upstream/uncloud/internal/docker/image.go).
- Moby's non-GET redirect policy returns `ErrRedirect` when a response has a
  redirect target; a 3xx that does not trigger redirect handling remains in its
  `[200, 400)` accepted status range. For status 400+, `checkResponseErr` reads
  through a 1 MiB limiter and uses exact-content-type JSON versus trimmed-text
  handling before applying status classifications: [redirect
  policy](https://github.com/moby/moby/blob/v28.5.0/client/client.go#L150-L172)
  and [response-error
  mapping](https://github.com/moby/moby/blob/v28.5.0/client/request.go#L191-L275).

### Reqwest 0.13.4

- The exact published manifest declares `rust-version = 1.85.0`,
  `MIT OR Apache-2.0`, and separately gated `query`, `stream`, and
  `rustls-no-provider` features: [0.13.4
  manifest](https://github.com/seanmonstar/reqwest/blob/v0.13.4/Cargo.toml#L1-L74).
- `Response::bytes_stream` exposes body data as a pull-driven
  `Stream<Item = Result<Bytes>>` without imposing a generated response model:
  [response
  source](https://github.com/seanmonstar/reqwest/blob/v0.13.4/src/async_impl/response.rs#L329-L353).
- `ClientBuilder::unix_socket` and `windows_named_pipe` route all connections
  through the selected local endpoint. TCP, verified Rustls roots, PEM root
  bundles, and a combined PEM client identity cover the approved remote
  transport: [local transport
  APIs](https://github.com/seanmonstar/reqwest/blob/v0.13.4/src/async_impl/client.rs#L1798-L1835),
  [TLS builder
  APIs](https://github.com/seanmonstar/reqwest/blob/v0.13.4/src/async_impl/client.rs#L1875-L1953),
  and [PEM
  parsers](https://github.com/seanmonstar/reqwest/blob/v0.13.4/src/tls.rs#L174-L222)
  plus [client-identity
  parser](https://github.com/seanmonstar/reqwest/blob/v0.13.4/src/tls.rs#L351-L376).
- Redirects, retries, proxies, HTTP version, total/read timeouts, and TCP
  options are explicit builder policies. Reqwest warns that side-effecting
  requests must not be retried and supplies `retry::never()`:
  [retry source](https://github.com/seanmonstar/reqwest/blob/v0.13.4/src/retry.rs#L1-L76).
- Tag `v0.13.4` is immutable commit
  `11489b34eda6d32b15ad4033e62beba2ee401350`, released 2026-05-25. Its exact
  GitHub Actions run passed platform-matrix jobs for Linux amd64, Linux arm64,
  macOS arm64, Windows x86_64 MSVC/GNU, Windows i686 MSVC/GNU, and Windows
  arm64 MSVC. Separate Linux feature jobs passed `rustls-no-provider`; the
  platform jobs and providerless-feature jobs were not the same job: [release CI
  run](https://github.com/seanmonstar/reqwest/actions/runs/26411710903).
- Official crates.io data on the research date reported Reqwest 0.13.4 as the
  current non-yanked release, 635,082,483 total downloads, 160,124,401 recent
  downloads, and 27,989 reverse-dependency rows: [crate
  API](https://crates.io/api/v1/crates/reqwest) and [reverse-dependency
  API](https://crates.io/api/v1/crates/reqwest/reverse_dependencies?page=1&per_page=1).

### Why Bollard cannot be adapted at package level

- Bollard 0.21.0's generated `CreateImageInfo` omits textual `progress`; its
  `PushImageInfo` omits both `id` and textual `progress`; and its
  `ProgressDetail` omits `start`, `hidecounts`, and `units`: [pull
  model](https://github.com/fussybeaver/bollard/blob/v0.21.0/codegen/swagger/src/models.rs#L2412-L2429),
  [push
  model](https://github.com/fussybeaver/bollard/blob/v0.21.0/codegen/swagger/src/models.rs#L6251-L6264),
  and [progress
  model](https://github.com/fussybeaver/bollard/blob/v0.21.0/codegen/swagger/src/models.rs#L6229-L6238).
- Both image methods convert an embedded error item to
  `DockerStreamError { error: String }`, discarding the original item and code:
  [pull source](https://github.com/fussybeaver/bollard/blob/v0.21.0/src/image.rs#L120-L151)
  and [push source](https://github.com/fussybeaver/bollard/blob/v0.21.0/src/image.rs#L493-L526).
  The raw decoder, response processing, transport address, and HTTP client are
  crate-private: [decoder](https://github.com/fussybeaver/bollard/blob/v0.21.0/src/read.rs#L125-L191)
  and [client fields](https://github.com/fussybeaver/bollard/blob/v0.21.0/src/docker.rs#L277-L284).
- Bollard serializes structured credentials with ordinary Base64
  `STANDARD`, while Docker specifies URL-safe encoding: [Bollard auth
  source](https://github.com/fussybeaver/bollard/blob/v0.21.0/src/auth.rs#L1-L31).

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior and ordering | Preserve ordered `id`, `status`, textual `progress`, complete required `progressDetail`, textual `error`, and complete `errorDetail`; accept JSON `null` as the zero message; retain unknowns; retain an embedded-error item at its original position whenever non-null `errorDetail` is present | Raw bytes are observable before modeling. The Rust 1.96 split-chunk probe decoded ordered objects, a zero `null` item, and retained every required and injected unknown field, including `start`, `hidecounts`, `units`, and error code/message. It also accepted empty, code-only, and message/code `errorDetail` objects. | `pass` |
| EOF, cancellation, and cleanup | Clean EOF; malformed and transport errors remain distinct; outer connect/header cancellation, wrapped blocked-body-read cancellation, and bare post-decode cancellation remain distinct; close response; no orphan task | `bytes_stream` is pull-driven. The probe observed clean EOF, rejected incomplete final JSON, and proved that dropping either a pending response-header future or a response blocked on its next chunk caused the fake Engine to observe connection EOF within two seconds. The rules below preserve the oracle's three cancellation positions; package code must not spawn a producer task. | `pass` |
| Registry authentication | Preserve the resolver's exact auth string, correct URL-safe alphabet, no-auth pull, and encoded-empty push | Reqwest accepts the exact `X-Registry-Auth` header value without reserializing it. The probe asserted authenticated, empty pull, and URL-safe encoded-`{}` header transmission and distinguished URL-safe `-` from standard `+` for valid UTF-8 credential JSON. `base64::URL_SAFE` supplies padded URL-safe encoding. Credential discovery remains outside this decision. | `pass` |
| Engine API and architecture | Versioned pull/push paths, exact pull/push bodies/query/status/redirect handling; no general duplicate Engine client, retry, proxy, or independent negotiation | Reqwest's URL/query and raw stream APIs cover exactly two endpoints. The probe asserted the full push transcript and a custom POST redirect error while leaving 3xx-without-Location observable. The rules below source Bollard's post-negotiation client version, bound error bodies to 1 MiB, and retain Bollard for every other method. | `pass` |
| Platforms and transports | Linux/macOS amd64/arm64; Unix socket and configured TCP/verified mTLS; retain Windows named-pipe support when compiled | Exact selected features ran over TCP and a real Unix socket on Linux; exact graph checked for Linux arm64 and Windows x86_64 GNU, including the named-pipe builder call. Reqwest's exact tag separately passed its Linux arm64, macOS arm64, Windows matrix, and providerless-feature jobs. Unix-socket code is shared by Linux and both macOS architectures; Rustls/Ring is the same target family already selected with Bollard. Native macOS amd64 remains an integration-release check, not an unsupported dependency target. | `pass` |
| Maintenance and Rust 1.96 | Maintained current release with compatible MSRV and exact-version build | Reqwest 0.13.4 is current, released 2026-05-25, declares Rust 1.85, has leading adoption, and its exact graph ran/check/clippy under Rust 1.96. | `pass` |
| License and security | Apache-2.0-compatible permissive graph; verified TLS; no known advisory in exact graph; bounded privileged surface | The 141 registry packages in the exact probe metadata had only permissive/compatible license expressions. RustSec scanned the 142-dependency lock with 1,211 advisories at advisory-db commit `d0861df1eab469d3c58d6b836ce48b5766e5f217` and found none. Remote TLS must verify hostname/chain; insecure verification is not authorized. | `pass` |

## Candidate comparison

Adoption figures are official crates.io snapshots from 2026-08-11.

| Candidate | Fit and maintenance | Decision |
| --- | --- | --- |
| **Reqwest `=0.13.4` narrow raw stream** | Public raw ordered body stream; built-in Unix socket, Windows named pipe, TCP, verified Rustls/mTLS; exact request policy controls; Rust 1.85 MSRV; 635.1M total / 160.1M recent downloads and 27,989 reverse dependencies. | **Selected.** It is the only maintained, highly adopted facade that passes behavior and all required connectors without bespoke Hyper connector code. |
| Bollard `=0.21.0`, `pipe,ssl` | The established Docker client (44.5M total / 13.7M recent downloads), but released public models irreversibly lose fields and embedded-error detail, auth uses the wrong Base64 alphabet, and raw response/transport internals are private. | Rejected for this capability; remains the selected general Engine client. |
| Upstream-corrected Bollard | A common lossless image item/raw stream plus URL-safe auth would reuse one client and be architecturally best. | Not selectable: no immutable corrected release/commit exists. Re-evaluate on a released correction; do not use a speculative fork or git revision under this approval. |
| Dockworker `=0.17.0` | Pull has a hand-written progress enum, but push consumes/discards its response and returns `Result<()>`; Windows uses TCP rather than named pipes. 261,856 total / 12,474 recent downloads. | Rejected at behavior and Windows transport gates. |
| Docker-api `=0.14.0` | Pull streams generated chunks, but push buffers/discards its response; no matching named-pipe/version policy; last released in 2023. 641,427 total / 64,887 recent downloads. | Rejected at behavior, platform, and maintenance gates. |
| Shiplift `=0.7.0` | Pull exposes raw-ish `serde_json::Value`, but the release has no image push API and was last released in 2021. 930,915 total / 98,171 recent downloads. | Rejected at push behavior and maintenance gates. |
| Direct Hyper 1.11 stack with `hyperlocal` and `hyper-named-pipe` | Can expose raw bytes but requires package-owned connector composition, TLS setup, and more request policy. | Rejected because Reqwest supplies the same low-level stream with maintained connectors and substantially less privileged protocol code. |
| Docker CLI subprocess | Can display progress but changes installation, credential, cancellation, stream, and output behavior. | Rejected at architecture and behavior gates. |

## Selected integration

Use these exact direct dependency lines for this capability. Root workspace
files remain integrator-owned.

```toml
base64 = { version = "=0.22.1", default-features = false, features = ["std"] }
futures-util = { version = "=0.3.34", default-features = false, features = ["std"] }
reqwest = { version = "=0.13.4", default-features = false, features = ["query", "rustls-no-provider", "stream"] }
rustls = { version = "=0.23.43", default-features = false, features = ["ring", "std", "tls12"] }
serde = { version = "=1.0.229", features = ["derive"] }
serde_json = "=1.0.151"
tokio = { version = "=1.53.1", default-features = false, features = ["io-util", "macros", "net", "rt", "sync", "time"] }
```

The crate may consume an integrator-owned superset of these already selected
Tokio/futures/Serde features; this record does not authorize Reqwest defaults.
Do not enable Reqwest default TLS, HTTP/2, compression, cookies, JSON request
helpers, multipart, DNS overrides, SOCKS, system-proxy, or HTTP/3 features.

### Process crypto and client construction

Install Ring once before constructing either Bollard or Reqwest:

```rust
rustls::crypto::ring::default_provider()
    .install_default()
    .map_err(|_| /* typed duplicate/conflicting-provider configuration error */)?;
```

Treat an already installed Ring provider as success only after confirming the
process-level setup is centralized; do not race per-client installs. Both
Bollard's `ssl` feature and Reqwest `rustls-no-provider` use this same Rustls
provider choice.

Construct one narrow Reqwest client from the same parsed daemon configuration
as Bollard. Required common settings are:

- `.http1_only()`, `.no_proxy()`,
  `.retry(reqwest::retry::never())`, and a custom redirect policy whose
  callback returns a typed `unexpected redirect in response` error. Do not use
  `Policy::none()`: it returns a 3xx response instead of reproducing Moby's
  non-GET redirect failure. Map the resulting Reqwest error to the broader
  client's connection-failed/retryable outer class while retaining the typed
  redirect source;
- a 10-second connection deadline matching Docker Go's socket dialer, with no
  total timeout and no read timeout; on Linux call
  `.tcp_user_timeout(None::<Duration>)` to disable Reqwest's default
  `TCP_USER_TIMEOUT` for the long-lived progress body;
- `.unix_socket(path)` for `unix://`; `.windows_named_pipe(path)` for
  `npipe://`; use Moby's synthetic `api.moby.localhost` request authority for
  both local transports. Use HTTPS with hostname/chain verification for an approved
  production TCP configuration. Map a canonical `tcp://host` plus approved TLS
  material to `https://host`; reject a production TCP configuration without
  verified TLS. Plain HTTP is permitted only inside the local fake-Engine test
  harness, not as a production Docker-daemon transport;
- native roots plus every certificate from `ca.pem` via
  `Certificate::from_pem_bundle`/`add_root_certificate`, and combined
  `cert.pem` + `key.pem` via `Identity::from_pem`; and
- no permissive verifier. The broader authority's refusal to reproduce
  `DOCKER_TLS_VERIFY=""` insecure TLS applies equally here.

Do not read environment variables twice. Parse and validate daemon/TLS
configuration once, then construct Bollard and this client from that value.
The two clients have separate pools, but one endpoint, API-version value,
credentials source, TLS policy, and cancellation owner.

### Request and Engine API policy

- Run only after the broader client has applied its authorized eager/manual
  negotiation policy. Read the result from Bollard's public
  `Docker::client_version()` and pass that explicit value into the seam. Do not
  independently Ping or negotiate. Prefix both Reqwest paths with exactly
  `/v{major}.{minor}`. Bollard 0.21.0's URI-join defect can leave its own
  authorized operations unversioned on wire even though this seam sends the
  selected prefix; record that asymmetry as the existing broader client's
  version deviation rather than claiming identical on-wire paths.
- Pull: `POST /v{version}/images/create`; use Reqwest's `.query()` for
  `fromImage`, optional lower-case platform, and tag/digest only when `All` is
  false; omit tag when `All` is true, matching the Moby client. Push:
  `POST /v{version}/images/{name}/push`; use only the name from
  the separately approved OCI-reference parser, preserve its validated `/`
  separators, reject digest references, and use query parameters for tag,
  optional all-tags behavior, and the OCI platform JSON. Return the same
  version error before sending a push platform on API below 1.46.
- Pull has no request body and no JSON content type. Push sends exactly `{}\n`
  with `Content-Type: application/json` and known `Content-Length: 3`. Always
  set `X-Registry-Auth` to the exact caller/resolver value, including an empty
  header on a no-credential pull. Push must send an encoded value; when no
  credentials exist, encode `{}` with
  `base64::engine::general_purpose::URL_SAFE` (padded).
- Never follow or retry these side-effecting POST requests. A 3xx with a usable
  `Location` invokes the custom redirect policy and returns the outer
  connection-failed classification with the typed redirect source. A 2xx or
  3xx without a redirect attempt is accepted, matching Moby's `[200, 400)`
  success range.
- For status 400 or above, do not use `Response::error_for_status()`. Read at
  most 1 MiB, close the response, and reproduce Moby's
  `checkResponseErr`: reaching the bound yields its API-route/version size
  diagnostic; an empty body yields its route/version diagnostic; exact
  `Content-Type: application/json` parses the open `{ "message": ... }`
  response, trims a non-empty message, reports malformed JSON distinctly, and
  uses the status/no-message diagnostic for valid JSON without a message; every
  other content type is trimmed as plain text. Preserve the status-derived
  invalid-parameter/not-found/conflict/system classification used by callers.

### Lossless incremental decoder and lifecycle

Use one local open response model for both endpoints. It must contain optional
`id`, `status`, textual `progress`, `progressDetail`, textual `error`,
`errorDetail`, stream/from/time/aux fields needed by the frozen model, and
`#[serde(flatten)]` maps on the top-level item, `progressDetail`, and
`errorDetail`. Decode each top-level value as `Option<Message>` and map JSON
`null` to the zero/default message, as Go's decoder does for a struct target.
Do not use Bollard's generated image progress types. Decode
consecutive JSON values incrementally across arbitrary byte chunks with
`serde_json::Deserializer`; do not require newlines and do not buffer the whole
operation.

The natural API is one caller-owned async stream/future, not a channel backed
by a detached producer. For each item:

1. poll/read and decode exactly one next JSON value;
2. if the body cleanly ends with only JSON whitespace buffered, finish without
   an item; if non-whitespace remains or the body/decoder fails, return one
   wrapped decode/transport error and finish;
3. build the item, retaining its complete message. Any non-null `errorDetail`
   object makes the item an embedded error, even `{}` or a code-only object;
   its error text is the optional message or the empty string, matching
   `errors.New(jm.Error.Message)`;
4. immediately after decode, check cancellation before yielding; cancellation
   wins and yields a bare cancellation error, suppressing the decoded item as
   in the oracle; otherwise yield the item; and
5. yield an embedded-error item at its decoded position. Do not decode ahead.
   The frozen direct callers stop and drop the stream on that item; if a
   different consumer deliberately polls again, decoding may continue just as
   the frozen producer loop would.

Cancellation has three distinct observable positions. Cancellation while
connecting, sending, or awaiting response headers returns the outer operation
error before a stream exists. Cancellation while the decoder is blocked on a
body read drops the body and yields one stream item wrapping cancellation as
`decode image pull/push message: ...`, because the oracle observes it as a
decode/read failure. Cancellation observed immediately after a successful
decode yields a bare cancellation item and suppresses that decoded message.
Whichever read/decode or cancellation outcome wins a true race determines the
position, as in the oracle. In all cases drop the Reqwest future/body/response
before returning. Dropping the consumer must also drop the body. Do not spawn a
forwarding task; if a later crate API makes one unavoidable, return to review
with an owned join handle and prove closure/join on EOF, error, cancellation,
and consumer drop.

### Known deviations and limits

- Two maintained HTTP clients/pools connect to the same privileged daemon.
  This is accepted only because Bollard's raw stream and connector are private
  and Reqwest contains the connector/security implementation. The duplication
  is bounded to two endpoints and one canonical configuration. Remove the seam
  after a reviewed Bollard release exposes lossless raw image streams and
  URL-safe auth.
- Reqwest's Windows named-pipe API compiled in the exact selected graph and
  the exact release passed Windows CI, but no Windows daemon runtime probe ran
  on this Linux host. Windows is not in the frozen shipped release matrix;
  named-pipe support is retained rather than newly claimed as a shipped target.
- The exact selected graph ran natively on Linux amd64 and checked for Linux
  arm64 and Windows x86_64 GNU. Exact-tag upstream CI supplies macOS arm64 and
  broader Windows build/test evidence. The local macOS amd64 cross-check reached
  Ring's C compilation but could not run without an Apple SDK; this is an
  environment gap, not evidence of target rejection. Native shipped-target
  integration remains required before release, including macOS amd64/arm64
  Unix-socket operation and configured TLS where those modes are shipped.
- Separate-pool connection timing may differ from Bollard after clean EOF. No
  progress values, ordering, errors, auth, security decision, or cancellation
  outcome may differ. A new observable limitation returns to the dependency
  gate; it is not implicitly accepted here.

## Verification commands and probe result

The bounded probe is intentionally outside the repository because this role
owns only the decision record. On 2026-08-11 it used the exact dependency lines
and lock at `/tmp/ployz-reqwest-progress-probe-e455ce01`, including Tokio's
exact `rt` feature and a current-thread runtime. A fake chunked Engine split
JSON values across HTTP chunks and asserted pull query encoding, exact
authenticated/empty/encoded-`{}` auth header transfer, ordered required and
unknown fields, JSON `null`, three `errorDetail` shapes, clean EOF, a valid
final item without a newline, incomplete-final-value rejection, and connection
EOF after dropping a pending response-header future or blocked response body.
A push fake asserted the versioned name path, tag/platform query, encoded auth,
`Content-Type`, `Content-Length: 3`, and exact `{}\n` body. Redirect fakes
proved a custom error for 307 plus `Location` and an accepted 307 without
`Location`. A second fake Engine ran through a real Unix socket. The probe also
asserted padded URL-safe auth alphabet behavior and referenced the Windows
named-pipe builder under its target cfg.

```sh
cargo +1.96.0 generate-lockfile \
  --manifest-path /tmp/ployz-reqwest-progress-probe-e455ce01/Cargo.toml
cargo +1.96.0 fmt \
  --manifest-path /tmp/ployz-reqwest-progress-probe-e455ce01/Cargo.toml -- --check
cargo +1.96.0 run --locked \
  --manifest-path /tmp/ployz-reqwest-progress-probe-e455ce01/Cargo.toml
cargo +1.96.0 clippy --locked --all-targets --all-features \
  --manifest-path /tmp/ployz-reqwest-progress-probe-e455ce01/Cargo.toml -- -D warnings
cargo +1.96.0 check --locked --target aarch64-unknown-linux-gnu \
  --manifest-path /tmp/ployz-reqwest-progress-probe-e455ce01/Cargo.toml
cargo +1.96.0 check --locked --target x86_64-pc-windows-gnu \
  --manifest-path /tmp/ployz-reqwest-progress-probe-e455ce01/Cargo.toml
cargo audit --file /tmp/ployz-reqwest-progress-probe-e455ce01/Cargo.lock
```

Observed run output:

```text
confirmed: lossless stream, pull/push/auth transcripts, redirects, EOF, and blocked drops
```

The package implementation must convert this exploratory coverage into durable
tests for both pull and push. Include split/multiple/final-without-newline JSON,
JSON `null`, all known plus unknown fields, malformed and transport failures,
embedded error retained at its exact position followed by direct-caller stream drop,
clean EOF, all three cancellation positions and their race, consumer drop,
connection closure, exact URL-safe authenticated/empty-pull/empty-object-push
headers, pull `All`, the complete push transcript, redirect-with/without-
Location and retry classification, JSON/plain/empty/malformed/at-and-over-limit
status bodies, Unix socket,
configured TLS, Engine API version/query handling, and target compilation.
Run the same Rust 1.96 format/check/test/clippy/audit/license checks against the
integrated lock.

## License and security notes

- Direct licenses are: Base64, Futures-util, Reqwest, Serde, and Serde JSON
  `MIT OR Apache-2.0`; Rustls `Apache-2.0 OR ISC OR MIT`; Tokio `MIT`.
  `cargo metadata` enumerated 141 registry packages in the exact normal probe
  graph; all expressions were permissive/compatible (MIT, Apache-2.0, ISC,
  BSD-3-Clause, Unicode-3.0, CDLA-Permissive-2.0, BSL-1.0, Unlicense, and
  compatible combinations/exceptions).
- `cargo audit` found no advisory or warning in the exact 142-dependency lock
  against 1,211 RustSec advisories at advisory-db commit
  `d0861df1eab469d3c58d6b836ce48b5766e5f217` dated 2026-08-11. Re-run against
  the integrated lock; this snapshot is not a future waiver.
- Docker daemon access is host-root-equivalent. Docker warns against exposing
  an unauthenticated daemon: [daemon attack
  surface](https://docs.docker.com/engine/security/#docker-daemon-attack-surface).
  Default to the permission-protected local socket; remote TCP is explicit
  operator configuration and requires verified TLS under the existing
  authority. Never log auth headers or credential JSON.

## Review

Fresh read-only adversarial reviewer `/root/adversarial_review` initially
returned **not clean** with blocking findings P01-P06 and R09, plus platform and
package-test precision notes. It found no usable Bollard public raw-response
seam and considered Reqwest viable after correction.

All findings were accepted and fixed:

- P01: split pull's absent body from push's exact JSON `{}\n` transcript and
  added a push transcript probe;
- P02: separated outer connect/header cancellation, wrapped blocked-body-read
  cancellation, and bare post-decode cancellation;
- P03: keyed embedded errors on non-null `errorDetail` presence, including
  empty and code-only objects;
- P04: rejected production plaintext TCP and required verified HTTPS for the
  canonical TCP endpoint;
- P05: replaced `Policy::none()` with a custom redirect error, specified the
  `[200, 400)` success range and bounded/content-sensitive Moby error mapping,
  and probed redirects with and without `Location`;
- P06: changed the probe to the selected Tokio `rt` feature/current-thread
  runtime and reran the exact graph; and
- R09: sourced the explicit seam version from post-policy
  `Docker::client_version()` and documented Bollard's on-wire unversioned
  asymmetry.

The same reviewer rechecked every finding against the current file and reran
the Rust 1.96 probe, warnings-denied Clippy, Linux arm64 check, Windows GNU
check, RustSec audit, and package-count metadata. It reported **CLEAN**: every
finding is closed, Reqwest 0.13.4 remains the popular idiomatic passing choice,
no public Bollard lossless/raw seam was overlooked, and no actionable finding
remains.

Affected package: `upstream/uncloud/internal/docker` / future
`crates/ployz-internal-docker`. Direct consumers used as behavioral evidence
are `upstream/uncloud/internal/machine/docker`, `upstream/uncloud/pkg/client`,
and `upstream/uncloud/internal/docker/container.go`.
