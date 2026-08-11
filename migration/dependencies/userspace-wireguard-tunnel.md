# Dependency decision: userspace WireGuard tunnel

| Field | Value |
| --- | --- |
| Status | `blocked` |
| Selected dependency | None |
| Leading conditional stack | `boringtun = "=0.7.1"` plus `smoltcp = "=0.13.1"` with the exact features below |
| Candidate licenses | `BSD-3-Clause` plus `0BSD` |
| Research date | 2026-08-11 UTC |
| Request | Direct controller delegation; no dependency-request file exists at base `da46a6d` |

No published Rust dependency presently clears the behavior, security,
maintenance, and shipped-platform hard gates. The only drop-in API match,
`tokio-wireguard 0.1.3`, has a definite lost-wakeup bug on TCP writes, a
live-socket replacement bug, and an actively vulnerable pinned cryptographic
dependency. The current protocol and TCP/IP primitives are plausible, but
approving them would silently assign a substantial, security-sensitive network
driver to the package implementor without an interoperation or lifecycle proof.

The controller must keep `internal/machine/network/tunnel` at the dependency
gate. A successor may approve the conditional stack only after the focused
prototype and platform evidence under [Unblock conditions](#unblock-conditions)
pass, or after an independently maintained drop-in release fixes the findings.

## Capability and oracle contract

The behavioral oracle is
[`upstream/uncloud/internal/machine/network/tunnel/tunnel.go`](../../upstream/uncloud/internal/machine/network/tunnel/tunnel.go),
and its direct caller is
[`upstream/uncloud/pkg/client/connector/wireguard.go`](../../upstream/uncloud/pkg/client/connector/wireguard.go).
There are no upstream package tests. The dependency must therefore be checked
against source, caller, and live WireGuard interoperation.

The required implementation must:

- run an unprivileged, in-process WireGuard client and userspace TCP/IP stack;
- accept a local IPv4 or IPv6 address and private key, one UDP endpoint and
  remote public key, one allowed remote prefix, an MTU, and a persistent
  keepalive interval;
- expose the caller-used endpoint-port constant 51820 and pass the supplied
  endpoint through unchanged; default MTU to the engine's WireGuard default,
  keepalive to 25 seconds, and DNS to `1.1.1.1` when omitted;
- bring the tunnel up during construction, return contextual construction,
  configuration, and activation errors, and allow idempotent close;
- establish cancellable TCP connections through the userspace stack and return
  a bidirectional stream suitable for tonic/h2 backpressure and shutdown;
- support simultaneous TCP connections even though the current direct caller
  normally creates one gRPC HTTP/2 connection; and
- work for the shipped `uc` matrix: Linux and macOS, amd64 and arm64. Windows
  is not currently shipped; see the oracle
  [GoReleaser matrix](../../upstream/uncloud/.goreleaser.yaml).

There is a scope ambiguity that this record cannot waive. The only checked-in
caller passes numeric IPv6 and `"tcp"`, but the oracle method forwards its
public `network` and `address` arguments to WireGuard Go netstack. The exact
upstream implementation supports TCP, UDP, and ping, `4`/`6` suffixes,
in-tunnel DNS with multiple-address attempts, and caller-context-derived
deadlines. A missing package packet/dependency request means no authority has
narrowed the port contract to the present TCP caller. [Exact Go dependency
source](https://github.com/WireGuard/wireguard-go/blob/12269c276173/tun/netstack/tun.go#L955-L1064).

The Go implementation uses WireGuard's maintained Go protocol engine and
gVisor netstack via `netstack.CreateNetTUN`, then returns that stack's
`DialContext`. It does not create a kernel TUN interface, require root, start an
external executable, or alter host routes. Kernel-interface controllers and
ordinary userspace TUN executables therefore do not satisfy this capability.

## Primary-source evidence

### Oracle

- [Frozen tunnel source](../../upstream/uncloud/internal/machine/network/tunnel/tunnel.go)
- [Frozen direct connector](../../upstream/uncloud/pkg/client/connector/wireguard.go)
- [Frozen WireGuard dependency versions](../../upstream/uncloud/go.mod)
- [Frozen release target matrix](../../upstream/uncloud/.goreleaser.yaml)

### Leading primitives

- [BoringTun 0.7.1 manifest](https://docs.rs/crate/boringtun/0.7.1/source/boringtun/Cargo.toml.orig)
- [BoringTun 0.7.1 `Tunn` packet/timer API](https://docs.rs/boringtun/0.7.1/boringtun/noise/struct.Tunn.html)
- [BoringTun commit `051c9d4` support matrix](https://github.com/cloudflare/boringtun/tree/051c9d47dc9c5cb36e461b7d36dcd673820dc98b#supported-platforms)
- [smoltcp 0.13.1 manifest and features](https://docs.rs/crate/smoltcp/0.13.1/source/Cargo.toml)
- [smoltcp TCP feature/limitation matrix](https://github.com/smoltcp-rs/smoltcp/tree/v0.13.1#tcp-layer)
- [Official crates.io metadata: BoringTun](https://crates.io/api/v1/crates/boringtun)
- [Official crates.io metadata: smoltcp](https://crates.io/api/v1/crates/smoltcp)

### Rejected drop-in and alternatives

- [`tokio-wireguard 0.1.3` API](https://docs.rs/tokio-wireguard/0.1.3/tokio_wireguard/)
- [`tokio-wireguard 0.1.3` exact manifest](https://docs.rs/crate/tokio-wireguard/0.1.3/source/Cargo.toml.orig)
- [Exact released lost-wakeup source](https://github.com/raftario/river/blob/4e71d82c4dac3989cd2a4b814144e3ae3dda6613/crates/tokio-wireguard/src/io.rs#L123-L136)
- [Open live-socket replacement defect](https://github.com/raftario/river/issues/3)
- [Open patch explaining concurrent TCP hangs](https://github.com/raftario/river/pull/5)
- [RustSec timing-variability advisory `RUSTSEC-2024-0344`](https://rustsec.org/advisories/RUSTSEC-2024-0344.html)
- [RustSec unmaintained ring 0.16 advisory `RUSTSEC-2025-0010`](https://rustsec.org/advisories/RUSTSEC-2025-0010.html)
- [`wireguard-netstack 0.3.0` manifest](https://docs.rs/crate/wireguard-netstack/0.3.0/source/Cargo.toml.orig)
- [`wireguard-netstack 0.3.0` IPv4-only netstack source](https://docs.rs/crate/wireguard-netstack/0.3.0/source/src/netstack.rs)
- [`onetun 0.3.10` architecture and port-forward API](https://docs.rs/crate/onetun/0.3.10/source/README.md)
- [`wiretun 0.5.0` production warning and native-interface API](https://docs.rs/crate/wiretun/0.5.0/source/README.md)
- [NepTUN library/CLI distinction and support matrix](https://github.com/NordSecurity/NepTUN)
- [`wireguard-sans-io 1.0.0` embedding obligations](https://docs.rs/crate/wireguard-sans-io/1.0.0/source/README.md)
- [Official incomplete/insecure Rust reference implementation notice](https://git.zx2c4.com/wireguard-rs/about/)

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | In-process, rootless WireGuard plus IPv4/IPv6 userspace IP stack; at minimum cancellable, backpressured, concurrent TCP streams; exact endpoint/key/prefix/MTU/keepalive lifecycle; resolve the broader `DialContext` scope | `tokio-wireguard` exposes the nearest natural model, but exact 0.1.3 can lose the TCP write waker and replace a live socket slot, hanging active streams. It also imposes a fixed 10-second TCP connect timeout rather than deriving only from caller context. BoringTun + smoltcp expose sufficient protocol/IP primitives, but no verified driver yet composes UDP, timers, packet queues, routing, DNS, cancellation, task failure, and close. | **fail** |
| License and security | Apache-2.0-compatible permissive dependencies; no known unmitigated crypto advisory | The conditional pair is BSD-3-Clause + 0BSD and its 90-package probe lock had only permissive/compatible expressions. RustSec at DB commit `d0861df1eab469d3c58d6b836ce48b5766e5f217` found no advisory in that pair's graph. `tokio-wireguard 0.1.3`, however, pins BoringTun 0.6.0, x25519-dalek 2.0.0-rc.3, curve25519-dalek 4.0.0-rc.3, and ring 0.16.20; the curve version is affected by `RUSTSEC-2024-0344`, and ring 0.16 is unmaintained. `wireguard-netstack` is GPL-3.0. | **fail for every drop-in; provisional pass for the unapproved pair's resolved graph** |
| Platforms and targets | Linux/macOS amd64 and arm64, no privileged host networking | `tokio-wireguard` claims any Tokio platform but has no current native matrix proof. BoringTun 0.7.1's official matrix lists macOS x86_64, not Apple arm64. The conditional pair compiled on Rust 1.96 for Linux x86_64 and Windows GNU; Apple and Linux-arm cross-checks were blocked by absent target C compilers/SDK, not by established support. No native macOS or arm64 interop ran. | **incomplete / fail** |
| Maintenance and Rust version | Maintained current dependencies compatible with Rust 1.96 | BoringTun 0.7.1 was published 2026-05-01 and smoltcp 0.13.1 on 2026-04-30; smoltcp declares MSRV 1.91 and the pair compiled under 1.96. `tokio-wireguard 0.1.3` declares 1.75 but was last published 2024-08-04, and its 2025 repository change did not fix the known defects or publish a release. | **pass only for the conditional primitives; fail for the drop-in** |
| Architectural constraints | Dependency API may shape the Rust crate, but dependency research may not assign an unreviewed custom cryptographic/network transport | Every safe high-level/control candidate uses a host interface or lacks a TCP/IP stack. Direct BoringTun + smoltcp requires a package-owned async driver and stream abstraction of material complexity; a build-only API probe is not behavioral or security evidence. | **fail pending prototype** |

Overall verdict: **blocked**.

## Candidate comparison

Official crates.io and repository metadata were checked on 2026-08-11.
Downloads and dependents are adoption signals, not security guarantees.

| Candidate | Behavior/platform/security | Adoption and maintenance | Disposition |
| --- | --- | --- | --- |
| `tokio-wireguard = "=0.1.3"` | Exact natural API: rootless in-process BoringTun + dual-stack smoltcp with Tokio `TcpStream`. Exact release has TCP lost-wakeup and active-socket replacement bugs. Its pinned crypto graph contains vulnerable curve25519-dalek and unmaintained ring 0.16. | 5,121 total / 451 recent downloads; 1 reverse dependent; 28 repository stars; last release 2024-08-04. | **Rejected at behavior, security, and maintenance gates.** A git patch or fork would be a new dependency and security decision. |
| `boringtun = "=0.7.1"` + `smoltcp = "=0.13.1"` | Current WireGuard engine plus dual-stack TCP/IP primitives. Requires a nontrivial driver; no end-to-end WireGuard/TCP/cancellation/close proof exists. BoringTun's documented Apple matrix omits arm64. | BoringTun: 546,250 total / 265,199 recent downloads, 17 reverse dependents, 7,162 stars. smoltcp: 7,278,917 / 1,358,202, 93 reverse dependents, 4,565 stars. Both released in 2026 and active. | **Leading conditional stack, not approved.** Best ecosystem/maintenance choice after a successful focused prototype and review. |
| `wireguard-netstack = "=0.3.0"` | Turnkey TCP connection API, but its manifest enables only smoltcp IPv4 and its implementation uses IPv4 types. It cannot address the oracle's IPv6 management `/128`. It also bundles unrelated DoH/TLS policy. | 792 total / 275 recent downloads; 3 reverse dependents; released 2026-02-23; repository has 5 stars. | **Rejected at behavior and license gates:** GPL-3.0 and IPv4-only. |
| `onetun = "=0.3.10"` | Demonstrates rootless BoringTun + dual-stack smoltcp, but exposes static local port forwarding rather than an owned cancellable virtual TCP stream. It carries older BoringTun/smoltcp versions and binary-oriented machinery. | 13,407 total / 218 recent downloads; no reverse dependents; last release 2024-12-01; 1,041 stars. | **Rejected:** wrong API/lifecycle and no advantage over current primitives. |
| `wiretun = "=0.5.0"` / NepTUN 1.0.8 | Protocol implementations backed by OS TUN interfaces; not an in-process TCP/IP stack. WireTun explicitly says it is early and not production-ready. NepTUN's library explicitly omits network and tunnel stacks. | WireTun: 10,727 total / 52 recent downloads, no dependents, last release 2023-09-18. NepTUN has one 2025 release and 16 stars and is not a crates.io library at that version. | **Rejected:** privileges/host interface or missing netstack, plus weak readiness/adoption. |
| `wireguard-sans-io = "=1.0.0"` + `wireguard-embed = "=1.0.0"` | Promising panic-free/no-unsafe protocol core, but the embedder must still supply packet queueing, rate limits, UDP, timers, and all TCP/IP behavior. | First published 2026-06-25; 76 and 24 total downloads respectively; 2 and 0 reverse dependents; repository had no stars at research time. | **Rejected for this decision:** too new and unadopted for a critical crypto dataplane, with more integration burden than BoringTun. |
| Kernel/control crates (`defguard_wireguard_rs`, `wireguard-uapi`, `wireguard-control`) and official `wireguard-rs` | Control existing host interfaces or wrap native code; they do not provide the required rootless in-process netstack. The official Rust reference declares itself incomplete and insecure. | Some are popular as interface controllers, which is irrelevant to this capability. | **Rejected at behavior/architecture gate.** |

## Conditional integration to prove

This is a prototype target, **not an approved dependency line**:

```toml
boringtun = { version = "=0.7.1", default-features = false }
smoltcp = { version = "=0.13.1", default-features = false, features = [
  "std",
  "medium-ip",
  "proto-ipv4",
  "proto-ipv6",
  "proto-dns",
  "socket-tcp",
  "socket-udp",
  "socket-icmp",
  "socket-dns",
  "async",
] }
```

The UDP/ICMP features preserve the unresolved broad `DialContext` option; a
controller-authorized TCP-only contract may omit them. Do not enable
BoringTun's `device`, FFI, or JNI features: they introduce host
TUN/interface behavior that is outside the oracle. Use its natural
`Tunn::{encapsulate,decapsulate,update_timers}` result loop and smoltcp's
`Medium::Ip`, `Interface`, `SocketSet`, TCP socket, and DNS socket. The async
runtime remains a separately gated capability; this record does not approve a
runtime crate.

The driver must bound buffers so BoringTun's documented too-small-destination
panic is unreachable, drain every `WriteToNetwork` continuation, preserve
queued plaintext across the initial handshake, poll WireGuard timers for
rekey/keepalive/retransmission, validate decrypted source/destination against
the configured prefix, propagate UDP/task failure, and wake read and write
waiters independently. Close must cancel and join all driver work and wake
pending operations. It must not copy `tokio-wireguard`'s affected slot/waker
logic.

smoltcp's documented TCP limitations include no selective acknowledgements,
timestamps, or packetization-layer path-MTU discovery. A successor must decide
whether these alter required deployed behavior and test at the configured MTU,
loss, reordering, half-close, and backpressure boundaries.

## Verification performed

An isolated Rust 1.96 probe used exactly the conditional manifest above. It
constructed a BoringTun tunnel, exercised its handshake-producing encapsulation
path, constructed a dual-stack `Medium::Ip` smoltcp interface, installed TCP and
DNS sockets, and polled the stack. UDP/ICMP features compiled but their socket
behavior was not exercised; that remains part of the broader-scope successor
proof.

```text
cargo +1.96.0 run --locked --offline
  pass: Linux x86_64
cargo +1.96.0 check --locked --offline --all-targets
  pass: Linux x86_64
cargo +1.96.0 clippy --locked --offline --all-targets -- -D warnings
  pass: Linux x86_64
cargo +1.96.0 check --locked --offline --target x86_64-pc-windows-gnu
  pass (unshipped reference target)
cargo audit --no-fetch --file Cargo.lock
  pass: 1,211-advisory DB at d0861df, 90-package resolved lock
```

Linux arm64 and Intel macOS cross-checks stopped because `ring 0.17.14` needs
target C compilers/SDKs absent from this VM. Those are environment gaps, not
candidate failures and not platform passes. No native UDP/WireGuard peer or
TCP exchange was run, so the probe deliberately does not pass the behavior or
platform gates.

## Unblock conditions

A fresh successor decision must provide all of the following:

1. A minimal driver using the exact conditional stack and an already approved
   runtime, with no copied `tokio-wireguard` slot/waker implementation.
2. A controller-owned behavior decision that either preserves the delegated Go
   netstack networks (`tcp`, `udp`, `ping`, family suffixes, DNS, and contextual
   deadlines) or explicitly narrows the internal API to observed TCP callers.
3. Live interoperation against the frozen Go WireGuard/netstack or an official
   WireGuard peer for IPv6 `/128`: handshake, 25-second keepalive, rekey,
   retransmission, endpoint failure, and exact MTU boundaries.
4. Concurrent TCP connections covering connect cancellation, DNS and literal
   IPv4/IPv6 addresses, sustained full-buffer backpressure, independent
   read/write wakeups, half-close, remote reset, packet loss/reordering, and
   tunnel close while calls are pending.
5. Bounded queue/buffer and malformed-datagram tests proving no panic, unbounded
   allocation, silent driver death, stale stream, or task leak.
6. Native Linux and macOS amd64/arm64 build and runtime results, or an explicit
   platform exception. In particular, establish BoringTun Apple-arm64 support
   rather than inferring it from an x86_64-only README row.
7. A fresh RustSec scan, complete target-inclusive license check, exact lock
   review, and fresh adversarial security/behavior review of the resulting
   driver commit.

## Review

This networking/cryptography decision received a fresh read-only adversarial
review. The reviewer targeted frozen oracle base `da46a6d`, exact published
`tokio-wireguard 0.1.3` source commit
`4e71d82c4dac3989cd2a4b814144e3ae3dda6613`, BoringTun 0.7.1 source commit
`051c9d47dc9c5cb36e461b7d36dcd673820dc98b`, and the candidate manifests above.

Reviewer result: **BLOCK**. It independently reproduced the API fit but found
the TCP write lost-wakeup, active-socket replacement/concurrent-hang defect,
silently discarded dataplane errors, pinned vulnerable crypto graph, stale
release, and missing Apple-arm64 proof. Those findings are reflected in the
hard gates and unblock conditions. No unresolved reviewer disagreement remains.

Affected package: `internal/machine/network/tunnel` (migration crate
`crates/ployz-internal-machine-network-tunnel`), with direct downstream caller
`pkg/client/connector`.
