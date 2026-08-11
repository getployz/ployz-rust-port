# Dependency decision: Unix account resolution and ownership changes

| Field | Value |
| --- | --- |
| Status | `approved`. Native GitHub Actions run `31529008863` passed on macOS amd64 and arm64 at source commit `45ecd305c32860edace30dc07f1201d16d4166a9`; both architectures produced byte-identical Go 1.26.1 and Rust 1.96.0/libc 0.2.189 results for current and Directory Service accounts. Every hard gate and the fresh exact critical dependency review pass. |
| Capability | Resolve a user name to UID and primary GID, resolve a group name to GID, preserve the shipped account source and errors, and change pathname ownership while independently leaving either ID unchanged. |
| Selected dependency and exact version | Cgo-free Linux uses `rustix = "=1.1.4"` with `default-features = false`, features `std,process`, a project safe files parser, and Rust 1.96 `std::os::unix::fs::chown`; native macOS and cgo-equivalent Linux use target/profile-scoped `libc = "=0.2.189"` with `default-features = false` behind a narrow project safe wrapper. |
| Required configuration | Preserve every non-experimental build row with explicit files/native Cargo profiles: Linux release/Docker uses files; macOS always uses native; canonical/ordinary cgo-enabled Linux development uses native. Never choose by ambient linkage. The files profile must forbid rustix `use-libc` and external `rustix_use_libc`; only the native profile may enable direct libc account calls. |
| License | rustix: `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`; libc: `MIT OR Apache-2.0`; project parser and Rust standard library otherwise. |
| Research date | `2026-08-11` UTC |
| Research base | `4e774f684afa3f44df83032fe7f84a20f22a9a73` |
| Supersedes | The blocked native-macOS state recorded through `0cb7411`/`74f067b`. Package compilation alone did not prove `internal/fs` lookup reachability, but caller tracing proves a separate shipped `sshexec` path reaches native macOS `Current()`, and native run `31529008863` now supplies the missing amd64/arm64 Open Directory evidence. |
| Request | Direct controller delegation for `upstream/uncloud/internal/fs` and discovered direct consumers `upstream/uncloud/internal/sshexec` and `upstream/uncloud/internal/machine` / their future Rust crates. |

## Approved decision

Continue with the narrow target/profile-specific libc boundary. Repository
policy defines unsafe FFI as a critical capability subject to research and a
second fresh adversarial reviewer. The native runtime blocker is resolved:
manually dispatched GitHub Actions run
[`31529008863`](https://github.com/getployz/ployz-rust-port/actions/runs/31529008863)
executed the committed differential at exact head
`45ecd305c32860edace30dc07f1201d16d4166a9` on both required native runner
architectures. The probe source is unchanged between that head and this
research base.

Both jobs verified the native machine, exact Go/Rust toolchains, cgo, and the
locked libc version before execution. On each runner, the current account was
absent from `/etc/passwd` but resolved through both `dscl` and `dscacheutil`, so
the result is not files-backend evidence mislabeled as Open Directory. Go and
Rust then returned byte-identical TSV rows for `Current()`, lookup by current
name and UID, and lookup of a second Directory Service account by name and UID.
The mandated fresh adversarial critical reviewer independently reproduced the
run/probe/artifact checks and found no actionable issue. The dependency gate is
approved; the controller may update the registry and recompute readiness.

All non-experimental observable rows remain in scope. The eventual build
mapping must preserve files-only Linux releases, native macOS releases, and
native NSS in cgo-enabled canonical/ordinary Linux development builds. There is
no option in this record to discard one of those contracts.

For the cgo-free profile, use this exact design:

- obtain the real UID and GID with safe
  `rustix 1.1.4` process APIs using its raw Linux backend;
- reproduce Go's process-global `Current()` cache and environment fallback;
- parse `/etc/passwd` and `/etc/group` in project-owned safe Rust for remaining
  lookups; and
- use `std::os::unix::fs::chown` for pathname ownership.

The native macOS need is narrower than `internal/fs` account ownership. The
released CLI's SSH path needs only cached `Current()` and consumes only its
username; daemon-only `LookupUIDGID`, group lookup, and `Chown` remain Linux
runtime behaviors. That narrower reachability still makes Open Directory
observable and must not be replaced by `$USER` or `/etc/passwd`.

## Frozen caller and artifact authority

- [`internal/fs/fs.go`](../../upstream/uncloud/internal/fs/fs.go) looks up a
  user before a group, parses returned textual IDs as Go `int`, treats empty
  username/group values as independently omitted only for `Chown`, calls
  `os.Chown` even when both are omitted, and wraps lookup, UID parse, GID parse,
  and chown failures separately. Its existing tests cover only home expansion.
- [`cmd/uc/main.go`](../../upstream/uncloud/cmd/uc/main.go) calls only
  `fs.ExpandHomeDir` and `fs.Exists`. The CLI uses `machine` package constants
  and token/client helpers, but it never constructs the daemon `Machine` or
  calls the `internal/fs` account/ownership entry points.
- The CLI nevertheless reaches account resolution through a different package:
  both [`machine/add.go`](../../upstream/uncloud/cmd/uc/machine/add.go) and
  [`machine/init.go`](../../upstream/uncloud/cmd/uc/machine/init.go) accept
  `ssh+go://host`; `SSHDestination.Parse` preserves its omitted user; their
  add/init flows reach `provisionOrConnectRemoteMachine`, which calls
  [`sshexec.Connect`](../../upstream/uncloud/internal/sshexec/ssh.go), which
  calls `os/user.Current()` when the user is empty and uses the returned
  username for both SSH attempts. It ignores a `Current()` error, leaving the
  SSH username empty. This is shipped Linux and macOS behavior.
- All production `internal/fs.LookupUIDGID`/`Chown` callers are:
  [`machine.go`](../../upstream/uncloud/internal/machine/machine.go),
  [`corroservice/config.go`](../../upstream/uncloud/internal/machine/corroservice/config.go),
  [`corromigrate/migrate.go`](../../upstream/uncloud/internal/machine/corromigrate/migrate.go),
  and
  [`caddyconfig/controller.go`](../../upstream/uncloud/internal/machine/caddyconfig/controller.go).
  `internal/daemon/daemon.go` is the only production constructor of
  `machine.NewMachine`; `cmd/uncloudd` owns that daemon path.
- [`internal/machine/machine.go`](../../upstream/uncloud/internal/machine/machine.go)
  also directly calls `os/user.LookupGroup("uncloud")` while creating its API
  Unix socket. Typed unknown-group falls back to root GID 0; every other lookup
  failure aborts. A found textual GID is parsed as Go `int`, the socket parent
  is chowned with unchanged UID, and that GID is supplied to socket creation.
- [`.goreleaser.yaml`](../../upstream/uncloud/.goreleaser.yaml) builds
  `uncloudd` only for Linux amd64/arm64 and explicitly sets `CGO_ENABLED=0`.
  [`Dockerfile`](../../upstream/uncloud/Dockerfile) independently sets
  `CGO_ENABLED=0` for the Linux daemon copied into the Alpine image.
- Go 1.26.1's official
  [`lookup_unix.go`](https://go.dev/src/os/user/lookup_unix.go) selects the
  files backend on non-Darwin Unix when cgo is disabled. Its source defines the
  byte parser, malformed-record handling, streaming behavior, typed unknown
  errors, and fixed `/etc/passwd` and `/etc/group` paths used below.
- Go 1.26.1's official
  [`cgo_lookup_unix.go`](https://go.dev/src/os/user/cgo_lookup_unix.go) is also
  selected on Darwin. `Current()` resolves the real UID with `getpwuid_r`,
  retries `ERANGE` with bounded buffer growth, and returns the native account
  record supplied by macOS/Open Directory.
- Apple's official
  [`getpwuid_r` manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/getpwnam.3.html)
  defines the target C API. Apple's
  [Open Directory overview](https://developer.apple.com/library/archive/documentation/Porting/Conceptual/PortingUnix/additionalfeatures/additionalfeatures.html)
  documents that the native directory supplies local and remote user data,
  including LDAP-backed accounts.
- Rust's official
  [`std::os::unix::fs::chown`](https://doc.rust-lang.org/std/os/unix/fs/fn.chown.html)
  accepts `Option<u32>` IDs, follows the final symlink, documents privilege
  requirements, and preserves kernel set-ID-bit and file-capability effects.
- rustix 1.1.4's official
  [`getuid`](https://docs.rs/rustix/1.1.4/rustix/process/fn.getuid.html) and
  [`getgid`](https://docs.rs/rustix/1.1.4/rustix/process/fn.getgid.html) APIs
  return real process IDs without a fallible or unsafe caller API. Its
  [published manifest](https://docs.rs/crate/rustix/1.1.4/source/Cargo.toml.orig)
  selects the raw Linux backend by default on the required x86_64/aarch64
  targets and makes libc opt-in through `use-libc`/`rustix_use_libc`.
- libc 0.2.189's published
  [`getpwuid_r`](https://docs.rs/libc/0.2.189/libc/fn.getpwuid_r.html),
  [`getpwnam_r`](https://docs.rs/libc/0.2.189/libc/fn.getpwnam_r.html), and
  [`getgrnam_r`](https://docs.rs/libc/0.2.189/libc/fn.getgrnam_r.html) bindings
  expose the exact target ABI without imposing a higher-level cache or error
  model. The [Rust project crate](https://crates.io/crates/libc/0.2.189) is the
  current maximum stable release, not the separate 1.0 alpha line.

## Exact target and build matrix

| Frozen build path | Targets | Account backend for observable lookup callers | Port decision |
| --- | --- | --- | --- |
| GoReleaser `uncloudd` | Linux amd64/arm64 | Explicit `CGO_ENABLED=0`; fixed files, including cached-current shortcut/fallback | Conditional cgo-free profile: rustix IDs, safe Rust files parser, `std::chown`. |
| Docker `uncloudd` | Linux amd64/arm64 in Alpine | Explicit `CGO_ENABLED=0`; container files, including cached-current shortcut/fallback | Same conditional cgo-free profile. |
| GoReleaser Linux `uc` | Linux amd64/arm64 | `CGO_ENABLED=0`; empty-user `ssh+go` in both machine add/init reaches cached cgo-free `Current()` (files then environment fallback) | Conditional cgo-free profile is required for these CLI paths. They do not reach group lookup or chown. |
| GoReleaser macOS `uc` | macOS amd64/arm64 | Explicit `CGO_ENABLED=1`; empty-user `ssh+go` in machine add/init reaches cached native `Current()` through `getpwuid_r`/Open Directory | Native profile required. libc 0.2.189 passed the native amd64/arm64 differential; files/environment-only behavior remains wrong. |
| `mise` `build:uncloudd-*` | Linux amd64/arm64 | Host-dependent because the task does not set CGO. On this Linux/amd64 host, amd64 was native glibc NSS and arm64 cross-build was files-only. | Preserve the resolved Go backend with an explicit Rust build feature/profile; do not infer it from linkage. Native libc on cgo-equivalent builds, files otherwise. |
| Ordinary development `go run` / `go build` | Host target | Native on cgo-enabled Linux and Darwin; files on cgo-free non-Darwin Unix | Preserve with the corresponding explicit native/files Rust configuration. |

The last two rows prevent ambient linkage from silently changing the Rust
design. The controller/integrator must encode the equivalent build choice;
worker crates may not invent root features or release commands.

## Required Linux files-backend contract

Use a small domain parser, not a general account-management abstraction.
Reader-based private helpers provide tests. Apart from the exact process-global
current-user behavior below, open the fixed files on every call. Do not consult
`nsswitch.conf`, invoke libc/NSS, spawn `getent`, or expand NIS `+`/`-` records.

### Process-global current-user contract

Go 1.26.1's `os/user.Lookup(name)` calls `Current()` first. The first
`Current()` call computes and process-globally caches either its successful
user record or its error; later calls return a copy and never observe account
file, environment, or identity changes. Reproduce this with `OnceLock` or an
equivalent one-winner primitive, including concurrent first calls.

For the cgo-free Linux profile:

1. Read the process's real UID through `rustix::process::getuid()` and convert
   it to an unsigned decimal string.
2. Look up that UID in `/etc/passwd`. If successful, cache the full file user
   record. Match field two against the canonical unsigned decimal UID text
   exactly (`0`, not `00`), while validating both UID and primary GID as in the
   parser contract. The file record wins even when `$USER` or `$HOME`
   disagrees.
3. On *any* UID lookup failure (unknown, open, read, or malformed-only result),
   discard that error and build a fallback from the real UID, real GID from
   `rustix::process::getgid()`, `$USER`, and `os.UserHomeDir` behavior (`$HOME`
   on this Unix profile). Preserve Unix environment values as `OsString` bytes,
   including non-UTF-8; only byte emptiness is missing. The display name is
   empty.
4. Accept the fallback only when UID, username, and home are all nonempty.
   Otherwise cache and return `user: Current requires cgo or <missing> set in
   environment`, listing `$USER`, `$HOME`, or both in that order. The cached
   error is just as sticky as a cached success.
5. `Lookup(name)` returns a copy of the cached current record only when its
   username exactly matches `name`; if current initialization failed or the
   name differs, perform the ordinary name lookup. Group lookup never uses the
   current-user cache.

This shortcut is observable even though `internal/fs` does not call
`user.Current` directly: its username lookup calls `os/user.Lookup`. It is also
called directly by `sshexec.Connect` for an omitted SSH user.

There must be one process-global current-user cell shared across these caller
paths, not one cache per crate. Because frozen `sshexec` already depends on
`internal/fs`, the Rust account capability may live behind one natural
`ployz-internal-fs` API consumed by `ployz-internal-sshexec`; an equivalent
single-owner design is acceptable. Initialization by either caller must govern
the other for the remainder of the process.

Preserve Go 1.26.1's files parser:

1. Accept account names as Unix bytes (`&OsStr`). Read logical lines as bytes.
   Match Go `bytes.TrimSpace`: trim Unicode White_Space at each end while
   retaining invalid UTF-8 bytes. Skip blank lines and lines whose first
   remaining byte is `#`.
2. For users, buffer through at least six colons and split at most seven ways.
   For groups, buffer through at least three colons and split at most four
   ways. Compare field zero exactly. Reject an empty name and records whose
   name begins `+` or `-`.
3. Validate a matched user's UID and primary GID, or a matched group's GID, as
   signed decimal `i64`, matching `strconv.Atoi` on released 64-bit targets.
   Skip a malformed or overflowing matched record so a later duplicate valid
   record may win. Return the first valid match. If none is valid, return typed
   unknown-user/unknown-group, not a parse error.
4. Preserve large-record streaming. Before the required colon count there is
   no invented line cap. Once enough fields are buffered, a match may return
   without reading its irrelevant suffix; a nonmatch must drain the suffix.
   Consequently an I/O error in an unread suffix is hidden after a match but
   observed while searching.
5. Distinguish file-open/read errors from not-found, with user/group context.
   Malformed records are not I/O errors. Ignore close errors, matching the
   oracle's deferred close.

### Byte and numeric edges

- Embedded NUL in the requested name is an ordinary byte on the files path.
  It is normally unknown, but a malformed account file containing that byte
  can match. Do not use `CString`.
- Accept signed values through `i64::MAX`, including negatives and values above
  `u32::MAX`. At ownership time, narrow with the oracle's modulo-2^32 result
  (`id as u32`); do not add validation the oracle lacks.
- `-1`, `4294967295`, and every value congruent to `u32::MAX` collide with the
  kernel all-ones “leave unchanged” sentinel. The named identity cannot be
  assigned by pathname `chown`; the other ID may still change. Preserve this
  limitation in tests.
- Account files are normally privileged, but malformed IDs and unbounded
  prefixes remain observable security/denial-of-service behavior. The parser
  contains no `unsafe`.

## Ownership and errors

Use `std::os::unix::fs::chown` on Linux:

- accept `&Path` to preserve non-UTF-8 bytes;
- map an omitted name to `None` and a found signed ID to `Some(id as u32)`;
- call `chown(path, None, None)` when both names are empty;
- retry only `io::ErrorKind::Interrupted`, matching Go `os.Chown`;
- use `chown`, not `lchown`, so the final symlink is followed and a broken link
  fails;
- preserve a NUL pathname as a contextual input/chown failure without prefix
  truncation; and
- do not pre-authorize, suppress kernel errors, restore cleared mode bits or
  capabilities, suppress ctime changes, or skip same-owner calls.

Lookup order is user, then group, then chown. A user failure prevents group
lookup and ownership. A group failure prevents ownership. Lookup and ownership
are synchronous; only the `Current()` result/error described above is cached.
There is no cancellation input, timeout, or background work to orphan.

The natural Rust error model must distinguish:

| Operation | Required outcomes |
| --- | --- |
| Files lookup | Found signed IDs; typed unknown user/group; contextual file-open/read error |
| Ownership | Contextual path/input or OS error retaining the `io::Error`; retry only `Interrupted` |

## Required macOS native-current contract

Only the no-user `ssh+go` CLI path is currently reachable on macOS. Preserve
Go 1.26.1's generic process-global `Current()` success/error cache described
above, but compute its first value with the Darwin native backend:

1. Read the real UID, then call `getpwuid_r`; do not substitute effective UID,
   `$USER`, `$HOME`, `/etc/passwd`, a process, or a framework query with
   different semantics.
2. Get the initial scratch size from `_SC_GETPW_R_SIZE_MAX`. Use 1,024 when
   `sysconf` returns `-1`; clamp any other nonpositive or greater-than-1-MiB
   value to 1 MiB. Retry `ERANGE` by doubling. If the next buffer would exceed
   1 MiB, return `internal buffer exceeds 1048576 bytes`.
3. Treat `ENOENT`, or success with no result, as unknown UID. Preserve other
   errno failures with lookup-user-ID context. The caller deliberately ignores
   that error and proceeds with an empty SSH username, but the error remains
   cached for later calls.
4. Copy UID/GID as unsigned decimal strings and copy username, GECOS, and home
   from the native record; truncate display name at its first comma. The SSH
   path consumes only username, but preserve the whole cached `User` contract.

Any wrapper must encapsulate pointer lifetimes, result-pointer validation,
buffer ownership, integer conversions, and C-string copying behind a safe Rust
API. Native macOS amd64 and arm64 runtime probes are mandatory; cross-checking
cannot establish Open Directory behavior.

## Required cgo-enabled Linux native contract

When the explicit build profile corresponds to cgo-enabled Go, use glibc NSS;
do not reuse the files parser. Native `Current()` uses the same `getpwuid_r`
algorithm and shared process-global cache as macOS. `Lookup(name)` checks that
cache first and returns it on an exact username match; otherwise it calls
`getpwnam_r`. `LookupGroup(name)` calls `getgrnam_r` without a current-user
shortcut.

For both name functions:

- copy the complete requested Go/Rust byte string into a zero-filled buffer
  one byte longer, so embedded NUL truncates the native query at its first NUL;
- use `_SC_GETPW_R_SIZE_MAX` or `_SC_GETGR_R_SIZE_MAX`, the 1,024 fallback,
  `ERANGE` doubling, and the same 1-MiB ceiling;
- map `ENOENT` and success-with-null-result to the typed unknown-name error for
  the *original untruncated* requested value; retain other errno with user/group
  operation context; and
- copy a returned user as unsigned-decimal UID/GID plus username, comma-trimmed
  GECOS, and home; copy a group as name plus decimal GID.

The direct `internal/machine` group consumer must preserve its own branch:
typed unknown `uncloud` group logs/falls back to GID 0, any other native error
aborts, a found GID is parsed as signed 64-bit Go `int`, then the parent path is
chowned with unchanged UID and the same GID is passed to Unix socket creation.
This direct caller and `internal/fs` must use the same native group primitive;
do not create subtly different error mappings.

## Candidate comparison

| Candidate | Behavior fit | Decision |
| --- | --- | --- |
| `rustix = "=1.1.4"` (`default-features = false`, `std,process`) + project files parser + `std::chown` | Safe real-UID/GID calls plus the exact cgo-free files/current contract. On Linux x86_64/aarch64 rustix defaults to its raw backend, independent of glibc/musl. | **Selected for the cgo-free profile.** Forbid `use-libc` and `rustix_use_libc`; audit final feature unification. |
| Project-owned files parser + `std::chown` without an ID source | Cannot implement Go's current-user fallback because Rust `std` exposes no real UID/GID query. | Reject as incomplete. |
| Target/profile-only `libc = "=0.2.189"` (`default-features = false`) with a narrow safe wrapper | Exact symbols/types compile for Linux and both Apple targets; native Linux fields match Go; native macOS amd64/arm64 `getpwuid_r`/`getpwnam_r` results match Go byte-for-byte for current and Directory Service accounts. It is the standard low-level Rust OS-ABI crate and preserves Open Directory/glibc NSS without leaking into files-only builds. | **Selected for the native profile.** |
| `objc2-open-directory = "=0.3.2"` | Reaches Open Directory through a broader generated Objective-C/framework FFI surface, with a different query/error/string model from `getpwuid_r`. | Reject at behavior and integration-cost gates. |
| `nix = "=0.31.3"`, `users = "=0.11.0"`, `uzers = "=0.12.2"`, `pwd = "=1.4.0"`, `etc-passwd = "=0.2.2"` | Wrap native libc lookup but alter or hide the exact bounded-ERANGE/error/result-pointer contract; if shared, also make cgo-free Linux native by accident. | Reject for this exact boundary; direct target/profile-only libc is smaller and more controllable once its remaining gates pass. |
| General passwd/group parser crate | None evaluated preserves Go's Unicode trimming, invalid UTF-8, malformed-duplicate, signed-ID, streaming, late-I/O-error, and no-line-cap behavior. Adapting one is larger than the domain parser. | Reject at behavior and integration-cost gates. |
| Spawn `getent`, `id`, or platform tools | Adds executable discovery, environment, process, parsing, timeout, and cancellation behavior; native NSS is wrong for releases. | Reject at behavior and architecture gates. |

## Hard gates

| Gate | Evidence | Result |
| --- | --- | --- |
| Required behavior | Cgo-free and native contracts are specified, including cache/fallback, native name/group queries, both SSH CLI callers, and machine's group fallback. Native macOS current/name/UID behavior matches Go on amd64 and arm64 with a current account absent from `/etc/passwd`. | `pass` |
| License and security | rustix has the stated permissive licenses and safe ID APIs. libc 0.2.189 is MIT OR Apache-2.0; the exact isolated locks have no local RustSec finding. The probe demonstrates a narrow documented unsafe boundary. Final production wrapper code remains subject to ordinary package safety review. | `pass`; fresh exact critical dependency review clean |
| Platforms and targets | rustix/files code checked on Linux x86_64 and aarch64 targets and is GNU/musl independent when raw backend selection is enforced. libc checked and ran on Linux x86_64 and ran natively against Open Directory on macOS amd64/arm64. Linux arm64 runtime and final artifacts remain package/release acceptance, not an unresolved dependency API gate. | `pass` |
| Maintenance and Rust version | rustix 1.1.4 is Bytecode Alliance maintained, MSRV 1.63, and already exists transitively. crates.io reported 1.02B total/234M recent downloads. libc 0.2.189 is the current maximum stable release, Rust project maintained, MSRV 1.65, already locked exactly here, with 1.468B total/320M recent downloads. Rust 1.96 exceeds both MSRVs; `std::chown` is stable since 1.73. | `pass` |
| Architectural constraints | Explicit profiles prevent native linkage from contaminating files-only artifacts. Both paths are synchronous and share only the oracle-required cache; no runtime/process/service is added. Unsafe is isolated behind one safe native wrapper as required by policy. | `pass`; enforce again at package review |

## Exact Cargo configuration

Add these exact dependencies centrally. Both versions are already present in
the current workspace lockfile:

```toml
rustix = { version = "=1.1.4", default-features = false, features = ["std", "process"] }
libc = { version = "=0.2.189", default-features = false }
```

The files-profile workspace feature tree must contain neither rustix
`use-libc` nor an external `--cfg rustix_use_libc`. The libc dependency is
enabled only for the explicit native profile/target selection. No parser or
subprocess crate is authorized.

## Executable verification

All generated artifacts remained outside the repository.

Using the repository-pinned Go 1.26.1 toolchain through `mise`:

```text
CGO_ENABLED=0 go test -count=1 os/user       pass
CGO_ENABLED=1 go test -count=1 os/user       pass
CGO_ENABLED=0 go test -count=1 ./internal/fs pass
CGO_ENABLED=1 go test -count=1 ./internal/fs pass
```

A focused `user.Lookup("root\0suffix")` probe returned typed unknown-user with
`CGO_ENABLED=0` and resolved `root` with `CGO_ENABLED=1`. This confirms the
backend difference and why native NSS must not leak into released Linux
artifacts.

A statically built cgo-free Go 1.26.1 probe ran in disposable
`alpine:3.23.3` containers. It demonstrated all three otherwise easy-to-miss
current-user cases:

```text
passwd hidden after first Current(): Current and Lookup("root") returned the cached record
passwd absent, USER=fallback, HOME=/fallback: Current and Lookup returned uid/gid 0 fallback
passwd absent, USER/HOME absent then set: the original missing-variable error remained cached
```

An isolated Rust 1.96 probe pinned rustix 1.1.4 with only `std,process` and
called safe `getuid`/`getgid`. Formatting, all-target check, warnings-denied
Clippy, native execution, and `aarch64-unknown-linux-gnu` check passed. The
normal target trees for x86_64 and aarch64 contained `linux-raw-sys` and no
`libc`. `cargo audit 0.22.2 --no-fetch --deny warnings` scanned the exact
eight-package lockfile against 1,211 locally available advisories with no
finding. These results qualify the cgo-free selection; the separate native
evidence below resolves native-NSS scope.

A second isolated Rust 1.96 probe pinned libc 0.2.189 with default features
disabled and implemented the bounded `getpwuid_r` loop behind a safe API. Its
manifest, lockfile, and source SHA-256 values were respectively
`da887c0592f88fb4c43d2d53f13070dc37992a41f67f839dce517033e622ccfa`,
`e91a5780dd95489e126edadc98c8592350656085e08c1c3eff77fbff36e2f9bf`,
and `cf451eed73ca71005099f1c2d0a34f1e3f6e63c1bb71dcff281d204c9b4a62d7`.
These commands passed:

```text
cargo fmt --all --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked --quiet
cargo check --locked --target x86_64-apple-darwin
cargo check --locked --target aarch64-apple-darwin
cargo audit --no-fetch --deny warnings
```

Native Linux returned UID 1000, GID 1001, username `codex`, display name
`Codex`, and home `/home/codex`, exactly matching Go 1.26.1 cgo `Current()` on
the same host. The two-package lock audit found no issue in 1,211 available
advisories. Those earlier Apple cross-checks established only APIs/types and
compilation; the following later native run supplies the runtime evidence.

### Native macOS differential

GitHub Actions run
[`31529008863`](https://github.com/getployz/ployz-rust-port/actions/runs/31529008863)
was manually dispatched on public repository `getployz/ployz-rust-port`. Both
jobs checked out exact head `45ecd305c32860edace30dc07f1201d16d4166a9`;
`git diff` confirms that head's workflow and probe sources are byte-identical to
this research base.

| Job | Native evidence | Exact toolchain and dependency | Result |
| --- | --- | --- | --- |
| `93904233162`, `macOS NSS (arm64)` | `macos-15` runner; `uname -m=arm64`; macOS 15.7.7 (24G720); Rust host `aarch64-apple-darwin`; Go target `darwin/arm64`; `CGO_ENABLED=1` | Go 1.26.1; rustc/cargo 1.96.0; locked libc 0.2.189 | pass, five byte-identical rows |
| `93904233178`, `macOS NSS (x86_64)` | `macos-15-intel` runner; `uname -m=x86_64`; macOS 15.7.7 (24G720); Rust host `x86_64-apple-darwin`; Go target `darwin/amd64`; `CGO_ENABLED=1` | Go 1.26.1; rustc/cargo 1.96.0; locked libc 0.2.189 | pass, five byte-identical rows |

The arm64 artifact is `macos-nss-arm64` (ID `9116122391`, archive SHA-256
`4458c69e819d72a650fb486032499aeac712140019d66165dba0e6605862613d`);
the x86_64 artifact is `macos-nss-x86_64` (ID `9116132053`, archive SHA-256
`c27722aa0ec1e4303d8986d83bb0f186605240ab28085feb36f795aae18ef72e`).
The artifacts expire after 14 days, so this record retains the run, job,
source, and content hashes needed to audit the evidence after expiry.

On both machines, `runner` UID 501 was absent from `/etc/passwd` and resolved
through both `dscl` and `dscacheutil`; `_accessoryupdater` UID 278 supplied the
second Directory Service record. The probe compared current, current-name,
current-UID, directory-name, and directory-UID records. Go and Rust TSV SHA-256
values were identical within each architecture—
`c083ee946e87df290488817ad15d9ad17e849f0f4fc9b7fe9df0332a1d6ab850`
on arm64 and
`25eee1ad6267e3f39f36ae3d98f1e803d7423570345e9113a14c27db68c785bc`
on x86_64—and both `differential.diff` files were empty. This establishes
native status/result-pointer success, field copying, and actual Open Directory
resolution on both released macOS architectures. Error injection, cache races,
full SSH caller scenarios, and final artifact inspection remain package
acceptance work listed below; they do not require a different dependency.

Frozen production-call inventory:

```sh
rg -n 'fs\.(LookupUIDGID|Chown)|LookupUIDGID\(' \
  upstream/uncloud/cmd upstream/uncloud/internal \
  --glob '*.go' --glob '!**/*_test.go'
rg -n 'fs\.(ExpandHomeDir|Exists)' upstream/uncloud/cmd/uc --glob '*.go'
rg -n 'osuser\.Current|user\.Current' upstream/uncloud \
  --glob '*.go' --glob '!experiment/**'
rg -n 'machine\.NewMachine' upstream/uncloud --glob '*.go' \
  --glob '!experiment/**'
```

The first command found `internal/fs` account/ownership calls only in
`internal/machine/**`; the second found the portable filesystem calls in
`cmd/uc/main.go`; the third found the distinct current-user call in
`internal/sshexec/ssh.go`; and the fourth found production `Machine`
construction only in `internal/daemon/daemon.go`. Tracing both `machine add`
and `machine init` through `SSHDestination.Parse` and
`provisionOrConnectRemoteMachine` proves the SSH call is reachable with an
omitted user on released Linux and macOS CLIs.
`go list -deps` shows `internal/fs` and `internal/machine` are compiled into
both command dependency graphs, but package compilation is not execution of
otherwise unreachable internal functions.

On this Linux/amd64 host, exact `mise` defaults were:

```text
linux/amd64 CGO_ENABLED=1
linux/arm64 CGO_ENABLED=0
darwin/amd64 CGO_ENABLED=0
darwin/arm64 CGO_ENABLED=0
```

Building the `mise`-style Linux daemon outputs with `-buildvcs=false` solely
for the isolated worktree produced a dynamically linked amd64 ELF requiring
`libc.so.6` and a statically linked arm64 ELF. This proves the unqualified
development helper is host-dependent; it is not evidence against the explicit
cgo-free release rows.

Docker 29.1.3 ran `alpine:3.23.3` natively on amd64 and resolved `root` from
container account files. Its multi-architecture manifest contains arm64, but
arm64 execution failed with `exec format error` because this host has no
binfmt/QEMU handler. No arm64 runtime or final Rust artifact result is claimed.

## Required package acceptance

1. Files fixtures covering ASCII/Unicode whitespace, CRLF, comments, blank
   lines, invalid UTF-8, short rows, `+`/`-` rows, duplicate malformed/valid
   rows, embedded NUL, signed and plus-prefixed IDs, `i64` boundaries,
   overflow, and all-ones/modulo collisions.
2. `Current()`/`Lookup(name)` fixtures covering file-record preference over
   disagreeing environment, every UID-lookup failure falling back, real
   UID/GID capture, missing `$USER`/`$HOME` error wording, cached success and
   cached error across later file/environment changes, returned-record copies,
   exact-name shortcut, differing-name file lookup, and concurrent first-call
   one-winner behavior, including initialization through `internal/fs` then
   `sshexec` and the reverse to reject per-crate caches. Run cache cases in
   isolated processes.
3. Streaming fixtures with huge prefixes and discarded suffixes, newline-free
   final records, injected open/read errors before a match, and an error hidden
   in a matched unread suffix but observed while draining a nonmatch. Do not
   impose an undocumented line cap.
4. GNU/Linux differential fixtures where native NSS resolves an account absent
   from the files: the release profile must remain files-only while the
   cgo-equivalent development profile resolves it. Cover native user/group NUL
   truncation, typed unknown results, other errno, ERANGE/1-MiB behavior, and
   returned field copying.
5. User-plus-group, user-only, group-only, and both-omitted ownership;
   user-before-group short-circuiting; missing/broken-symlink paths;
   non-UTF-8/NUL paths; final-symlink following; `Interrupted` retry;
   permission denial; and no syscall after lookup failure.
6. Privileged disposable Linux amd64/arm64 fixtures for real ID changes,
   all-ones collision, set-ID/capability clearing, and ctime.
7. Native macOS amd64/arm64 fixtures for omitted-user `ssh+go` current-user
   resolution through both machine add and init: local and non-files/Open
   Directory identities where available,
   real-versus-effective UID, `sysconf` initial-size edges, `ERANGE` growth and
   1-MiB ceiling, no-result/`ENOENT`/other errno, native field copying, cached
   success/error, and the caller's error-to-empty-username behavior. A macOS
   build alone is not runtime parity evidence.
8. Direct `internal/machine` group fixtures: found `uncloud`, typed unknown
   fallback to root GID 0 with logging, non-unknown lookup failure, malformed
   returned GID, parent chown error, and propagation of the same GID to socket
   creation.
9. Target gating: macOS `uc` includes only the narrow native `Current()`
   boundary plus portable `ExpandHomeDir`/`Exists`; Linux daemon crates own
   `internal/fs` lookup/group/chown. Do not add unsupported non-Linux stubs or
   expose daemon-only account APIs merely to make unrelated crates compile.
10. Rust 1.96 formatting, targeted all-target tests/checks, warnings-denied
   Clippy, relevant Go oracle/differential tests, and final release-artifact
   inspection for Linux and macOS amd64/arm64, including selected backend and
   linkage.

## Review state and unlock impact

Earlier reviews corrected three independent scope errors: package compilation
alone does not make `internal/fs` lookup observable on macOS; a canonical
cgo-enabled Linux development build cannot be silently declared out of scope;
and the separately reachable omitted-user `ssh+go` paths do execute native
macOS `Current()`. They also exposed the cgo-free process-global `Current()`
behavior omitted by the first files-only proposal. This revision records both
CLI paths, the direct machine group caller, and the now-complete native macOS
evidence for the target-specific libc selection.

Fresh critical dependency review result: **CLEAN** on exact evidence-complete
commit `13f2ed21e5d4e68de2263f4756e3cce312e7932a`. The reviewer independently
downloaded the artifacts and logs; confirmed the head, native architectures,
toolchains, libc lock, Directory Service provenance, byte-exact TSVs, and
archive hashes; rechecked the Linux contracts, frozen callers, profile split,
versions/features/licenses/MSRVs, and package-versus-dependency gate boundary;
and confirmed sole-file scope with the oracle and probe sources unchanged.

Affected packages: future `crates/ployz-internal-fs`,
`crates/ployz-internal-sshexec`, and `crates/ployz-internal-machine` / matching
Go packages. The dependency decision is approved; the controller may update
the registry and recompute package readiness. The packages must share the
selected account primitives and process-global cache under the explicit
artifact profile.
