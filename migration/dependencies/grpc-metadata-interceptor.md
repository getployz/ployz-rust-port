# Dependency decision: `grpc-metadata-interceptor`

| Field | Value |
| --- | --- |
| Status | `approved` |
| Capability | gRPC version metadata, conditional server policy, response header inspection, and unary/streaming interception for `internal/grpcversion` |
| Selected dependency | **No standalone interceptor crate.** Reuse approved `tonic 0.14.6` and direct `tower 0.5.3`; reuse approved `semver 1.0.28` through the exact compatibility boundary in `semantic-version-parsing`. |
| License | `MIT` for Tonic/Tower; `MIT OR Apache-2.0` for SemVer |
| Research date | `2026-08-11` UTC |
| Request | Controller delegation for `grpc-metadata-interceptor`; no request file exists |
| Blockers | None for this capability |

## Decision and dependency composition

Do not add an interceptor helper crate. Tonic owns the gRPC-aware
`MetadataMap`, ASCII/binary key and value types, `Request`, `Response`, and
`Status`; Tower is Tonic's native response-aware middleware model. A separate
metadata/interceptor crate would wrap the selected stack without closing any
behavior gap.

Both prerequisites that blocked the earlier conditional record are now
approved:

- [`async-grpc-client-server`](async-grpc-client-server.md) selects Tonic,
  Tower, Tokio, and the public codec/HTTP service seam and proves the relevant
  response, status, deadline, cancellation, and custom-transport behavior.
- [`semantic-version-parsing`](semantic-version-parsing.md) selects SemVer and
  proves the exact Masterminds v1.5.0-compatible parser/formatter/comparator
  policy, including the frozen dependency's malformed-prerelease comparison
  flaw.

Use these exact direct declarations where their surfaces are used:

```toml
[dependencies]
bytes = "=1.12.1"
http = "=1.5.0"
hyper-util = { version = "=0.1.20", features = ["tokio"] }
semver = { version = "=1.0.28", default-features = false, features = ["std"] }
tokio = { version = "=1.53.1", features = ["io-util", "macros", "net", "rt-multi-thread", "sync", "time"] }
tokio-stream = { version = "=0.1.19", default-features = false, features = ["net"] }
tonic = { version = "=0.14.6", default-features = false, features = ["codegen", "router", "transport"] }
tower = { version = "=0.5.3", default-features = false, features = ["util"] }

[build-dependencies]
tonic-build = { version = "=0.14.6", default-features = false, features = ["transport"] }
```

`internal/grpcversion` itself normally needs only `tonic`, `tower`, `semver`,
and the existing application/runtime types it actually names. The larger block
is the approved shared gRPC graph used by the executable integration probes;
implementors must not add unused support crates merely to repeat the probe.
Only the integrator edits workspace manifests and the lockfile.

## Oracle contract and Ployz rename

The immutable oracle is
[`interceptor.go`](../../upstream/uncloud/internal/grpcversion/interceptor.go)
with
[`interceptor_test.go`](../../upstream/uncloud/internal/grpcversion/interceptor_test.go).
The Rust port preserves its behavior but applies the project-wide product-name
substitution:

| Oracle spelling | Ployz wire/user spelling |
| --- | --- |
| `uncloud-client-version` | `ployz-client-version` |
| `uncloud-min-server-version` | `ployz-min-server-version` |
| `uncloud-server-version` | `ployz-server-version` |
| `https://github.com/psviderski/uncloud/releases/latest` | `https://github.com/getployz/ployz/releases/latest` |

No compatibility aliases are added: accepting or emitting both namespaces
would be a new feature and would change the first-duplicate limitation. The
minimums remain independently fixed at `0.20.0`.

| Area | Required observable behavior | Selected API/design |
| --- | --- | --- |
| Client request metadata | Append current client and minimum server versions on every unary and streaming call. Existing caller values remain first, so a caller can override the injected value. Preserve unrelated ASCII and binary metadata. | `MetadataMap::append`, never `insert`, for both request keys. Tonic's `get` reads the first value and `get_all` preserves order. |
| Parsing | Missing, empty, non-text, malformed, or overflowing first values become `0.0.0`; ignore later duplicates. Preserve permissive parse/format/comparison behavior. | First-value `get` plus fallible `to_str`, then the exact approved package-local SemVer compatibility boundary. |
| Server validation order | Check client version first. Only if it passes, check the client's required minimum server. Reject before the handler with `FailedPrecondition` and the exact renamed-oracle message. | Synchronous policy before the raw-codec proxy handler. `Status::failed_precondition`; do not poll the inner service on rejection. |
| Server response header | A policy rejection has no server-version header. Every accepted request gets one package-owned initial server-version value before any same-key value staged by the handler, including on a handler status/error response. | One composite response-aware proxy service: validate outside, delegate through Tonic's codec service, then prepend the fixed value while retaining delegated duplicates in order. |
| Unary client warning | Inspect the server header only after `Ok(Response<_>)`. Missing/invalid/below-minimum maps to zero and emits the exact renamed warning once process-wide. Status/transport errors never warn. | Inspect Tonic response metadata after successful decode; gate output with a process-wide `AtomicBool`. |
| Streaming client warning | Stream creation and message receipt do not inspect or warn. Only an explicit successful header-equivalent call inspects metadata. Header failure does not warn. Repeated successful header calls still warn at most once process-wide. | Natural `VersionedStreaming<T>` wrapper retaining the initial `MetadataMap` and exposing an explicit `header()` method; `message()` delegates untouched. |
| Status/details | Preserve delegated code, message, opaque binary details, and status metadata. Policy errors use code 9 and the exact policy text. | Return/encode the original `Status`; the outer accepted-response header mutation must not rebuild it. |
| Deadline/cancellation | Metadata work is synchronous and preserves the existing deadline. Dropping/canceling a call or stream cancels the underlying gRPC operation; no middleware task survives it. | Do not spawn, buffer, retry, or wrap the body for metadata policy. Delegate the inner future/body directly. Whole-stream deadline ownership remains the approved async-gRPC adapter contract. |
| Platforms | No metadata policy varies by OS. | Safe, pure-Rust platform-neutral APIs; transport differences remain in the approved async-gRPC decision. |

## Exact attachment contract

The frozen direct-caller audit finds exactly five client channel constructors:

- `pkg/client/connector/tcp.go`
- `pkg/client/connector/unix.go`
- `pkg/client/connector/ssh.go`
- `pkg/client/connector/sshcli.go`
- `pkg/client/connector/wireguard.go`

Each attaches both unary and streaming client interceptors. The Rust channel
construction path must attach the Ployz request policy to every corresponding
transport; connector-specific retry/transport decisions remain separate.

The server policy is deliberately conditional. In
[`machine.go`](../../upstream/uncloud/internal/machine/machine.go), only the two
unknown-service transparent proxy frontends attach both server interceptors:
the local proxy built during machine creation and the cluster proxy built after
initialization. `newGRPCServer` constructs the ordinary four-service Machine,
Cluster, Docker, and Caddy backend with no version interceptor. The Rust port
must keep that backend unwrapped and place the version service only around the
two proxy-only raw-codec frontends selected by `async-grpc-client-server`.

## Middleware ordering and error precedence

Use a single conditional service or rigorously equivalent layer order:

1. Read only the first Ployz client/minimum-server values and validate them.
2. On policy failure, synthesize the exact `FailedPrecondition` gRPC response
   immediately, with no Ployz server-version header and without invoking the
   director/codec/handler.
3. On policy success, invoke Tonic's codec service. Tonic turns an application
   `Status` into the final HTTP/gRPC response while preserving code, message,
   details, and metadata.
4. Collect any delegated `ployz-server-version` values, clear that one key,
   append the fixed package value, then re-append the delegated values in their
   original order. This reproduces grpc-go staging: the package value is first,
   handler values follow, and all survive regardless of application success or
   status. Leave every other response header untouched.

This order preserves the oracle's response precedence. In grpc-go, a
`SetHeader` failure occurs before the handler and therefore wins; the
differential probe characterizes that branch. In the selected Rust path the
key and value are prevalidated static ASCII and the header map mutation is
infallible, so that generic production failure has no reachable equivalent.
Do not create a fallible dynamic header conversion on each call. A Tower
readiness/transport error after policy acceptance must be converted to the
normal gRPC response at the codec boundary before adding the server-version
header; it must not become a headerless policy rejection.

## Hard gates

| Gate | Evidence | Result |
| --- | --- | --- |
| Required behavior | The exact oracle-package harness and exact-version Rust harness produce byte-identical output across six scenarios after only the mandated product-name substitution. The approved async-gRPC codec and HTTP/2 probes cover actual wire duplication/binary metadata, response headers, success trailers, non-OK status/details, deadlines, cancellation, and conditional forwarding. The approved SemVer differential covers all parser/comparator behavior. | `pass` |
| License/security | Tonic/Tower are MIT; SemVer is MIT or Apache-2.0. Selected code is safe Rust with no FFI. The exact 66-package probe lock passed 1,211-advisory RustSec audit; SemVer adds no transitive normal dependency. | `pass` |
| Platforms | Rust 1.96 locked all-target checks passed for Linux/macOS x86_64/aarch64. Metadata policy is platform-neutral; the shared async stack passed the same matrix. | `pass` |
| Maintenance/MSRV/adoption | Tonic 0.14.6 declares Rust 1.88 and had 3,256 reverse dependents; Tower 0.5.3 declares Rust 1.64 and had 4,559; SemVer 1.0.28 declares Rust 1.68 and had 3,408. All exact releases are current, non-yanked, and compile on Rust 1.96. | `pass` |
| Architecture | Reuses the approved gRPC and SemVer primitives, adds no competing runtime/protocol/parser, keeps response behavior in Tower/Tonic, and keeps package policy small and domain-specific. | `pass` |

## Candidate comparison

| Candidate | Fit | Decision |
| --- | --- | --- |
| Tonic 0.14.6 + Tower 0.5.3 + approved SemVer boundary | Native typed metadata/statuses, response-aware service composition, all RPC shapes, broad adoption, and no duplicate transport/parser. | **Selected.** |
| Tonic `Interceptor` alone | Correct request mutation/rejection, but receives only `Request<()>` and cannot implement conditional response headers or client response inspection. | Use only where request-only composition is natural; insufficient alone. |
| Official `grpc 0.9.0` | Preview layered on Tonic; release documentation says unstable and not recommended for production. It would replace the approved stack. | Reject. |
| `grpcio 0.13.0` / C Core | Metadata/interceptors exist, but add unsafe FFI, CMake/C Core/BoringSSL defaults, native platform surface, and incompatible message-runtime coupling. | Reject. |
| Raw `http::HeaderMap` as the complete API | Represents duplicate wire fields but does not enforce Tonic's ASCII/`-bin` distinction or supply gRPC statuses/stream integration. | Reject standalone use; permitted only at the already-selected outer HTTP response seam. |
| `tower-http` or niche interceptor helpers | Do not implement this application's policy or delayed streaming warning and still require Tonic/Tower. | Reject as redundant. |

## Verification

Two new disposable probes are outside the repository:

- `/tmp/ployz-grpc-metadata-diff-go` imports the frozen
  `internal/grpcversion` package itself under Go 1.26.1. Its fake grpc-go
  transport/client/server streams invoke the real exported interceptors.
- `/tmp/ployz-grpc-metadata-diff-rust` pins the complete exact dependency and
  feature block above under Rust 1.96.0. It instantiates Tonic metadata,
  response, and status types plus the approved SemVer boundary and the natural
  explicit-header stream wrapper.

Six isolated process scenarios assert:

1. missing, malformed, old-client, old-server, accepted, and first-duplicate
   server policy branches with exact code/message, handler invocation, and
   conditional header presence;
2. unrelated binary metadata, application status/details preservation,
   package-first preservation of a same-key handler response value, and
   header-set-versus-handler error precedence;
3. client request append order, binary preservation, and deadline metadata;
4. unary success warning versus status/transport error silence;
5. no warning at stream creation or receive, warning only after a successful
   explicit header call, no warning after header error, and warn-once behavior;
6. streaming handler status/header preservation and cancellation propagation.

After substituting only the required upstream product strings in Go output,
all six Go and Rust outputs compare byte-for-byte:

```text
DIFFERENTIAL PASS: 6/6 scenarios byte-identical after explicit product-name substitution
```

The frozen package tests also pass:

```text
GOTOOLCHAIN=local go test ./internal/grpcversion
ok github.com/psviderski/uncloud/internal/grpcversion 0.005s
```

Rust commands passed:

```text
cargo +1.96.0 fmt --check
cargo +1.96.0 check --locked --offline --all-targets
cargo +1.96.0 clippy --locked --offline --all-targets -- -D warnings
cargo +1.96.0 check --locked --offline --all-targets --target x86_64-unknown-linux-gnu
cargo +1.96.0 check --locked --offline --all-targets --target aarch64-unknown-linux-gnu
cargo +1.96.0 check --locked --offline --all-targets --target x86_64-apple-darwin
cargo +1.96.0 check --locked --offline --all-targets --target aarch64-apple-darwin
cargo audit --no-fetch --deny warnings --file Cargo.lock
```

The audit loaded 1,211 advisories and found no vulnerability in 66 resolved
packages. Exact disposable-probe hashes are:

```text
Rust Cargo.toml  a01e15a8602c98174e16079dd2740f95e5660868385f9cc255f4dc2ea604ff51
Rust Cargo.lock  1a4f7a2f0d34b76cc92b20083ced27c3a613257ce4ec3ec9095aec7ee42782d4
Rust src/main.rs e57c4bb7232588b995520bdaa118d935ce6917bfd00c3dbba4892a6c185ed51b
Go go.mod         f726184c178a68fb460a216da421f5a2879a2fc5b5a626eb9dbf9f56ca5b1824
Go go.sum         75e708cc96fef09a6c770a03770d62e967a999d45aa8533c711ddae45b874ce6
Go main.go        03a58178c80f0ad9afb2f4f4d958a7b6529503c0ed63e3e51b376442fc7e2925
```

This targeted evidence composes with the approved async-gRPC record's two
actual network probes. Those probes already establish duplicate/binary
metadata over HTTP/2, conditional proxy-only response headers, opaque status
details and trailers, whole-stream deadline handling, downstream-reset
cancellation, all unary/server-streaming/bidirectional shapes, and no version
interception on the ordinary backend. The approved SemVer record separately
establishes zero differences across 1,559 parser/format/threshold cases and
20,736 ordered comparisons. The dependency approval relies on that combined
executable evidence rather than claiming the small policy harness is itself a
complete transport.

## Primary-source evidence

- Tonic's [`Interceptor`](https://docs.rs/tonic/0.14.6/tonic/service/interceptor/trait.Interceptor.html)
  accepts and may reject `Request<()>` and explicitly directs response-aware
  middleware to Tower.
- Tonic's [`MetadataMap`](https://docs.rs/tonic/0.14.6/tonic/metadata/struct.MetadataMap.html)
  documents append, first-value get, ordered get-all, replacement by insert,
  and distinct binary accessors.
- Tonic [`Response`](https://docs.rs/tonic/0.14.6/tonic/struct.Response.html)
  exposes initial metadata; [`Streaming`](https://docs.rs/tonic/0.14.6/tonic/struct.Streaming.html)
  owns messages and trailing metadata; [`Status`](https://docs.rs/tonic/0.14.6/tonic/struct.Status.html)
  carries code, message, details, and metadata.
- grpc-go 1.74.2's exact
  [`metadata.go`](https://github.com/grpc/grpc-go/blob/v1.74.2/metadata/metadata.go),
  [`metadata tests`](https://github.com/grpc/grpc-go/blob/v1.74.2/metadata/metadata_test.go),
  and [`SetHeader` contract](https://github.com/grpc/grpc-go/blob/v1.74.2/server.go#L2069-L2094)
  define append/first-value behavior and staged initial-header timing.
- Exact manifests define the selected versions, features, licenses, and MSRVs:
  [`tonic 0.14.6`](https://docs.rs/crate/tonic/0.14.6/source/Cargo.toml.orig),
  [`tower 0.5.3`](https://docs.rs/crate/tower/0.5.3/source/Cargo.toml.orig),
  and [`semver 1.0.28`](https://docs.rs/crate/semver/1.0.28/source/Cargo.toml.orig).

## Known limitations and package acceptance requirements

- This approval selects primitives and fixes the adapter contract; it does not
  mark `internal/grpcversion` implemented. Its crate tests must port every
  frozen case and the differential scenarios above.
- The explicit streaming `header()` method is deliberate compatibility
  behavior. Inspecting Tonic's outer `Response<Streaming<_>>` eagerly would
  warn too early even though the metadata is already available.
- Incoming non-text bytes cannot be represented as valid ASCII gRPC metadata
  by Tonic's public constructor and are rejected by transport. Package policy
  must still treat a failed `to_str` as zero when presented with such a value;
  unrelated `-bin` metadata must remain untouched.
- Request keys use append, not insert. For the server response key, rebuilding
  only that key as package value followed by delegated values is required;
  ordinary `insert` would incorrectly delete a handler/backend duplicate and
  ordinary post-handler `append` would incorrectly put the package value last.
- Do not apply the server policy to the ordinary four-service backend. Do not
  add old `uncloud-*` aliases, eager stream warnings, retries, background tasks,
  protobuf/reflection, TLS, or compression through this decision.
- Protobuf message/runtime choice and connector retry behavior remain their
  own dependency gates; neither is implied by metadata approval.

Affected package: `internal/grpcversion`.

Direct downstream attachment owners: `internal/machine` and
`pkg/client/connector`.

## Review

Fresh read-only adversarial reviewer
`/root/grpc_metadata_unblock/grpc_metadata_exact_review` returned one blocking
finding on exact candidate `a785147d8d2399d4ac543a85fa959b6346e9bc69`:
the recorded Rust source hash predated final `cargo fmt`, so the available
artifact could not reproduce that commit's exact evidence. No behavior,
architecture, dependency, platform, security, caller, or scope finding was
reported.

The record was corrected to the formatted artifact's actual SHA-256,
`e57c4bb7232588b995520bdaa118d935ce6917bfd00c3dbba4892a6c185ed51b`,
in candidate `cbd65d911e4b771f0dd1a99a3f8f38748230eb37`. Fresh corrected reviewer
`/root/grpc_metadata_unblock/grpc_metadata_corrected_review` independently
reran the six differential scenarios, oracle test, Rust 1.96 format/check/
Clippy/four-target matrix, both async network probes, and RustSec audits; it
returned `CLEAN / ACCEPT` with no actionable finding. It also confirmed exact
pins/features/licenses/MSRVs, all direct callers and attachment ordering, the
Ployz rename boundary, one-record scope, and unchanged frozen oracle.
