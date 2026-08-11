# Dependency decision: protobuf message codegen and runtime

| Field | Value |
| --- | --- |
| Status | `blocked` |
| Capability | Reproducible proto3 message generation and binary runtime for `internal/machine/api/pb`, including well-known types, frozen `google.rpc.Status`, presence/oneof/enum semantics, and complete unknown-field retention |
| Selected dependency and exact version | **None.** No evaluated pure-Rust runtime passes all behavior and maintenance gates. |
| Leading behavior comparator | Google `protobuf`, `protobuf-codegen`, and `protobuf-well-known-types` `=4.35.1-release` with `protoc` 35.1 passes the wire matrix, but the target runtime compiles and links C upb and relies on a large unsafe Rust/C FFI layer. It is not a pure-Rust port dependency and is rejected. |
| License | Prost: `Apache-2.0`; rust-protobuf: `MIT`; Google v4/protoc/upb: `BSD-3-Clause`, with bundled `utf8_range` under `MIT`; frozen Status schema: `Apache-2.0` |
| Research date | `2026-08-11` UTC |
| Request | Controller delegation for `upstream/uncloud/internal/machine/api/pb`; the delegation explicitly requires a pure-Rust target runtime, meaning no C/C++ implementation is compiled or linked into the shipped Ployz target. Build-host code generators are evaluated separately. |
| Exact blocker | Prost 0.14.4 loses ordinary unknown fields; rust-protobuf 3.7.2 loses unknown groups and is approaching end of life; Google v4 preserves the required wire behavior but is a native C/upb runtime, not pure Rust, and its native security/license evidence is incomplete. A pure-Rust candidate or an explicitly reviewed pure-Rust codec design must pass the same 195-fixture oracle matrix. |

## Scope and observable contract

The immutable inputs are the five Ployz schemas plus their frozen Status import:

- [`caddy.proto`](../../upstream/uncloud/internal/machine/api/pb/caddy.proto)
- [`cluster.proto`](../../upstream/uncloud/internal/machine/api/pb/cluster.proto)
- [`common.proto`](../../upstream/uncloud/internal/machine/api/pb/common.proto)
- [`docker.proto`](../../upstream/uncloud/internal/machine/api/pb/docker.proto)
- [`machine.proto`](../../upstream/uncloud/internal/machine/api/pb/machine.proto)
- [`google/rpc/status.proto`](../../upstream/uncloud/internal/machine/api/vendor/google/rpc/status.proto)

The five package schemas contain 615 lines. Status adds 49 supporting lines. The
contract includes exact field numbers and wire types, proto3 implicit and
explicit presence/defaults, unknown enum numerics, all three oneofs, maps,
cross-file imports, `Empty`, `Timestamp`, `Duration`, `Any`, and Status.
Parsed unknown fields must survive serialization, including deprecated group
wire fields; exact noncanonical byte spelling and field order need not survive.

This decision covers message codegen and binary runtime only. It does not select
protobuf JSON, gRPC codegen or transport, HTTP/2, an executor, cancellation, or
an async runtime.

### Caller boundary

- `internal/machine/api/proxy/backend.go` appends raw protobuf envelopes and
  constructs `Metadata.status`. Forwarded payloads are deliberately not decoded
  and re-encoded; the Rust proxy must preserve that raw-wire design.
- `store/store.go` and `corromigrate/migrate.go` use `protojson`. JSON parity
  is a separate dependency/implementation obligation.
- `machine/cluster.go` uses `proto.Equal` once to avoid an unchanged
  `MachineInfo` write. Both operands on that path contain known fields. Google
  v4's `message_eq` passed an equal/different known-field probe, but no candidate
  may generalize that result to unknown-sensitive equality.
- No handwritten frozen caller uses `ProtoReflect`, a message descriptor, or a
  file descriptor API. Generated Go invokes reflection internally, but matching
  the generated Go API shape is not an observable contract.

Descriptor-set equality is therefore a **schema/codegen validation artifact**,
not a runtime-reflection requirement. A checked `FileDescriptorSet` can prove
field/service schema parity without requiring stable reflection in the selected
Rust runtime. Google v4's lack of a stable generated-message reflection API is
a tooling limitation, not the reason this decision is blocked.

### Primary-source anchors

- Google's [proto3 unknown-field rules](https://protobuf.dev/programming-guides/proto3/#unknown-fields)
  require parsed unknown fields to be included again on serialization; its
  [noncanonical-serialization guidance](https://protobuf.dev/programming-guides/serialization-not-canonical/)
  explains why semantic/raw-unknown equality, rather than universal byte
  equality, is the hard gate.
- Prost's exact [unknown-field skip path](https://docs.rs/prost/0.14.4/src/prost/encoding.rs.html#166-199)
  and [descriptor-set documentation](https://github.com/tokio-rs/prost/tree/v0.14.4#accessing-the-protoc-filedescriptorset)
  distinguish typed-message behavior from a separately emitted schema artifact.
- rust-protobuf's exact
  [`read_unknown_or_skip_group`](https://docs.rs/crate/protobuf/3.7.2/source/src/rt/unknown_or_group.rs)
  stores four non-group wire kinds but skips a complete group; the maintainer's
  [official README](https://github.com/stepancheg/rust-protobuf#end-of-life)
  records the approaching-EOL status.
- Google v4's exact [runtime manifest](https://docs.rs/crate/protobuf/4.35.1-release/source/Cargo.toml.orig)
  and [`build.rs`](https://docs.rs/crate/protobuf/4.35.1-release/source/build.rs)
  show the `cc` build dependency and two bundled C inputs. The official
  [Rust codegen documentation](https://docs.rs/crate/protobuf-codegen/4.35.1-release)
  calls code generation beta and requires matching protoc 35.1.
- The official [protobuf v35.1 release](https://github.com/protocolbuffers/protobuf/releases/tag/v35.1),
  [BSD license](https://github.com/protocolbuffers/protobuf/blob/v35.1/LICENSE),
  [`utf8_range` MIT license](https://github.com/protocolbuffers/protobuf/blob/v35.1/third_party/utf8_range/LICENSE),
  and [security policy](https://github.com/protocolbuffers/protobuf/blob/v35.1/SECURITY.md)
  are the authority for the native tool/runtime provenance below.

## Candidate comparison

| Candidate | Pure Rust | Behavior result | Maintenance / integration | Decision |
| --- | --- | --- | --- | --- |
| `prost = 0.14.4`, `prost-build = 0.14.4`, `prost-types = 0.14.4` | Yes for the target runtime | Parses all 195 fixtures, but only 116/195 retain oracle semantics because all 75 top-level and four nested injected-unknown fixtures lose unknown fields. Unknown enum `i32` values do survive. | Most adopted and idiomatic candidate; active release, MSRV 1.85, Apache-2.0. `prost-reflect::DynamicMessage` does not cause typed Prost messages to retain unknowns. | **Reject: behavior hard gate.** |
| `protobuf = 3.7.2`, `protobuf-codegen = 3.7.2` | Yes for the target runtime; it contains Rust `unsafe`, but no C/FFI runtime | Parses all 195 fixtures, but only 116/195 retain semantics. Varint/fixed32/fixed64/length-delimited unknowns survive semantically; complete unknown groups are consumed and dropped. | Mature and adopted, RustSec group-recursion fix is present in 3.7.2, but the maintainer says the implementation is approaching EOL and needs a new maintainer. | **Reject: behavior and maintenance hard gates.** |
| Google `protobuf = 4.35.1-release`, `protobuf-codegen = 4.35.1-release`, `protobuf-well-known-types = 4.35.1-release` | **No.** Cargo builds bundled C upb and `utf8_range` for the target and links them through unsafe FFI. | 195/195 semantic and unknown-byte matches; 193/195 exact byte matches. The two byte differences are permitted noncanonical serialization. | Officially maintained, MSRV 1.79, codegen still documented beta. Exact protoc 35.1 is required. | **Reject: violates the pure-Rust target-runtime requirement; native security/license gate is also incomplete.** |
| `quick-protobuf = 0.8.1` + `pb-rs = 0.10.0` | Yes | Generated readers skip rather than retain unknown fields and provide no equivalent typed full reflection. | Last releases in 2022; WKT/Status/import policy would become local code. | **Reject: behavior and maintenance gates.** |

Popularity cannot override a failed hard gate. The confirmed crates.io snapshot
still explains the ordering: Prost is the dominant idiomatic option; rust-
protobuf is materially adopted; Google's new Rust runtime is official but much
newer. None is approved.

## Confirmed wire matrix

A descriptor-driven Go oracle generated 195 fixtures for all 75 non-map-entry
messages reachable from the six direct inputs:

- 75 recursively populated messages;
- 41 isolated default-valued presence cases;
- 75 populated messages with an unknown-field suffix; and
- four nested/repeated-unknown fixtures in an ordinary message, `Timestamp`,
  `Duration`, and `Status.details`/`Any`.

Every scalar, bytes/string, enum, singular message, repeated field, map, proto3
optional, and member of all three oneofs was exercised. Every declared enum
field used unknown numeric value 777. Unknown suffixes covered all six wire
types, including a complete group, and repeated values of one unknown field.

| Candidate | Parsed | Semantic matches | Raw unknown matches | Exact byte matches |
| --- | ---: | ---: | ---: | ---: |
| Google v4/upb | 195/195 | 195/195 | 195/195 | 193/195 |
| Prost 0.14.4 | 195/195 | 116/195 | 116/195 | 115/195 |
| rust-protobuf 3.7.2 | 195/195 | 116/195 | 116/195 | 92/195 |

The fixture manifest SHA-256 was
`2fcba162c0657e19951d57fb3441ac1c5e1651229a912f87267cd99eca62a8bd`.
Two Google runs produced identical output sets. These results are confirmed
research evidence and must be preserved, but the temporary harness was not
committed to this repository. They are consequently insufficient by themselves
for a future approval. Before approval, the package must land a repository-
owned equivalent harness and reproduce the result from the frozen oracle.

The 195-result distinction is intentional: Google v4 proves that preserving the
wire contract is feasible, but it does not prove a pure-Rust candidate exists.
Prost and rust-protobuf fail the same 79 unknown-bearing cases for different
runtime reasons.

## Codegen tool versus target runtime

`protoc` is a build-host/release code generator. It does not become part of the
deployed target runtime. Its host architecture therefore must match the machine
that runs generation, not the Cargo target. Generated Rust is target-neutral;
Google v4's bundled C upb is separately compiled for the Cargo **target**.
Conflating these two matrices previously produced an invalid requirement to
source-build protoc on every target/host pair.

### Official protoc 35.1 prebuilts

The official v35.1 GitHub release publishes these relevant host tools. GitHub's
release API reports the same SHA-256 digests, and fresh downloads matched them:

| Build host | Official asset | SHA-256 |
| --- | --- | --- |
| Linux x86_64 | `protoc-35.1-linux-x86_64.zip` | `6930ebf62bd4ea607b98fff052596c6ee564b9835b4ce172c75a3f53ae9d91b7` |
| Linux aarch64 | `protoc-35.1-linux-aarch_64.zip` | `01bf9d08808c7f96678b63f4bd8efa559bb4f83d5a7a270d5edaf507f9d5d9cf` |
| macOS x86_64 | `protoc-35.1-osx-x86_64.zip` | `537d73604a344ded6fc94e98e07e529d4fe3e4a0b09e59905353950fafc2a1f7` |
| macOS aarch64 | `protoc-35.1-osx-aarch_64.zip` | `193289af0470c6a1aada357d4fba0bbf8d78bfaac8b5e42ca30af2ef75583de2` |

Each archive contains `bin/protoc`, the Google WKT include tree, and the same
`readme.txt` (`cb87be42344000337bef2e65de178c7e2f9cd7a1b7cd0e4284377a5d375db82f`).
The readme does **not** contain the BSD license. Any cached or redistributed
tool must also install the exact v35.1 `LICENSE`, SHA-256
`6e5e117324afd944dcf67f36cf329843bc1a92229a8cd9bb573d7a83130fea7d`.
The prebuilt archives have release digests but no separate per-asset signature
or provenance statement in the release asset set inspected here.

The Linux x86_64 binary reports `libprotoc 35.1` and is statically linked. A
release tool can select one of the four exact host assets, verify its digest
before execution, reject unknown hosts, avoid `PATH`, and run offline from an
explicit cache. That is a viable codegen-tool design; source-only protoc is not
a hard gate.

### Checked generated snapshot

A second viable design is to commit generated Rust plus a `FileDescriptorSet`,
then regenerate in one canonical release job with the exact verified Linux
x86_64 prebuilt and fail on any diff. Ordinary developer and cross-target builds
would not execute protoc. This reduces host-tool exposure and avoids fabricating
an all-host codegen requirement, while retaining reproducibility. It is valid
only if the repository contains the generation command/configuration and CI
compares the entire generated tree and descriptor set; unchecked generated code
is not acceptable.

The official prebuilt and checked-snapshot strategies both solve compiler
provisioning. Neither changes Google v4's deployed C/upb runtime, so neither can
make that candidate pure Rust.

### Historical source-build evidence, correctly scoped

An official protobuf 35.1 source archive
(`f0b6838e7522a8da96126d487068c959bc624926368f3024ac8fd03abd0a1ac4`)
plus Abseil 20250512.1
(`9b7a064305e9fd94d124ffa6cc358592eb42b5da588fb4e07d09254aa40086db`)
was built fully disconnected on Linux in a temporary research directory. The
produced `libprotoc 35.1` emitted seven files identical to the Linux prebuilt
output. The build harness and output manifest were not committed, so this is
historical corroboration only: it is not reconstructed by the command block
below and is not approval evidence. In particular, it creates neither an
obligation to build protoc from source on every host nor evidence about target-
runtime portability.

## Exact native license and security analysis of Google v4

The crates.io lock evidence for the exact Google candidate is:

| Crate | Cargo checksum | Declared license |
| --- | --- | --- |
| `protobuf 4.35.1-release` | `a169648cc34d6f327fea8919ca63f38261fb26405fde8879745dc0a483db328e` | BSD-3-Clause |
| `protobuf-codegen 4.35.1-release` | `19dffd9a419f79bca4fad6bc09d5ead0e932596b76e4b3d7e79acc7e2eba4b98` | BSD-3-Clause |
| `protobuf-well-known-types 4.35.1-release` | `d7a458cbf1352c531efdfbd2499f2814cfaa7dd59c9fbf88f36efb3f08ca2b2c` | BSD-3-Clause |

The runtime `build.rs` unconditionally invokes `cc`, compiles
`libupb/upb/upb.c` (15,503 lines, SHA-256
`ad14fdcd0da6fa09632443356f797d124e70f3f11cb6b4f12ced3318e0258505`)
and `libupb/third_party/utf8_range/utf8_range.c` (207 lines, SHA-256
`f564d1f3bb9e1a477a30e683c4d994a113fb6efcd98b51b6ae95cbe9ee4c936b`),
then links `libupb` into the target. The crate package contains 192 files under
`libupb`. This is native runtime code, not merely a build-host generator.

An exact source scan found 39 Rust source files containing `unsafe` or
`extern "C"`, 465 matching lines, 28 `extern "C"` declarations, 28 unsafe
impls, and 89 unsafe-function declarations. Counts are inventory evidence, not
a claim that each use is unsound. They establish that memory safety depends on
the Rust/C ABI, pointer/arena invariants, generated mini-tables, and the bundled
C implementation. The crate does deny `unsafe_op_in_unsafe_fn`, which is useful
but does not eliminate that boundary.

The top-level crate license file is BSD-3-Clause and hashes to
`6e5e117324afd944dcf67f36cf329843bc1a92229a8cd9bb573d7a83130fea7d`.
Bundled `utf8_range` source headers declare MIT, but its MIT license file is not
present under the published crate's `libupb` tree. The corresponding exact v35.1
upstream license hashes to
`02de69b64fc36d9e938f418e52723e42f0b2b226d58a9cb3c8dcbdf7059f5074`.
Redistributing this runtime would therefore require preserving both notices and
reviewing whether the published crate's omission needs a downstream notice
install. The permissive license classes pass; the packaged-notice audit is
conditional, not silently waived.

The historical `cargo audit --no-fetch --deny warnings` result over a
16-package Google-v4 lock was clean against the then-local 1,211-advisory
RustSec database. Neither that transient lock nor the advisory-database
revision was committed, so the result is not reconstructible and is not
approval evidence. The command below instead performs a fresh scan whose
transitive resolution and advisory snapshot may differ. Cargo/RustSec does not
audit the bundled C upb implementation or the downloaded protoc executable.
The official v35.1 `SECURITY.md` (SHA-256
`6eefe2a6fbf4e9f404726d9b0b5eee43a4cd265643ee5844d9db13346820fb5f`)
provides Google's vulnerability reporting process, but this investigation did
not persist sanitizer/fuzzer results for the exact vendored C snapshot or
independent binary provenance. Thus the native security gate is **incomplete**,
in addition to the decisive pure-Rust failure.

## Hard gates

| Gate | Requirement | Prost 0.14.4 | rust-protobuf 3.7.2 | Google v4 4.35.1 |
| --- | --- | --- | --- | --- |
| Observable wire behavior | All schema semantics and complete unknown retention | **Fail**: 79 unknown-bearing fixtures lose data | **Fail**: unknown groups are dropped in 79 fixtures | Pass: 195/195 semantic and raw-unknown matches |
| Pure-Rust target runtime | No C/C++ runtime linked into the Ployz target | Pass | Pass | **Fail**: bundled C upb/utf8 plus unsafe FFI |
| License/security | Permissive, notices complete, untrusted-input surface reviewed | Pass at dependency level; integrated audit still required | RustSec-fixed exact version; runtime lifecycle remains a risk | **Incomplete**: missing bundled MIT notice in crate tree; RustSec excludes C; no persisted native sanitizer/provenance result |
| Supported targets | Rust 1.96; Linux/macOS amd64/arm64 | Rust target checks passed | Rust target checks passed | Linux x86_64/aarch64 checks passed; Apple cross-check from Linux lacked an Apple C compiler/SDK. This target-native C matrix is separate from protoc hosts. |
| Maintenance | Active enough for a foundational wire contract | Pass, with documented limited bandwidth | **Fail**: approaching EOL and seeking maintainer | Pass; codegen API stability remains a recorded risk |
| Architecture | Typed message API, reproducible generation, no transport/runtime coupling | Pass except unknown retention | Pass except behavior/lifecycle | Fails pure-Rust rule; stable runtime reflection is not required |

No candidate passes every row. Keep this decision `blocked` and do not add a
protobuf dependency to the workspace.

## Reconstructible verification from repository state

The following commands use only the repository's frozen oracle plus exact
official release assets. They reconstruct the source hashes, v35.1 descriptor
golden, compiler identity, notice/security hashes, and dependency/native-source
inventory. The audit is explicitly a fresh current scan. These commands do not
reconstruct the historical source build, 27.3/v31.1 descriptor comparison,
historical audit, or 195-fixture harness; none is approval evidence. Approval
requires a repository-owned oracle harness and pinned acceptance inputs.

```sh
repo=$(git rev-parse --show-toplevel)
test "$(git -C "$repo" status --porcelain -- upstream/uncloud)" = ""
sha256sum \
  "$repo"/upstream/uncloud/internal/machine/api/pb/{caddy,cluster,common,docker,machine}.proto \
  "$repo"/upstream/uncloud/internal/machine/api/vendor/google/rpc/status.proto

(cd "$repo/upstream/uncloud" && \
  GOTOOLCHAIN=local /opt/go1.26.1/bin/go test \
    ./internal/machine/api/pb ./internal/machine/api/proxy)

probe=$(mktemp -d /tmp/ployz-protobuf-decision.XXXXXX)
for asset in \
  protoc-35.1-linux-x86_64.zip \
  protoc-35.1-linux-aarch_64.zip \
  protoc-35.1-osx-x86_64.zip \
  protoc-35.1-osx-aarch_64.zip
do
  curl -fsSLo "$probe/$asset" \
    "https://github.com/protocolbuffers/protobuf/releases/download/v35.1/$asset"
done
(cd "$probe" && sha256sum -c <<'SUMS'
6930ebf62bd4ea607b98fff052596c6ee564b9835b4ce172c75a3f53ae9d91b7  protoc-35.1-linux-x86_64.zip
01bf9d08808c7f96678b63f4bd8efa559bb4f83d5a7a270d5edaf507f9d5d9cf  protoc-35.1-linux-aarch_64.zip
537d73604a344ded6fc94e98e07e529d4fe3e4a0b09e59905353950fafc2a1f7  protoc-35.1-osx-x86_64.zip
193289af0470c6a1aada357d4fba0bbf8d78bfaac8b5e42ca30af2ef75583de2  protoc-35.1-osx-aarch_64.zip
SUMS
)
for asset in \
  protoc-35.1-linux-x86_64.zip \
  protoc-35.1-linux-aarch_64.zip \
  protoc-35.1-osx-x86_64.zip \
  protoc-35.1-osx-aarch_64.zip
do
  test "$(unzip -p "$probe/$asset" readme.txt | sha256sum | cut -d' ' -f1)" = \
    "cb87be42344000337bef2e65de178c7e2f9cd7a1b7cd0e4284377a5d375db82f"
  test -z "$(unzip -Z1 "$probe/$asset" | rg '(^|/)LICENSE($|\.)' || true)"
done
curl -fsSLo "$probe/protobuf-LICENSE" \
  https://raw.githubusercontent.com/protocolbuffers/protobuf/v35.1/LICENSE
curl -fsSLo "$probe/utf8-range-LICENSE" \
  https://raw.githubusercontent.com/protocolbuffers/protobuf/v35.1/third_party/utf8_range/LICENSE
curl -fsSLo "$probe/SECURITY.md" \
  https://raw.githubusercontent.com/protocolbuffers/protobuf/v35.1/SECURITY.md
(cd "$probe" && sha256sum -c <<'SUMS'
6e5e117324afd944dcf67f36cf329843bc1a92229a8cd9bb573d7a83130fea7d  protobuf-LICENSE
02de69b64fc36d9e938f418e52723e42f0b2b226d58a9cb3c8dcbdf7059f5074  utf8-range-LICENSE
6eefe2a6fbf4e9f404726d9b0b5eee43a4cd265643ee5844d9db13346820fb5f  SECURITY.md
SUMS
)
unzip -q "$probe/protoc-35.1-linux-x86_64.zip" -d "$probe/protoc"
test "$("$probe/protoc/bin/protoc" --version)" = "libprotoc 35.1"

(cd "$repo/upstream/uncloud" && \
  "$probe/protoc/bin/protoc" \
    -I . \
    -I internal/machine/api/vendor \
    -I "$probe/protoc/include" \
    --descriptor_set_out="$probe/direct.pb" \
    internal/machine/api/pb/caddy.proto \
    internal/machine/api/pb/cluster.proto \
    internal/machine/api/pb/common.proto \
    internal/machine/api/pb/docker.proto \
    internal/machine/api/pb/machine.proto \
    google/rpc/status.proto)
test "$(wc -c < "$probe/direct.pb")" = "11671"
echo '6c3cbf942bd21b74e30b231480d1b6c8e79b026417e3b86cb00be4d76ff88bdf  direct.pb' \
  | (cd "$probe" && sha256sum -c)

mkdir "$probe/cargo"
printf '%s\n' \
  '[package]' \
  'name = "protobuf-decision-inspect"' \
  'version = "0.0.0"' \
  'edition = "2024"' \
  '' \
  '[dependencies]' \
  'protobuf = "=4.35.1-release"' \
  'protobuf-well-known-types = "=4.35.1-release"' \
  '' \
  '[build-dependencies]' \
  'protobuf-codegen = "=4.35.1-release"' \
  > "$probe/cargo/Cargo.toml"
mkdir "$probe/cargo/src"
printf '%s\n' 'pub fn marker() {}' > "$probe/cargo/src/lib.rs"
cargo +1.96.0 generate-lockfile --manifest-path "$probe/cargo/Cargo.toml"
cargo +1.96.0 vendor --locked --versioned-dirs \
  --manifest-path "$probe/cargo/Cargo.toml" "$probe/vendor" >/dev/null
cargo +1.96.0 check --locked --offline \
  --manifest-path "$probe/cargo/Cargo.toml"
# This is a fresh scan, not a reproduction of the historical audit snapshot.
cargo audit --no-fetch --deny warnings --file "$probe/cargo/Cargo.lock"

runtime="$probe/vendor/protobuf-4.35.1-release"
wc -l \
  "$runtime/libupb/upb/upb.c" \
  "$runtime/libupb/third_party/utf8_range/utf8_range.c"
sha256sum \
  "$runtime/LICENSE" \
  "$runtime/build.rs" \
  "$runtime/libupb/upb/upb.c" \
  "$runtime/libupb/third_party/utf8_range/utf8_range.c"
(cd "$runtime" && sha256sum -c <<'SUMS'
6e5e117324afd944dcf67f36cf329843bc1a92229a8cd9bb573d7a83130fea7d  LICENSE
cbb1a47443a1c50d888c3124662dca3136314f94d331ab0db25a5fe3340d4146  build.rs
ad14fdcd0da6fa09632443356f797d124e70f3f11cb6b4f12ced3318e0258505  libupb/upb/upb.c
f564d1f3bb9e1a477a30e683c4d994a113fb6efcd98b51b6ae95cbe9ee4c936b  libupb/third_party/utf8_range/utf8_range.c
SUMS
)
test "$(find "$runtime/libupb" -type f | wc -l)" = "192"
test "$(rg -l 'unsafe|extern "C"' "$runtime/src" | wc -l)" = "39"
test "$(rg -n 'unsafe|extern "C"' "$runtime/src" | wc -l)" = "465"
test "$(rg -n 'extern "C"' "$runtime/src" | wc -l)" = "28"
test "$(rg -n 'unsafe impl' "$runtime/src" | wc -l)" = "28"
test "$(rg -n '^unsafe fn|pub unsafe fn|unsafe extern' "$runtime/src" | wc -l)" = "89"
```

Expected schema hashes, in command order, are:

```text
dc5f3696159ccf20d527fb83a8fe4efa919504a69e0d5b2654511735a7fd2218  caddy.proto
48cc57aa8c8b904972d4c0a1d090485dad91e2deb58b111104700d21e28e1668  cluster.proto
ff5cde0ac982942d237810e9cd3e962b0bc705d88fb532c99b1e29beb0fc431d  common.proto
4bb8ed7f1985b29765e94f540d28d2c1b37822b3c894aaf571e2bd591c1f061a  docker.proto
89da97a9f1617822a74006cdf38f949892480b25a91af2725d7970fcbc961062  machine.proto
b35706fa0e4b2354f67f8d7b8e6b55584d0c4dae920d5f6d2bc2e7ba22f9d6c1  status.proto
```

The descriptor set is intentionally the six direct input descriptors, without
imported WKT file bodies. The command reconstructs the official v35.1 result:
11,671 bytes at the recorded hash. Temporary historical probes reported the
same bytes from frozen compiler 27.3 and official v31.1, but their harnesses and
outputs were not persisted, so that cross-version comparison is corroboration,
not approval evidence. The v35.1 golden proves schema grammar/descriptor parity
for the evaluated compiler; it does not manufacture a runtime-reflection
requirement.

## No approved integration

Do not add Prost, rust-protobuf, Google v4, `prost-reflect`, a custom protobuf
codec, tonic, grpc, Tokio, Hyper, or any transport generator under this blocked
decision. An eventual approval must:

1. use a pure-Rust target runtime;
2. reproduce the 195-fixture semantic and raw-unknown result from a committed
   repository harness;
3. retain the raw proxy concatenation path;
4. validate deterministic generated source and the descriptor golden;
5. pin generator/runtime/schema/tool coordinates together; and
6. pass Rust 1.96 plus Linux/macOS amd64/arm64 target checks.

If a checked generated snapshot is chosen, protoc is a release/verification
tool and need only run on the chosen build host. If generation remains in
`build.rs`, use an explicit checksummed official 35.1 host tool and fail on
unsupported hosts; never consult `PATH` or download during an ordinary build.

## Review

The inherited candidate record was rejected because it treated Google v4 as
though its C/upb runtime could satisfy a pure-Rust port, conflated build-host
protoc with target-native upb, made an all-host source-protoc matrix a hard
gate, under-specified native licensing/security, relied on `/tmp` probes as
reconstructible evidence, and mixed descriptor validation with unused runtime
reflection. This correction keeps the decision blocked for the narrower,
truthful reason: no evaluated pure-Rust runtime passes the required wire and
maintenance gates.

Fresh adversarial reviewer `/root/protobuf_record_fix/protobuf_corrected_review`
reproduced every core empirical claim and the 195-fixture result, then returned
`NEEDS CHANGES` on commit `585997d`: the pure-Rust gate lacked durable provenance,
and the record overstated reconstruction of unpersisted source-build,
cross-version descriptor, notice/security-hash, and audit evidence. This
revision records the delegation's exact gate, adds executable notice/security
hash checks, labels the scan as current, and explicitly demotes every
unpersisted comparison to non-approval historical evidence. No finding was
waived.

Fresh read-only re-reviewer `/root/protobuf_record_fix/protobuf_final_review`
then inspected exact commit
`a7019ce3f99316b84c57d92e4d1315b71af8e030`, reran the complete documented
verification, confirmed the corrected evidence boundaries and preserved matrix,
and returned **`ACCEPT / CLEAN` with zero actionable findings**. The only change
after that reviewed commit is this durable review-result paragraph.
