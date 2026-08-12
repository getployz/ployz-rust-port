# Dependency decision: `linux-iptables-firewalld-rule-management`

| Field | Value |
| --- | --- |
| Status | `approved` |
| Capability | Synchronous Linux IPv4/IPv6 iptables rule and chain management, including firewalld direct-passthrough parity |
| Selected dependency | **Rust 1.96 standard library** (`std::process`, `std::os::fd`, `std::os::unix::net`, and a small package-owned typed command runner) plus **`zbus = 5.19.0`** |
| License | Rust standard library: `MIT OR Apache-2.0`; zbus: `MIT` |
| Research date | `2026-08-12` UTC |
| Request | Controller delegation for `linux-iptables-firewalld-rule-management`; no request file exists |
| Affected packages | `internal/machine/firewall`, `internal/machine/docker` |
| Blockers | None for the required Linux behavior. The mandated firewalld direct interface is deprecated; its future removal is a documented lifecycle limit below. |

## Verdict

Do not add an iptables/nftables rule-model crate. The behavioral API is the
installed `iptables`/`ip6tables` command language itself, and firewalld exposes
that same argv as a D-Bus `as` through
`org.fedoraproject.FirewallD1.direct.passthrough`. Use a narrow Rust firewall
executor with typed `IpFamily`, `Table`, `Chain`, `Action`, and rule-argument
values, backed by:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
zbus = { version = "=5.19.0", default-features = false, features = ["async-io", "blocking-api"] }
```

There is no process-execution crate. Rust 1.96 `std::process::Command` accepts
the program and each argument separately and searches `PATH` for a relative
program; on Unix the child receives an argv array rather than a shell command
line ([Rust `Command` documentation](https://doc.rust-lang.org/1.96.0/std/process/struct.Command.html),
[process module security/platform notes](https://doc.rust-lang.org/1.96.0/std/process/index.html)).
The executor must never invoke `sh -c` or concatenate a command for execution.

The standard library can also preserve Go `CombinedOutput` semantics without
unsafe code or another crate: create one `UnixStream::pair`, convert two clones
of the writer to `OwnedFd`/`Stdio` for the child's stdout and stderr, drain the
reader concurrently, wait for the child, and retain both the merged bytes and
`ExitStatus`. The exact probe below compiles and observes
`stdout-1\nstderr-1\nstdout-2\n` with exit status 23. This package-owned runner
is a deep, small boundary around the
privileged transport; it is not a Go-shaped public API.

zbus is the natural Rust D-Bus client. Its blocking `Connection::system` and
`Proxy::call` APIs match the capability's synchronous sequencing
([exact 5.19.0 connection source](https://github.com/z-galaxy/zbus/blob/zbus-5.19.0/zbus/src/blocking/connection/mod.rs),
[blocking API](https://docs.rs/zbus/5.19.0/zbus/blocking/),
[proxy API](https://docs.rs/zbus/5.19.0/zbus/blocking/struct.Proxy.html)).
The selected `async-io` backend lets that blocking API drive D-Bus without
requiring an application Tokio runtime; `blocking-api` is the only public API
feature needed. Keep this executor synchronous and out of an async task. An
async caller must run the whole ordered firewall transaction in its blocking
startup/cleanup context and wait for completion before proceeding; it must not
spawn individual rule operations concurrently.

## Primary-source contract

The frozen package oracle is
[`iptables_linux.go`](../../upstream/uncloud/internal/machine/firewall/iptables_linux.go),
with the unsupported macOS behavior in
[`iptables_darwin.go`](../../upstream/uncloud/internal/machine/firewall/iptables_darwin.go).
The second consumer is
[`controller_linux.go`](../../upstream/uncloud/internal/machine/docker/controller_linux.go).
The imported dependency is pinned by the oracle to Docker 28.5.0, and the
relevant exact sources are Docker's
[`iptables.go`](https://github.com/moby/moby/blob/v28.5.0/libnetwork/iptables/iptables.go#L228-L381)
and
[`firewalld.go`](https://github.com/moby/moby/blob/v28.5.0/libnetwork/iptables/firewalld.go#L71-L310).
The local module-cache copies matched the Go checksum-selected version; their
SHA-256 values during research were:

```text
5bb9e541fb4266ff03b62f78505f3976fca4b4a876ccf4fc006d76ddc7929a37  Docker 28.5.0 libnetwork/iptables/iptables.go
84079ab78765d6843cb19e99792f5e15e8f0ed1c207ac14c1aea5fc759dfdfc3  Docker 28.5.0 libnetwork/iptables/firewalld.go
cc86438530283a3eb8d26343f56f8523df26d7af6ef8b0434ac743294db9d2ae  oracle firewall/iptables_linux.go
3e8498c81c3ff9034c29fa786eedbe4a854a71d69f6813e0d210d0762143236b  oracle firewall/iptables_darwin.go
f032fc91734621d675a19de2862ea36def3cb6fc5761ef3a5148267e6f7305c2  oracle docker/controller_linux.go
```

The firewalld project documents `/org/fedoraproject/FirewallD1`, the base
interface, and the deprecated direct interface. `passthrough(s ipv, as args) ->
s` accepts `ipv4`, `ipv6`, or `eb` and executes untracked direct arguments
([official firewalld D-Bus manual](https://firewalld.org/documentation/man-pages/firewalld.dbus.html#FirewallD1.direct)).
The same manual explicitly says the direct interface is deprecated and will be
removed in a future release. Its configuration-interface sections also define
the reachable `getZones() -> as`, deprecated
`addZone(s, (sssbsasa(ss)asba(ssss)asasasasa(ss)b)) -> o`,
`addPolicy(s, a{sv}) -> o`, and `reload() -> Nothing` calls. They remain
mandatory here because policies or native nftables do not preserve the oracle's
iptables argv, output, error, and rule-ordering contract, while omitting the
Docker zone/policy initialization would lose a separate reachable host mutation.

Netfilter documents `-w/--wait` as waiting indefinitely for the exclusive
xtables lock and `--line-numbers` as prefixing listed rules with their chain
positions
([official iptables manual](https://ipset.netfilter.org/iptables.man.html)).
The production runtime therefore requires `iptables` and `ip6tables` versions
that support both `--wait` and `-C`; the selected code does not silently fall
back to a racy read/list emulation on old versions.

## Authorized Ployz rename

Apply the project product-name substitution at the durable host-firewall
boundary:

| Frozen oracle | Rust port |
| --- | --- |
| `UNCLOUD-INPUT` | `PLOYZ-INPUT` |
| comment `Uncloud-managed` | comment `Ployz-managed` |
| `DOCKER-USER` | unchanged protocol/external chain name |

Do not create aliases and do not mutate or clean the old `UNCLOUD-INPUT` chain.
Supporting both namespaces would be a new migration feature and could attach
two INPUT jumps. The remaining iptables table, built-in chain, target, interface,
and option spellings are external protocol and remain unchanged.

## Required behavioral mapping

### Backend and transport

| Required behavior | Selected design and exact semantics |
| --- | --- |
| IPv4 and IPv6 backends | A typed family chooses the separately resolved `iptables` or `ip6tables` executable and maps to firewalld `ipv4` or `ipv6`. A missing IPv4 executable fails initialization; a missing IPv6 executable fails the first requested IPv6 operation. Resolve the executable once to an absolute path and execute that path with argv. Never interpolate a shell string. |
| xtables serialization | Prepend `--wait` to every raw executable argv. This is what Docker 28.5.0 `raw` does. Direct D-Bus calls pass the oracle argv unchanged; firewalld owns its backend serialization. Do not append a timeout or retry loop. |
| combined output | Feed stdout and stderr to one concurrently drained stream and preserve the resulting bytes in arrival order. On spawn/I/O failure preserve the `io::Error`; on child completion preserve `ExitStatus` even when nonzero. A nonzero exit produces an operation error containing executable/family, full argv, merged bytes, and status. |
| `RawCombinedOutput` behavior | For mutation paths, either a transport error/nonzero exit **or any nonempty successful combined output** is an error. A successful empty output is success. Do not discard output merely because status is zero. |
| known lock warning | Preserve Docker's asymmetric filter point. A **successful raw process** or a **returned D-Bus result** whose output contains the exact substring `Another app is currently holding the xtables lock` has its **whole output** replaced with empty bytes; do not delete only one line. A nonzero raw process returns before filtering, so its merged error output is not suppressed. Preserve any D-Bus error independently of output filtering. |
| slow operation warning | Preserve Docker's asymmetric timer/filter control flow exactly. For a raw process, start the timer immediately before spawn but call the `> 2s` filter only after status success; a slow nonzero raw exit produces no contention warning. For a direct-passthrough D-Bus operation, filter after both success and every non-fallback error, so those calls warn when strictly over two seconds. A service-file error bypasses the D-Bus filter and falls back; only the ensuing successful raw call can warn. The warning contains elapsed time, argv, and received output and does not cancel, retry, or reorder the operation. Initialization's detection/zone/policy/reload D-Bus calls are outside Docker's `Raw` timer and therefore do not emit this warning or apply the xtables-output filter. |
| firewalld detection | If `ROOTLESSKIT_STATE_DIR` is nonempty, skip firewalld exactly as Docker's `rootless.RunningWithRootlessKit` does and use raw iptables in the current namespace. Otherwise connect to the system bus, attempt the exact firewalld owner match before checking availability, create a proxy for service `org.fedoraproject.FirewallD1`, path `/org/fedoraproject/FirewallD1`, interface `org.fedoraproject.FirewallD1`, and call `getDefaultZone() -> s`. Success selects firewalld; bus or call failure closes the connection and selects raw iptables for process life. Docker ignores `AddMatch` errors: if zbus cannot install the filtered iterator, log that failure, perform the availability check, and retain the resulting initial state without lifecycle updates. Keep initialization single-shot and shared. |
| firewalld initialization side effects | An initially active daemon triggers this exact synchronous sequence before executable detection returns: start the process-lifetime owner observer; call runtime `org.fedoraproject.FirewallD1.zone.getZones() -> as`; if `docker` is absent, call permanent config `addZone` at `/org/fedoraproject/FirewallD1/config` with name `docker` and the exact 16-field settings `(version="1.0", name="docker", description="zone for docker bridge network interfaces", unused=false, target="ACCEPT", all collection fields empty, masquerade=false, icmp_block_inversion=false)`; then call `org.fedoraproject.FirewallD1.config.addPolicy(s, a{sv}) -> o` for `docker-forwarding` with `version="1.0"`, description `allow forwarding to the docker zone`, `ingress_zones=["ANY"]`, `egress_zones=["docker"]`, and `target="ACCEPT"`. Use `Proxy::call_method` for `addZone`/`addPolicy` so their object-path reply is deliberately ignored like Go's `.Err` rather than incorrectly decoded as `()`. If either object was added, call base-interface `reload()`. The names stay `docker`/`docker-forwarding`; they are Docker/firewalld protocol state, not an authorized product-name substitution. |
| firewalld initialization errors | Preserve Docker's uneven control flow. A `getZones` or `addZone` error stops zone/policy/reload setup and is debug-logged by the one-shot initializer; a policy error named `org.fedoraproject.FirewallD1.Exception` whose detail starts `NAME_CONFLICT`, or `org.freedesktop.DBus.Error.UnknownMethod`, means “not added” and is ignored. Any other policy error is warning-only; if the zone was added, reload still runs. A reload error stops initialization and is debug-logged. After any zone/policy/reload error, the already-published active state and connection remain active, so subsequent operations still try passthrough despite the log text saying raw fallback. Connection/default-zone failure alone selects raw. These initialization errors are not returned to the affected caller. |
| firewalld execution | Use a second proxy on the same service/path with interface `org.fedoraproject.FirewallD1.direct`; call `passthrough` with the family string plus typed array of the original arguments, and decode the returned string as operation output. This is D-Bus serialization, not a command string. |
| raw fallback | If and only if a firewalld passthrough error's rendered error chain contains `was not provided by any .service files`, immediately execute the same operation through the raw family backend (which adds `--wait`). Preserve the raw result. All other D-Bus errors are returned and must not mutate through a second backend. The exact zbus private-bus probe rendered the oracle substring. |
| lifecycle | Use zbus 5.19.0's blocking `fdo::DBusProxy::receive_name_owner_changed_with_args(&[(0, "org.fedoraproject.FirewallD1")])` for the narrow match attempted before the initial `getDefaultZone` check. If firewalld is absent initially, close the connection/drop the iterator and remain raw for process life, matching Docker. If present and the iterator was installed, one executor-owned, named **process-lifetime** thread continuously drains it; after every matching signal it re-runs `getDefaultZone` and atomically publishes availability. Owner loss routes later calls raw and owner return restores passthrough. The thread only changes backend state and never issues firewall mutations. It intentionally has no idle teardown/join protocol: the executor is a process singleton and Docker likewise leaves `signalHandler` alive until the bus closes or the process exits. Preserve the last state if the bus stream terminates. No affected caller observes Docker's `Reloaded` timestamp/callback API or calls its zone-interface add/delete helpers, so those surfaces are not ported; the separately reachable initialization zone/policy/reload mutations above are mandatory. |

The oracle has a lazy-initialization quirk: the first `NewChain` inspection can
enter `Raw` before firewalld state is initialized. Its inner raw path performs
the zone/policy/reload sequence and executable discovery, but that already
selected inspection still executes through the binary even when firewalld is
now active. This is observable on hosts that provide firewalld but not iptables.
The implementation must add a focused characterization test, including the
ordering of initialization mutations before that raw command, and retain the
limitation unless the controller records authority to initialize eagerly;
dependency choice does not erase it.

### Rule and chain behavior

All methods below operate synchronously, in source order, and stop on the first
error. There is no transaction, rollback, parallel iterator, best-effort
continuation, or deferred batch.

1. Chain creation/inspection runs `-t filter -n -L PLOYZ-INPUT`; any failure is
   treated as absent and followed by `-t filter -N PLOYZ-INPUT`. A nonempty
   successful create output is an error. The firewall configuration then
   flushes the chain with `-t filter -F PLOYZ-INPUT` even when it already
   existed.
2. For each family, test the exact jump with `-t filter -C INPUT -m comment
   --comment Ployz-managed -j PLOYZ-INPUT`. Exit status zero means present; all
   other statuses/errors mean absent for this predicate.
3. When absent, list with `-t filter -L INPUT --line-numbers`. Split lines into
   whitespace fields. The first line with at least two fields, numeric field 0,
   and field 1 exactly `DROP` or `REJECT` supplies the insertion position.
   Insert the jump at that position; if none exists, append it. Do not infer
   policy, parse textual comments, reorder an existing jump, or inspect
   nonnumeric lines.
4. Rule programming first checks exact existence using `-C`. Insert executes
   only when absent; delete executes only when present. Preserve Docker's
   limitation that existence collapses transport/check errors to “absent”:
   insertion proceeds to a mutation (and normally returns its clearer error),
   while deletion becomes a silent no-op.
5. Configuration completes all IPv4 chain setup before IPv6 chain setup, then
   inserts the two IPv4 accept rules in oracle order, then the two IPv6 accept
   rules in oracle order. Since each uses `-I`, final on-host ordering is the
   reverse within each pair. Stop at the first error and do not undo earlier
   operations.
6. Docker-network configuration is IPv4 only: conditionally insert the
   WireGuard-to-bridge rule in `filter/DOCKER-USER`; conditionally insert UDP
   then TCP DNS rules in `filter/PLOYZ-INPUT`; conditionally delete and then
   insert the skip-masquerade rule in `nat/POSTROUTING` so it is first. Cleanup
   conditionally deletes the DOCKER-USER and POSTROUTING rules and stops on the
   first error.
7. Firewall cleanup handles IPv4 fully before IPv6. Conditionally delete the
   INPUT jump, flush PLOYZ-INPUT, then delete PLOYZ-INPUT. Flush/delete errors
   containing `No chain` are ignored; other errors stop cleanup. This substring
   behavior applies to the fully rendered operation error, preserving the
   oracle's textual limitation.
8. On macOS, both configure and cleanup immediately return exactly `not
   supported on Darwin`. The Linux transport dependency stays under target
   configuration; the Darwin stub performs no lookup, D-Bus connection, or
   process spawn.

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Exact IPv4/IPv6 argv control, chain/rule lifecycle, first DROP/REJECT insertion, xtables wait, merged output/status, firewalld zone/policy/reload initialization, direct passthrough and fallback, synchronous first-error order | Standard `Command`/owned-descriptor probe; zbus exact initialization/error/passthrough call-shape and private-bus lifecycle probe; isolated privileged-container iptables/ip6tables probe; oracle/Docker source mapping above | `pass` |
| License and security | Permissive Rust dependencies; no shell interpolation; reviewed D-Bus serialization; no known RustSec vulnerability | Rust std is MIT/Apache-2.0; zbus is MIT. The exact 86-package lock scanned 1,216 local RustSec advisories with no finding and passed cargo-deny 0.20.2 against the complete transitive graph with only `MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, and `Unicode-3.0` allowed. All process inputs remain distinct argv; D-Bus destination/path/interface and method are fixed constants. | `pass` |
| Platforms and targets | Linux production; macOS unsupported stub; Rust 1.96 | The exact target-gated probe ran on `x86_64-unknown-linux-gnu` and cross-compiled for `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, and `aarch64-apple-darwin`. The non-Linux target contains only the stub; execution of the exact Darwin error remains a package acceptance check on a macOS runner. | `pass` |
| Maintenance and Rust version | Maintained, non-yanked, Rust 1.96-compatible dependency | zbus 5.19.0 was released 2026-08-09, declares MSRV 1.87 and edition 2024, and compiled on Rust 1.96. Its crates.io API reported 71,098,125 lifetime downloads, 19,390,181 recent downloads, and 449 reverse dependencies on the research date ([crate API](https://crates.io/api/v1/crates/zbus), [reverse-dependency API](https://crates.io/api/v1/crates/zbus/reverse_dependencies?page=1&per_page=1), [5.19.0 source tag](https://github.com/z-galaxy/zbus/tree/zbus-5.19.0)). | `pass` |
| Architectural constraints | Privileged mutations must be auditable, argv-safe, synchronous, and preserve transport distinctions | One package-owned typed domain executor; standard process and zbus blocking proxy are not hidden behind a clone of Docker's `IPTable` API. No shell, C D-Bus library, nftables translation, or competing async runtime is introduced. | `pass` |

The host-provided `iptables`/`ip6tables` programs and firewalld daemon have
their own GPL licenses. They are separate installed programs/services invoked
through process and D-Bus protocols; this decision neither links nor vendors
them into Ployz. Deployment packaging must continue to treat them as host
requirements.

## Candidate comparison

Adoption figures below are crates.io first-party API values observed on
2026-08-12. Download counts are supporting evidence, not a substitute for the
hard behavior gates.

| Candidate | Version / license / MSRV / maintenance / adoption | Behavioral and architectural result |
| --- | --- | --- |
| Rust std process/fd/UnixStream + zbus | Rust 1.96; zbus **5.19.0**, MIT, declared Rust 1.87, released 2026-08-09; 71.1M lifetime downloads and 449 reverse dependencies | **Selected.** Exact argv, output/status, and synchronous D-Bus control; pure-Rust D-Bus protocol implementation (with reviewed internal unsafe), no shell or firewall-model translation. |
| `iptables` | **0.6.0**, MIT, no declared `rust-version` (edition 2024 implies Rust 1.85+), released 2025-08-07; 855,444 lifetime downloads; 0 current reverse dependencies in the API ([crate API](https://crates.io/api/v1/crates/iptables), [source](https://github.com/yaa110/rust-iptables/tree/v0.6.0)) | Rejected at behavior/security architecture gates. Its rule APIs accept one string and use a regex `split_quoted`, losing arbitrary argv boundaries; `Command::output` keeps stdout/stderr separate; it returns different unique-rule semantics; performs a version probe; and has no firewalld backend, slow warning, Docker fallback, or raw combined-output policy. The exact 8-package lock was RustSec-clean, but security alone cannot repair parity. |
| `duct` | **1.1.1**, MIT, no declared MSRV, released 2025-11-09; 35.2M downloads, 242 reverse dependencies ([crate API](https://crates.io/api/v1/crates/duct), [source](https://github.com/oconnor663/duct.rs/tree/v1.1.1)) | Credible argv-safe process runner with `stderr_to_stdout`, capture, and unchecked status. Rejected as unnecessary: Rust 1.96 safely supplies the one-command, one-merged-stream primitive in a smaller auditable seam, while Duct's pipeline/timeout/shared-child surface does not address firewalld or firewall policy. |
| `os_pipe` | **1.2.3**, MIT, Rust 1.63, released 2025-10-11; 90.6M downloads, 134 reverse dependencies ([crate API](https://crates.io/api/v1/crates/os_pipe)) | Credible low-level combined-output helper, but Rust 1.96 `UnixStream` plus owned descriptors already passed the exact probe. Adding a crate would not close another behavior gap. |
| `nftables` | **0.6.3**, MIT OR Apache-2.0, Rust 1.76, released 2025-08-15; 2.58M downloads ([crate API](https://crates.io/api/v1/crates/nftables), [source](https://github.com/nftables-rs/nftables-rs/tree/v0.6.3)) | Maintained and permissive, but models the `nft` JSON API. It cannot manage an iptables-legacy host, preserve iptables listing/error/output semantics, or call firewalld direct passthrough. Rejected at behavior gate. |
| `rustables` | **0.8.8**, GPL-3.0-or-later, released 2026-06-23; 42,207 downloads ([crate API](https://crates.io/api/v1/crates/rustables), [source](https://gitlab.com/rustwall/rustables/-/tree/v0.8.8)) | Safe nftables netlink model, not xtables/firewalld, and its copyleft license fails the permissive dependency gate. Rejected at license and behavior gates. |
| `dbus` / dbus-rs | **0.9.12**, MIT OR Apache-2.0, no declared MSRV, released 2026-07-03; 35.3M downloads and 178 reverse dependencies ([crate API](https://crates.io/api/v1/crates/dbus), [source](https://github.com/diwic/dbus-rs/tree/0.9.12)) | Maintained and popular, with a natural blocking API, but links the native libdbus C library through FFI and adds a system development/runtime dependency. zbus is more idiomatic for a new Rust design, more adopted by downloads, declares its MSRV, and passed the exact pure-Rust call probe. |
| `rustbus` | **0.19.3**, MIT, no declared MSRV, last released 2023-08-29; 453,684 downloads and 7 reverse dependencies ([crate API](https://crates.io/api/v1/crates/rustbus), [source](https://github.com/KillingSpark/rustbus/tree/v0.19.3)) | Pure-Rust but lower-level, materially less adopted, and stale beside zbus. It offers no firewall-specific advantage. Rejected on maintenance/adoption and integration complexity. |
| `libiptc-sys` / direct libiptc | **0.1.0**, GPL-3.0, last released 2016-09-21; 2,487 downloads ([crate API](https://crates.io/api/v1/crates/libiptc-sys)) | Old unsafe FFI, copyleft, bypasses firewalld, and cannot preserve executable combined-output/exit behavior. Rejected at every hard gate except Linux availability. |
| `riptables` / `rust-iptables` | **0.1.0** (MIT, 2019) / **0.0.2** (MIT OR Apache-2.0, 2021); 2,087 / 3,626 downloads ([riptables API](https://crates.io/api/v1/crates/riptables), [rust-iptables API](https://crates.io/api/v1/crates/rust-iptables)) | Abandoned or near-zero adoption, no complete Docker/firewalld contract, and no advantage over the current `iptables` crate. Rejected on maintenance, adoption, and behavior. |

Named established-project manifests support the aggregate adoption comparison.
GNOME's maintained
[`glycin` workspace](https://github.com/GNOME/glycin/blob/cf6d31827fbb37c53a37441626318b136dfa070e/Cargo.toml#L134-L140)
pins zbus 5.x and zvariant for its production image-loading stack. For the
closest blocking competitor, Mullvad VPN's exact
[`talpid-dbus` manifest](https://github.com/mullvad/mullvadvpn-app/blob/71a1e796bb17d76929534adb9f70bd4db6dfbaa1/talpid-dbus/Cargo.toml#L8-L20)
keeps dbus-rs 0.9 as its default Linux backend while marking zbus experimental;
System76's firmware daemon likewise directly selects
[`dbus = "0.9"`](https://github.com/pop-os/system76-firmware/blob/0337bd4298f0de43f4adeb2f527e5ab6dd9e611b/daemon/Cargo.toml#L8-L18).
These are primary-source examples of real deployment for both candidates. They
do not outweigh zbus's hard-gate advantages here: pure Rust, declared MSRV,
exact typed-array probe, and no native libdbus build/runtime dependency.

## Lifecycle, operational limits, and security boundary

- This is privileged host mutation. Production command vectors must be created
  from typed/internal values and stored as `OsString`/argv elements. Never log a
  reconstructed command as executable input. Tests should inject a runner or
  resolved fake executable, not alter production command construction.
- Resolve executables once and retain absolute paths. A privileged deployment
  must supply a controlled `PATH` at initialization; relative or empty PATH
  entries must not make a writable working-directory binary eligible.
- The executor is intentionally synchronous. Preserve the oracle's command
  granularity: each raw command uses `--wait`, but the `-C` followed by
  `-I`/`-D` pair is not widened into a new process-local transaction. A wider
  mutex would remove an observable race and needs separate authority if future
  concurrent callers make that desirable.
- `--wait` is indefinite, matching the oracle. The two-second threshold only
  warns; it is not a timeout. Preserve the Moby asymmetry that a failed raw
  process returns before the slow/output filter, while a non-fallback D-Bus
  error passes through it. Adding cancellation would create a partially
  mutated state and requires separate product authority.
- An active firewalld makes one-shot initialization a privileged permanent
  configuration mutation: it may create the `docker` zone and
  `docker-forwarding` policy, then reload runtime state. This behavior is
  inherited from Docker 28.5.0 even though neither affected caller names those
  helpers directly. Renaming, modernizing `addZone` to `addZone2`, or suppressing
  this setup requires product authority because each changes persistent host
  state or compatibility.
- firewalld direct passthrough is runtime and untracked. A firewalld reload can
  remove rules; no affected caller registers Docker's generic reload callbacks.
  This capability preserves the frozen scope and must not silently add
  persistence or replay.
- firewalld declares the direct interface deprecated and promises future
  removal. If a supported distribution removes it, exact passthrough parity is
  no longer available. Falling back for UnknownMethod, translating to policies,
  or switching to nftables would change ordering and error semantics and
  requires user/product authority. Until then, only the exact service-file
  error triggers raw fallback.
- zbus's blocking API runs an internal executor and warns against use inside an
  async context. Keep the firewall transaction at the synchronous system edge;
  an async caller must await one blocking transaction as a unit rather than
  invoking zbus blocking methods on an executor thread.
- When firewalld is initially active, the filtered owner observer and its bus
  connection are process-lifetime resources. There is deliberately no idle
  shutdown/join promise; adding one would be new lifecycle behavior and must be
  separately designed and proven if the executor ever stops being a singleton.
- There is no rollback. If IPv4 succeeds and IPv6 fails, or an early rule
  succeeds and a later rule fails, retain the partial host state and return the
  first error, exactly as the oracle does.

The exact transitive license audit allows only `MIT`, `Apache-2.0`,
`Apache-2.0 WITH LLVM-exception`, and `Unicode-3.0`. Packages with OR
expressions are accepted through a permissive branch: notably `r-efi 6.0.0`
declares `MIT OR Apache-2.0 OR LGPL-2.1-or-later`, so this graph uses the
MIT/Apache grant and does not rely on LGPL; rustix/linux-raw-sys similarly pass
through Apache/MIT alternatives. The unpublished probe package is the only
unlicensed row and is explicitly ignored as private. There are no unknown or
non-permissive-only transitive packages.

## Verification and runnable probes

The corrected disposable Rust probe at
`/tmp/ployz-firewall-research-probe.rGSN5u` matches the exact selected
Linux-target dependency declaration and non-Linux stub. On a private
`dbus-daemon`, it installed the filtered owner iterator before initial
detection, then successfully served the exact base, zone, config, and direct
firewalld interfaces. It observed, in order, `getDefaultZone`, `getZones`, the
full 16-field `addZone` input, the five-key typed `addPolicy` dictionary, and
`reload`; the mock returned real object paths and the client deliberately used
`call_method` to ignore those bodies. It also verified the exact nonfatal
`NAME_CONFLICT` and `UnknownMethod` policy-error classifier. The service decoded
`passthrough(s, as) -> s` and asserted the received family and every argv
element. The argv included whitespace plus `$();*` in one element, proving no
shell interpretation or boundary loss. It asserted the returned output,
confirmed the no-activation-file error below, and exercised an initially
running service through owner loss and owner return. The exact
`NameOwnerChanged` iterator re-ran `getDefaultZone`; the observed route log was
`dbus, raw, dbus`, while the service saw only the two passthrough operations in
their original order. The bounded probe consumes two signals and joins for test
determinism; production deliberately retains its listener for process life.
The same probe implemented the standard-library merged-output runner and
verified interleaved stdout/stderr bytes, exit status 23, and a
metacharacter/whitespace argument arriving as one argv element.

The missing-service branch produced the oracle substring:

```text
org.freedesktop.DBus.Error.ServiceUnknown: The name org.fedoraproject.FirewallD1 was not provided by any .service files
```

Commands passed on Rust 1.96.0:

```text
cargo +1.96.0 fmt --check
cargo +1.96.0 check --locked --all-targets
cargo +1.96.0 clippy --locked --all-targets -- -D warnings
dbus-run-session -- cargo +1.96.0 run --locked --quiet
cargo +1.96.0 check --locked --all-targets --target x86_64-unknown-linux-gnu
cargo +1.96.0 check --locked --all-targets --target aarch64-unknown-linux-gnu
cargo +1.96.0 check --locked --all-targets --target x86_64-apple-darwin
cargo +1.96.0 check --locked --all-targets --target aarch64-apple-darwin
cargo audit --no-fetch --deny warnings --file Cargo.lock
cargo deny --locked check licenses
```

The audit loaded 1,216 advisories and found no vulnerability in the exact
86-package lock. cargo-deny 0.20.2 returned `licenses ok` for the allowed set
recorded above. Probe hashes after the final run:

```text
2675a9b43c39d63ba2c821564af6ef01460c80e4df49fe330168c741286e4f2a  Cargo.toml
d82fd319cae5d8052d391d808b7af46f7071c9ef5148d57c52a9b854744cea2c  Cargo.lock
0dd8301faf2e300af9432d72e961933e2b820974eff4486c1172f7961c6cc267  deny.toml
f22701ae08e767c02cfa413412899ac7160e7a28560b82da5d29c5293593e0de  src/main.rs
```

An isolated privileged container (`ghcr.io/psviderski/ucind:latest`, container
network namespace only, `--rm`) ran iptables/ip6tables 1.8.11 nf_tables backend
and passed create, `-C`, conditional insert, flush, and delete for both
families. A second run created `PLOYZ-INPUT`, inserted ACCEPT then DROP/REJECT
rules, inserted the Ployz jump at numeric position 2, and listed it immediately
before the first DROP for both IPv4 and IPv6. The host firewall was not
modified.

The rejected `iptables = 0.6.0` candidate separately compiled on Rust 1.96 and
its exact 8-package lock passed the same 1,216-advisory audit. Its probe hashes
were:

```text
90dc3a020f012ef86ec19c48e6524c8dd008de63aca7138a5bbf1a3b43e82786  Cargo.toml
fbf3e5e0b2eb0dde3a885bf882de5d5ecefcb88b52af4f123c1ed78eedf03076  Cargo.lock
```

Package acceptance must add deterministic fake-runner/D-Bus tests for every
mapping above, especially output-vs-status errors, the exact fallback
substring, two-second logging with paused/injected time, check-error collapse,
every zone-exists/policy-error/reload branch and the lazy first-raw ordering,
first DROP/REJECT position, partial progress, and Darwin's exact error. The
privileged namespace test is an integration gate where CI supplies the
capability; it must never target the CI host firewall.

## Review

Fresh adversarial dependency review is required because this is privileged
network/firewall mutation. The authoritative dependency owner will record each
finding, correction, fresh re-review context, and the final clean result here.

| Review pass | Reviewer | Result | Findings / corrections |
| --- | --- | --- | --- |
| Background research child | `/root/background_research` | draft complete | Primary-source comparison and runnable probes above; no approval authority exercised by this child. |
| Fresh adversarial review | `/root/adversarial_review_1` | findings fixed | `P01`: made the exact filtered zbus owner-change iterator, state recomputation, thread ownership, and teardown unconditional for an initially active firewalld and added loss/return route evidence. `P02`: replaced the insufficient preliminary artifact with the exact target-gated successful service/argv/fallback/lifecycle probe and described Darwin evidence as cross-compilation. `P03`: added the exact-lock cargo-deny audit, allowed SPDX set, and OR-license evaluation. `P04`: added primary-source GNOME, Mullvad, and System76 manifest adoption evidence. |
| Fresh corrected re-review | `/root/adversarial_rereview_1` | findings fixed | Prior `P02`-`P04` closed. Remaining `P01`: removed an invented idle teardown/join promise and specified Docker-parity process-lifetime observer ownership. `P05`: restored the reachable Docker zone, forwarding-policy, reload, and error-control-flow initialization; expanded the primary-source range and runnable D-Bus probe. |
| Fresh post-correction re-review | `/root/adversarial_rereview_2` | **clean / accepted** | Reviewed candidate SHA-256 `e1db05950a6a0fabdd871d7107a466612e6dcea6fb2182c06516622e9ee0d9b5`; zero actionable findings. Independently confirmed `P01`/`P05` closed, `P02`-`P04` remained closed, and all source, probe, license, security, platform, lifecycle, and alternative claims matched. |
