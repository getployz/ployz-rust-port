# Dependency decision: `embedded-dns-server-client-runtime`

| Field | Value |
| --- | --- |
| Status | `approved` |
| Selected dependency | `hickory-proto = 0.26.1`; existing runtime `tokio = 1.53.1`; `tokio-util = 0.7.19`; existing randomizer `fastrand = 2.5.0`; `nix = 0.31.3` |
| License | `MIT OR Apache-2.0`; `MIT`; `MIT`; `Apache-2.0 OR MIT`; `MIT` |
| Research date | `2026-08-12` UTC |
| Request | Capability assigned directly by the controller; no request file exists |

Select Hickory's low-level protocol crate, not its higher-level server or
resolver policy. `hickory-proto` supplies the maintained, popular DNS data
model, safe parser, serializer, EDNS representation, record types, and bounded
encoder. Narrow package-owned Tokio loops supply UDP/TCP serving and raw
forwarding because the oracle's asymmetric startup, exact per-transport
forwarding, overload, and graceful-drain policy do not fit a generic DNS
server/resolver lifecycle.

## Oracle and caller contract

Repository-wide symbol and caller search excluded `upstream/uncloud/experiment/**`
as directed. The frozen contract comes from
[`server.go`](../../upstream/uncloud/internal/machine/dns/server.go),
[`resolver.go`](../../upstream/uncloud/internal/machine/dns/resolver.go),
[`resolver_test.go`](../../upstream/uncloud/internal/machine/dns/resolver_test.go),
and direct consumers in `internal/machine/{machine,cluster}.go` and
`internal/machine/docker/{server,controller_linux}.go`:

- validate the listen IP and resolver; listen on port 53 at that one address;
  run UDP and TCP concurrently; a UDP startup/runtime failure is fatal, while
  TCP bind/runtime failure is logged and ignored;
- cancellation closes both listeners and connections, stops accepting new
  work, and waits for every already accepted request/forward to finish; preserve
  the miekg defaults that UDP receives at most 512 bytes, the first TCP read has
  a two-second deadline, later reads an eight-second idle deadline, and a TCP
  connection serves at most 128 queries;
- parse and serialize DNS wire messages. A packet shorter than the 12-byte
  header is silently dropped. Header admission silently ignores QR=response,
  returns `NOTIMP` for opcodes other than QUERY/NOTIFY, and returns `FORMERR`
  unless QDCOUNT is exactly one, ANCOUNT and NSCOUNT are at most one, and
  ARCOUNT is at most two. A readable admitted header whose body cannot parse
  also receives `FORMERR`. Count-level errors have empty sections. A body error
  after a valid question retains that decoded question but clears Answer,
  Authority, and Additional; a missing/malformed question leaves it empty.
  preserve ID plus TC/RD/RA/AD/CD, clear AA and the reserved Z bit, set QR, and
  set opcode QUERY for `FORMERR` (retain the rejected opcode for `NOTIMP`).
  Consequently empty and multiple-question requests never reach the package
  handler. For an admitted request, only the first question is handled/copied;
- names outside the canonical case-insensitive `internal.` subtree are
  forwarded unchanged in meaning over the incoming transport. Upstreams are
  tried sequentially in configured order. Each upstream gets a three-second
  dial timeout followed, after connection, by a fresh common three-second
  write/read deadline; a TCP attempt can therefore approach six seconds. UDP
  ignores mismatched response IDs until the deadline, while TCP rejects one;
  either case then falls back. UDP receives 512 bytes without usable EDNS or
  the request's advertised EDNS size when at least 512. Accept the first
  matching, parseable transport-success response regardless of RCODE. The
  1,024-request limit is a nonblocking admission cap across UDP and TCP; empty
  upstreams, overload, timeout, ID/parse failure, or exhaustion returns
  `SERVFAIL`;
- internal replies are authoritative and recursion-available. `A` queries use
  TTL zero and return `NXDOMAIN` when the resolver yields no addresses. Every
  unsupported type returns authoritative `NOERROR` with no answers, even for a
  missing name;
- shuffle every multi-address A result. Default and `rr.` modes retain the
  shuffled order. `nearest.` stably partitions addresses on the configured
  local subnet first, preserving random order within both partitions;
- UDP internal replies use 512 bytes unless EDNS advertises a larger size
  (values below 512 are clamped); TCP uses 65,535. Truncation first tests the
  uncompressed message, then compresses and retains as many complete records as
  fit, sets `TC`, and does not echo the request's OPT record in the internal
  response;
- `None` upstreams read `/etc/resolv.conf` with Go `strings.Fields` semantics:
  only a line whose first token is exactly `nameserver` contributes its second
  token; trailing tokens are ignored, while inline `#`/`;` attached to the
  address are part of that token and make it invalid. Retain valid IPv4/IPv6
  `netip` forms in file order, including arbitrary nonempty IPv6 zones. Match
  `bufio.Scanner`'s 64 KiB token ceiling: stop on an overlong line while
  retaining servers parsed before it. Exclude
  only a zone-aware exact match to the listen address. Use `1.1.1.1:53` then
  `8.8.8.8:53` when none remain or the file cannot be read. Explicit
  `Some(vec![])` disables external forwarding. Resolve an IPv6 zone by trying
  the exact platform interface-name lookup first; if absent, consume its
  leading decimal prefix as the index (zero when it has no such prefix and
  capped at `0xFFFFFF`), immediately before dialing;
- the server IP is also handed to Docker DNS/search configuration and Linux
  firewall rules, so listener identity and TCP/UDP availability are observable.

The cluster-store subscription and resolver-index update mechanics are owned by
the package and its internal store dependency, not this external capability.

## Hard gates

| Gate | Requirement | Primary-source evidence | Result |
| --- | --- | --- | --- |
| Behavior | Complete DNS wire model, A/unknown records, EDNS, bounded truncating encoding, UDP/TCP I/O, timeouts, cancellation, and scoped resolv.conf input | `Message::{from_vec,to_vec}`, public sections/metadata and `BinEncoder::set_max_size` are in Hickory's tagged [message source](https://github.com/hickory-dns/hickory-dns/blob/v0.26.1/crates/proto/src/op/message.rs) and [encoder source](https://github.com/hickory-dns/hickory-dns/blob/v0.26.1/crates/proto/src/serialize/binary/encoder.rs). Tokio documents cancellation-safe [`UdpSocket::recv_from`](https://docs.rs/tokio/1.53.1/tokio/net/struct.UdpSocket.html#method.recv_from), [`TcpListener::accept`](https://docs.rs/tokio/1.53.1/tokio/net/struct.TcpListener.html#method.accept), and timeout/async I/O APIs. `CancellationToken` provides child cancellation and awaited cancellation ([API](https://docs.rs/tokio-util/0.7.19/tokio_util/sync/struct.CancellationToken.html)). Go's tagged [`ClientConfigFromReader`](https://github.com/miekg/dns/blob/v1.1.65/clientconfig.go) and [`netip.ParseAddr`](https://github.com/golang/go/blob/go1.26.1/src/net/netip/netip.go) define the narrow parser seam; `nix::net::if_::if_nametoindex` safely converts nonnumeric scope names ([API](https://docs.rs/nix/0.31.3/nix/net/if_/fn.if_nametoindex.html)). Runnable probes covered the hard paths below. | `pass`, with package seams specified below |
| License and security | Permissive licenses, safe application API, no unresolved advisory in exact lockfile, bounded hostile input/work | Tagged manifests declare [Hickory MIT OR Apache-2.0](https://github.com/hickory-dns/hickory-dns/blob/v0.26.1/Cargo.toml), [Tokio MIT](https://github.com/tokio-rs/tokio/blob/tokio-1.53.1/LICENSE), [Tokio-util MIT](https://github.com/tokio-rs/tokio/blob/tokio-1.53.1/tokio-util/LICENSE), [fastrand Apache-2.0 OR MIT](https://github.com/smol-rs/fastrand/blob/v2.5.0/Cargo.toml), and [nix MIT](https://github.com/nix-rust/nix/blob/v0.31.3/LICENSE). The selected Hickory release fixes the name-compression CPU-amplification advisory and NSEC3 loop ([release](https://github.com/hickory-dns/hickory-dns/releases/tag/v0.26.1), [GHSA-q2qq-hmj6-3wpp](https://github.com/hickory-dns/hickory-dns/security/advisories/GHSA-q2qq-hmj6-3wpp)); DNSSEC features are disabled. Hickory publishes a current [security policy](https://github.com/hickory-dns/hickory-dns/blob/v0.26.1/SECURITY.md) and fuzzes message round trips ([fuzz target](https://github.com/hickory-dns/hickory-dns/blob/v0.26.1/fuzz/fuzz_targets/message.rs)). `cargo audit --deny warnings` passed. | `pass` |
| Platforms and targets | Rust 1.96; Linux/macOS x86_64/aarch64 | Published manifests declare MSRVs 1.88 for Hickory, 1.71 for Tokio/Tokio-util, 1.63 for fastrand, and 1.69 for nix. The exact probe passed Rust 1.96 checks for all four requested targets. Hickory's manifest publishes Apple and Linux support; nix's [manifest](https://github.com/nix-rust/nix/blob/v0.31.3/Cargo.toml) includes x86_64 macOS and Unix networking. | `pass`; native runtime tests remain required on macOS |
| Maintenance and adoption | Current, maintained, popular, idiomatic crates | On 2026-08-12 the primary crates.io APIs reported `hickory-proto` about 68.8M total/19.0M recent downloads and 110 reverse dependencies and `domain` about 11.3M/1.0M. Hickory 0.26.1 shipped 2026-05-01 and its repository had about 5.4k stars with commits on 2026-08-11; Tokio 1.53.1 shipped 2026-07-20. Sources: [hickory-proto API](https://crates.io/api/v1/crates/hickory-proto), [domain API](https://crates.io/api/v1/crates/domain), [Hickory releases](https://github.com/hickory-dns/hickory-dns/releases), [Tokio releases](https://github.com/tokio-rs/tokio/releases). | `pass` |
| Architectural constraints | Natural APIs, explicit ownership, graceful draining, nonblocking bound, no hidden resolver policy | `Message`/`BinEncoder` are direct protocol primitives; Tokio sockets/tasks/semaphore expose the lifecycle explicitly. No custom DNS parser, unsafe application code, subprocess, global resolver, cache, retry policy, or Go-shaped dependency adapter is required. | `pass` |

## Candidate comparison

Adoption snapshots above are preference evidence only after hard gates.

| Candidate | Evidence and result |
| --- | --- |
| `hickory-proto 0.26.1` plus package-owned Tokio transport | Strongest fit. It is the widely adopted maintained low-level crate specifically documented for direct DNS packet manipulation ([crate README](https://github.com/hickory-dns/hickory-dns/blob/v0.26.1/crates/proto/README.md)). Public typed messages and bounded encoding matched the oracle differential. It does not impose cache, retry, fallback, or lifecycle policy. |
| `hickory-server 0.26.1` | Rejected as the direct runtime, though its underlying protocol crate wins. Its single aggregate server cancellation token and `block_until_done` model ([server source](https://github.com/hickory-dns/hickory-dns/blob/v0.26.1/crates/server/src/server/mod.rs)) cannot naturally express UDP-fatal/TCP-nonfatal startup/runtime disposition. More seriously, UDP and TCP loops own local `JoinSet`s for in-flight handlers; leaving those loops on cancellation drops the sets, and Tokio documents that dropping [`JoinSet`](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinSet.html#drop) aborts all contained tasks. That violates the oracle's graceful request/forward drain. Its UDP layer also uses a fixed 4,096-byte receive buffer and its response encoder uses 4,096 without EDNS, differing from the oracle's 512-byte request/response baseline. |
| `hickory-resolver 0.26.1` / higher-level Hickory forwarding authority | Rejected for this forwarding seam. It is a stub/recursive resolver with name-server pools, retries, caching/configuration, response interpretation and UDP-to-TCP behavior. The oracle instead forwards the already received DNS message to each configured address sequentially, preserves the incoming transport, accepts the first transport-success response regardless of RCODE, and applies a fresh fixed timeout per server. Reproducing those semantics below the resolver would still require the selected protocol/Tokio primitives. |
| `domain 0.12.2` | Credible, maintained BSD-3-Clause protocol library (about 11.3M downloads) with excellent low-level composition and a stub resolver. Rejected because its server/client transport features are explicitly named `unstable-server-transport`/`unstable-client-transport` in the tagged [manifest](https://github.com/NLnetLabs/domain/blob/v0.12.2/Cargo.toml), while Hickory's stable protocol surface is far more adopted and passed the exact differential with less bespoke composition. |
| `resolv-conf 0.7.6` | Credible, maintained parser, but rejected for this exact seam after adversarial differential review. It strips inline `#`/`;`, diagnoses extra tokens, and restricts scoped-zone syntax, while miekg takes the literal second whitespace token, ignores later tokens, and lets `netip.ParseAddr` accept arbitrary nonempty zones. The exact required subset is a bounded line/whitespace/token adapter plus standard IP parsing; a general resolver-config dependency would require more compensation than it removes. |
| `dns-message-parser 0.9.0` and small packet crates | Rejected on maturity/adoption and incomplete runtime ecosystem: about 112k downloads and six reverse dependencies at the snapshot, versus Hickory's maintained security process, fuzzing, record coverage, and adoption. They would increase package-owned parser/serializer/security responsibility without improving parity. |
| Full custom wire codec on Tokio | Hard reject. DNS compression parsing, malformed-packet handling, EDNS/unknown records, safe truncating emission and response forwarding are attacker-facing protocol machinery already supplied by the maintained selected crate. |

## Selected integration

Use the exact stack:

```toml
hickory-proto = { version = "=0.26.1", default-features = false, features = ["std"] }
tokio = { version = "=1.53.1", default-features = false, features = ["io-util", "macros", "net", "rt", "sync", "time"] }
tokio-util = { version = "=0.7.19", default-features = false }
fastrand = { version = "=2.5.0", default-features = false, features = ["std"] }
nix = { version = "=0.31.3", default-features = false, features = ["net"] }
```

`tokio`, `tokio-util`, and `fastrand` are already approved workspace choices.
Do not add `hickory-server`, `hickory-resolver`, `hickory-client`, `hickory-net`,
DNSSEC, TLS, DoH, DoQ, mDNS, serde, or access-control features. `nix` feature
`net` already includes `socket`; do not list both.

Follow these package-owned seams:

1. Bind the UDP socket and surface its failure. Independently attempt the TCP
   bind on the same address; log and continue on any TCP failure. Own separate
   supervised accept loops so later UDP failure is fatal and later TCP failure
   is nonfatal. Use a 512-byte UDP receive buffer to retain the oracle limit;
   Tokio TCP uses the two-byte DNS length prefix, maximum 65,535-byte frame,
   two-second first-read, eight-second later-read, and 128-query connection cap.
2. On cancellation close/stop the listeners and idle connections, but retain
   and join every accepted request task. Do not propagate shutdown cancellation
   into an admitted forward: the Go server waits for those operations, which
   can take three seconds per remaining upstream. Apply a bounded outer shutdown
   guard only at the owning application level if already authorized; silently
   aborting handlers is not parity.
3. Decode the 12-byte `Header` first and apply the exact admission matrix above
   before `Message::from_vec`: ignore QR responses; `NOTIMP` non-QUERY/NOTIFY;
   `FORMERR` QDCOUNT != 1, ANCOUNT > 1, NSCOUNT > 1, or ARCOUNT > 2. A body
   decode error after an accepted header is also `FORMERR`. For accepted
   headers, decode and retain the first question separately before full-message
   decode so a later body failure can preserve it; clear Answer, Authority, and
   Additional. Count rejection and missing/malformed-question errors have no
   sections. Apply the exact retained/cleared flags above. Build application
   replies by copying only ID, opcode and the first question; copy RD/CD only
   for opcode QUERY, while accepted NOTIFY clears them. Do not echo EDNS into
   an internal response.
4. For internal truncation, first emit with
   `NameEncoding::Uncompressed`; if that fits, send it uncompressed. Otherwise
   emit with normal compression and `BinEncoder::set_max_size`, then reparse and
   normally re-emit the bounded message. This exactly matched miekg's 90-A
   oracle fixture: UDP `510 bytes/30 answers`, EDNS-1232 `1230/75`, TCP
   `2550/90`. Do not use `Message::truncate()`, which drops all response records.
5. Preserve `Option<Vec<SocketAddr>>`: `None` means system config and
   `Some(empty)` means forwarding disabled. Stream `/etc/resolv.conf` with
   `bufio.Scanner`'s 64 KiB line-token ceiling (stop on overflow but retain
   earlier entries) and use a package-owned line adapter matching Go
   `strings.Fields`: lossily decode each bounded line, take only the second
   token of exact `nameserver` lines, ignore later tokens (including invalid
   bytes), and do not strip inline comments from that token. Split an IPv6
   `%zone` at the first percent and require a nonempty arbitrary zone, retaining
   any later percent in the zone. Drop
   only zone-aware exact matches to the listener address. Call
   `if_nametoindex` first; if no interface has that exact name, reproduce Go's
   `dtoi`: consume only the leading decimal prefix, use zero when there is none,
   and cap at `0xFFFFFF`, immediately before constructing `SocketAddrV6`. Fall
   back to Cloudflare then Google only when no usable system server remains.
6. Acquire an owned `tokio::sync::Semaphore(1024)` permit with
   `try_acquire_owned` before any forward I/O. Failure immediately maps to
   `SERVFAIL`. For each upstream in order, use one three-second timeout for
   bind/connect and then a fresh three-second deadline shared by write and read.
   UDP uses a connected family-matching ephemeral socket and a receive buffer
   of 512 or the request's advertised EDNS size (minimum 512); loop past
   parseable mismatched IDs until deadline. TCP uses connect, big-endian `u16`
   length, exact request bytes, then exact framed response; a mismatched ID is
   an attempt failure. Fall back on timeout, framing, parse, or ID failure and
   accept the first matching transport-success response without RCODE policy.
7. Use `fastrand::shuffle` once for every multi-IP answer, followed for
   `nearest` by stable local-subnet-first partitioning. Do not sort either
   partition. For an unexpected true IPv6 resolver address, emit A RDATA
   `0.0.0.0`: miekg calls `To4()`, obtains nil, and copies zero bytes into the
   zero-initialized four-byte A slot. This oddity is characterized, not left to
   implementor choice.

## Known limitations and required package tests

- The selected dependencies deliberately do not implement the combined server
  policy. The listener, drain, forwarding, overload, and fallback seams above
  are required parity code, not optional enhancements.
- Hickory typed parse/re-emit may normalize legal wire encodings (compression
  pointers, case-preserving names, OPT representation). The oracle also parses
  and re-emits through miekg; tests must compare observable decoded messages and
  the specifically differential-traced truncation wire, not require arbitrary
  request byte identity.
- A 512-byte UDP receive buffer is an oracle limitation and can silently discard
  larger client datagrams. Preserve it for baseline parity even though modern
  EDNS clients may send larger requests.
- The server has no DNSSEC validation, cache, DNS-over-TLS/HTTPS/QUIC, response
  authenticity, rate limit for internal requests, or per-client quota. The
  global 1,024 cap only bounds external forwards; these are oracle limitations.
- Sequential timeout is per upstream and phase: UDP gets up to three seconds
  to bind/connect and then three seconds for write/read; TCP gets up to three
  seconds to connect and then three seconds for write/read. Total latency can
  therefore approach `6 seconds × upstream count`. This is oracle behavior,
  not an accepted deviation. Cancellation waits for admitted forwards.
- Native macOS x86_64/aarch64 tests must cover scoped interfaces, UDP/TCP port
  53 binding behavior, cancellation/drain, and socket cleanup; cross-checking
  those targets on Linux proves compilation only.
- Before package acceptance, differential tests must cover unreadable versus
  malformed headers, zero/multiple questions, mixed-case/subdomain boundaries,
  A/NXDOMAIN/unsupported types, `nearest` stability, EDNS below/at/above 512,
  truncation, explicit-empty versus absent upstreams, self-only/invalid/scoped
  resolv.conf, both transports, mismatched response IDs, 512/EDNS upstream UDP
  receive sizing, sequential fallback/RCODE acceptance, independent dial and
  I/O timeout phases, 1,024+1 admission, UDP-fatal/TCP-nonfatal start and
  runtime failure, 128 TCP requests, and graceful shutdown with an in-flight
  forward.

## Verification

The Rust 1.96 probe lived outside the repository at
`/tmp/ployz-dns-probe-primary` and left no repository artifact. Its twelve tests
compiled the exact selected feature graph and exercised:

- the complete header admission count/opcode/QR matrix, retained `FORMERR`
  flags, partial-question retention after a later body error, and QUERY versus
  NOTIFY RD/CD reply behavior;
- exact nameserver token behavior, punctuation-bearing and numeric IPv6 zones,
  and zone-aware self comparison;
- miekg's unexpected IPv6-in-A result (`0.0.0.0`);
- UDP mismatched-ID looping, RCODE acceptance, 512/EDNS receive sizing,
  sequential fallback and exhaustion; TCP mismatched-ID fallback and distinct
  dial/I/O timeout phases;
- 1,024 simultaneously admitted operations plus an immediately rejected
  1,025th operation;
- actual occupied UDP and TCP bind outcomes, and supervised UDP-fatal versus
  TCP-nonfatal runtime results;
- the 128-query TCP connection cap; and
- a real listener that stopped accepting on cancellation but awaited an
  admitted three-second operation before returning.

The probe binary also constructed internal FORMERR/NXDOMAIN/unsupported
responses, exercised both transports, and matched bounded truncation.

A copied-oracle differential lived outside the repository at
`/tmp/ployz-go-dns-oracle`. Six Go 1.26.1 tests characterized the admission
matrix/flags, IPv6-in-A result, UDP/TCP ID checks and UDP sizing preconditions,
partial-question/NOTIFY reply behavior, resolver token/zone behavior, and
internal response/truncation behavior. Go
reported `UDP 510 bytes/30 answers`, `EDNS-1232 1230/75`, and `TCP 2550/90`;
the corrected Hickory bounded-emission seam reported the same values.

Commands passed:

```sh
cargo +1.96.0 fmt --manifest-path /tmp/ployz-dns-probe-primary/Cargo.toml --check
cargo +1.96.0 test --manifest-path /tmp/ployz-dns-probe-primary/Cargo.toml --all-targets
cargo +1.96.0 clippy --manifest-path /tmp/ployz-dns-probe-primary/Cargo.toml --all-targets -- -D warnings
cargo +1.96.0 run --manifest-path /tmp/ployz-dns-probe-primary/Cargo.toml
cargo audit --file /tmp/ployz-dns-probe-primary/Cargo.lock --deny warnings
cargo +1.96.0 check --manifest-path /tmp/ployz-dns-probe-primary/Cargo.toml --all-targets --target x86_64-unknown-linux-gnu
cargo +1.96.0 check --manifest-path /tmp/ployz-dns-probe-primary/Cargo.toml --all-targets --target aarch64-unknown-linux-gnu
cargo +1.96.0 check --manifest-path /tmp/ployz-dns-probe-primary/Cargo.toml --all-targets --target x86_64-apple-darwin
cargo +1.96.0 check --manifest-path /tmp/ployz-dns-probe-primary/Cargo.toml --all-targets --target aarch64-apple-darwin
/home/codex/.local/share/mise/installs/go/1.26.1/bin/go test -count=1 -run 'Test(OracleParityProbe|AdversarialOracle)' -v ./internal/machine/dns
```

## Review

The first fresh read-only adversarial review returned **not clean** with six
findings: missing header admission/flag parity; missing response-ID and upstream
UDP-size rules; inadequate hard-gate probes; resolver-parser/scoped-zone
divergence; an incorrect unresolved IPv6-in-A description; and an incorrectly
collapsed three-second timeout. All six were fixed by specifying the exact
package seams above, rejecting `resolv-conf`, and adding the Rust/Go probes
listed in Verification. A different fresh read-only adversarial reviewer then
re-reviewed those fixes and also returned **not clean** after finding a
temporary-`Cow` ownership compile error in the lossy parser probe, the
unnecessary `tokio-util/rt` feature, and two remaining scoped-zone differences
(first-percent splitting and Go's name-first/decimal-prefix/`0xFFFFFF` index
fallback). Those were fixed in the probe, selected feature set, resolver seam,
and Go characterization. An entirely fresh third read-only reviewer also
returned **not clean**, finding that a later malformed section retains an already
decoded question in `FORMERR`, and that `SetReply` copies RD/CD only for QUERY,
not accepted NOTIFY. Both were fixed in the contract, package seam, and Rust/Go
characterization above. A fourth entirely fresh read-only reviewer then
returned **CLEAN**: it independently confirmed exact
base `ceb86fd1d34e9be60bc994307fedb7b1ef63611b`, sole-file scope, oracle/miekg
and caller coverage, exact minimal features/licenses/MSRVs, all twelve Rust
probes, six Go differentials, RustSec audit, Clippy, and all four target checks,
including partial-question `FORMERR` and NOTIFY RD/CD behavior. Routine approval
is therefore effective. No approval registry or authority file was edited.

Affected package: `internal/machine/dns`.
