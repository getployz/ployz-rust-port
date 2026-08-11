# Dependency decision: Unix account resolution and ownership changes

| Field | Value |
| --- | --- |
| Status | `human-decision-required`. Released Linux daemon artifacts have a cgo-free files contract, but the canonical `mise` Linux/amd64 daemon build and ordinary cgo-enabled development builds have native glibc NSS behavior. The controller does not authorize excluding those builds or silently changing their account source. |
| Capability | Resolve a user name to UID and primary GID, resolve a group name to GID, preserve the shipped account source and errors, and change pathname ownership while independently leaving either ID unchanged. |
| Selected dependency and exact version | No complete selection pending the supported-build decision. For the cgo-free files profile only: conditionally approve `rustix = "=1.1.4"` with `default-features = false` and features `std,process`, plus a project-owned safe parser and Rust 1.96 `std::os::unix::fs::chown`. |
| Required configuration | Decide whether host-dependent `mise`/ordinary development builds are supported behavior. A files profile must forbid rustix's `use-libc` feature and `--cfg rustix_use_libc`; a native-NSS profile requires a separate reviewed backend and an explicit build/profile mapping. |
| License | Conditional rustix selection: `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`; project parser and Rust standard library otherwise. No native-NSS dependency is approved. |
| Research date | `2026-08-11` UTC |
| Research base | `c943d3e84914ae24fcce4cc2a19f92c5c04599bf` |
| Supersedes | The provisional Linux/macOS FFI split recorded through `0e7cf39c0a2768c0966f100fef3a01084f7ac025`. That proposal inferred macOS lookup reachability from package compilation, but frozen shipped callers do not execute lookup or ownership in macOS `uc`. |
| Request | Direct controller delegation for `upstream/uncloud/internal/fs` / future `crates/ployz-internal-fs`. |

## Decision required

The controller must choose and record one supported-build contract before this
capability can unblock implementation:

1. **Release-artifact scope.** Explicitly exclude host-dependent `mise` and
   ordinary cgo-enabled development builds from observable parity for this
   capability. Use the cgo-free profile below for Linux daemon artifacts.
2. **Release plus development scope.** Preserve native glibc NSS behavior for
   cgo-enabled Linux builds as well as the cgo-free release profile. This needs
   an explicit Cargo feature/profile-to-artifact mapping and a fresh critical
   dependency/FFI decision for the native backend. Do not let ambient host
   linkage choose a backend.

The current migration contract preserves frozen observable behavior and only
excludes `upstream/uncloud/experiment/**`; it grants no authority for option 1.
Therefore this record cannot choose that option by inference.

For the cgo-free profile, conditionally approve this exact design:

- obtain the real UID and GID with safe
  `rustix 1.1.4` process APIs using its raw Linux backend;
- reproduce Go's process-global `Current()` cache and environment fallback;
- parse `/etc/passwd` and `/etc/group` in project-owned safe Rust for remaining
  lookups; and
- use `std::os::unix::fs::chown` for pathname ownership.

The macOS CLI imports packages that eventually compile `internal/fs`, but its
only calls are `fs.ExpandHomeDir` and `fs.Exists` in `cmd/uc/main.go`. No
shipped macOS caller executes lookup or ownership, so Open Directory is not a
current blocker. If reachability changes, return to the dependency gate.

## Frozen caller and artifact authority

- [`internal/fs/fs.go`](../../upstream/uncloud/internal/fs/fs.go) looks up a
  user before a group, parses returned textual IDs as Go `int`, treats empty
  username/group values as independently omitted only for `Chown`, calls
  `os.Chown` even when both are omitted, and wraps lookup, UID parse, GID parse,
  and chown failures separately. Its existing tests cover only home expansion.
- [`cmd/uc/main.go`](../../upstream/uncloud/cmd/uc/main.go) calls only
  `fs.ExpandHomeDir` and `fs.Exists`. The CLI uses `machine` package constants
  and token/client helpers, but it never constructs the daemon `Machine` or
  calls the account/ownership entry points.
- All production account/ownership callers are:
  [`machine.go`](../../upstream/uncloud/internal/machine/machine.go),
  [`corroservice/config.go`](../../upstream/uncloud/internal/machine/corroservice/config.go),
  [`corromigrate/migrate.go`](../../upstream/uncloud/internal/machine/corromigrate/migrate.go),
  and
  [`caddyconfig/controller.go`](../../upstream/uncloud/internal/machine/caddyconfig/controller.go).
  `internal/daemon/daemon.go` is the only production constructor of
  `machine.NewMachine`; `cmd/uncloudd` owns that daemon path.
- [`.goreleaser.yaml`](../../upstream/uncloud/.goreleaser.yaml) builds
  `uncloudd` only for Linux amd64/arm64 and explicitly sets `CGO_ENABLED=0`.
  [`Dockerfile`](../../upstream/uncloud/Dockerfile) independently sets
  `CGO_ENABLED=0` for the Linux daemon copied into the Alpine image.
- Go 1.26.1's official
  [`lookup_unix.go`](https://go.dev/src/os/user/lookup_unix.go) selects the
  files backend on non-Darwin Unix when cgo is disabled. Its source defines the
  byte parser, malformed-record handling, streaming behavior, typed unknown
  errors, and fixed `/etc/passwd` and `/etc/group` paths used below.
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

## Exact target and build matrix

| Frozen build path | Targets | Account backend for observable lookup callers | Port decision |
| --- | --- | --- | --- |
| GoReleaser `uncloudd` | Linux amd64/arm64 | Explicit `CGO_ENABLED=0`; fixed files, including cached-current shortcut/fallback | Conditional cgo-free profile: rustix IDs, safe Rust files parser, `std::chown`. |
| Docker `uncloudd` | Linux amd64/arm64 in Alpine | Explicit `CGO_ENABLED=0`; container files, including cached-current shortcut/fallback | Same conditional cgo-free profile. |
| GoReleaser Linux `uc` | Linux amd64/arm64 | `CGO_ENABLED=0`, but shipped CLI callers do not reach lookup/ownership | No account backend is needed by the CLI path. |
| GoReleaser macOS `uc` | macOS amd64/arm64 | Go compiles a native account backend, but shipped CLI callers do not reach lookup/ownership | No Rust NSS/Open Directory dependency. Portable `ExpandHomeDir` and `Exists` remain required. |
| `mise` `build:uncloudd-*` | Linux amd64/arm64 | Host-dependent because the task does not set CGO. On this Linux/amd64 host, amd64 was native glibc NSS and arm64 cross-build was files-only. | **Unresolved:** canonical repository build, so it cannot be excluded without explicit authority. |
| Ordinary development `go run` / `go build` | Host target | Native on cgo-enabled Linux and Darwin | **Unresolved:** current port rules do not exclude ordinary supported development behavior. |

The last two rows prevent ambient linkage from silently changing the Rust
design. Their inclusion is the reason this decision remains gated.

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
   record. The file record wins even when `$USER` or `$HOME` disagrees.
3. On *any* UID lookup failure (unknown, open, read, or malformed-only result),
   discard that error and build a fallback from the real UID, real GID from
   `rustix::process::getgid()`, `$USER`, and `os.UserHomeDir` behavior (`$HOME`
   on this Unix profile). The display name is empty.
4. Accept the fallback only when UID, username, and home are all nonempty.
   Otherwise cache and return `user: Current requires cgo or <missing> set in
   environment`, listing `$USER`, `$HOME`, or both in that order. The cached
   error is just as sticky as a cached success.
5. `Lookup(name)` returns a copy of the cached current record only when its
   username exactly matches `name`; if current initialization failed or the
   name differs, perform the ordinary name lookup. Group lookup never uses the
   current-user cache.

This shortcut is observable even though `internal/fs` does not call
`user.Current` directly: its username lookup calls `os/user.Lookup`.

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

## Candidate comparison

| Candidate | Behavior fit | Decision |
| --- | --- | --- |
| `rustix = "=1.1.4"` (`default-features = false`, `std,process`) + project files parser + `std::chown` | Safe real-UID/GID calls plus the exact cgo-free files/current contract. On Linux x86_64/aarch64 rustix defaults to its raw backend, independent of glibc/musl. | **Conditionally selected for the cgo-free profile.** Forbid `use-libc` and `rustix_use_libc`; audit final feature unification. |
| Project-owned files parser + `std::chown` without an ID source | Cannot implement Go's current-user fallback because Rust `std` exposes no real UID/GID query. | Reject as incomplete. |
| `libc = "=0.2.189"` / direct NSS | Could preserve cgo-enabled Linux NSS, but changes cgo-free releases unless explicitly profiled and introduces reviewed native ABI/unsafe concerns. macOS lookup remains unreachable in current callers. | Not approved; research only if development scope is retained. |
| `nix = "=0.31.3"`, `users = "=0.11.0"`, `uzers = "=0.12.2"`, `pwd = "=1.4.0"`, `etc-passwd = "=0.2.2"` | Wrap native libc lookup, change NUL/error/signed-ID behavior, and add dependency/unsafe surface. | Reject at behavior and architecture gates. |
| General passwd/group parser crate | None evaluated preserves Go's Unicode trimming, invalid UTF-8, malformed-duplicate, signed-ID, streaming, late-I/O-error, and no-line-cap behavior. Adapting one is larger than the domain parser. | Reject at behavior and integration-cost gates. |
| Spawn `getent`, `id`, or platform tools | Adds executable discovery, environment, process, parsing, timeout, and cancellation behavior; native NSS is wrong for releases. | Reject at behavior and architecture gates. |

## Hard gates

| Gate | Evidence | Result |
| --- | --- | --- |
| Required behavior | The cgo-free contract is specified, including the `Current()` shortcut/cache/fallback. Canonical `mise` and ordinary cgo-enabled builds instead expose native NSS, and no authority excludes them. | `human-decision-required` |
| License and security | Conditional rustix has the stated permissive licenses, safe public ID APIs, and no RustSec findings in the exact probe lockfile. Root-controlled files do not erase malformed-record/large-prefix risks. Native NSS remains unevaluated. | `conditional pass` |
| Platforms and targets | Conditional rustix/files code checked on Linux x86_64 and aarch64 targets and is GNU/musl independent when raw backend selection is enforced. Arm64 runtime remains package/release acceptance. | `conditional pass` |
| Maintenance and Rust version | rustix 1.1.4 is Bytecode Alliance maintained, MSRV 1.63, and already exists transitively in this workspace; Rust 1.96 exceeds its MSRV. crates.io reported 1.02B total and 234M recent downloads on the research date, supporting its idiomatic/adopted status. `std::chown` is stable since 1.73. | `pass` |
| Architectural constraints | Cgo-free profile is synchronous, has only the oracle-required process-global cache, and adds no runtime/process/service. Whether a separately selected native profile is allowed remains unresolved. | `human-decision-required` |

## Conditional Cargo configuration

If release-artifact scope is explicitly authorized, add the following exact
direct dependency centrally; it is already present transitively in the current
workspace lockfile:

```toml
rustix = { version = "=1.1.4", default-features = false, features = ["std", "process"] }
```

The final workspace feature tree must contain neither rustix `use-libc` nor an
external `--cfg rustix_use_libc`. This record does not authorize a native NSS,
account parser, or subprocess dependency.

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
finding. These results qualify only the conditional cgo-free selection; they
do not resolve native-NSS scope.

Frozen production-call inventory:

```sh
rg -n 'fs\.(LookupUIDGID|Chown)|LookupUIDGID\(' \
  upstream/uncloud/cmd upstream/uncloud/internal \
  --glob '*.go' --glob '!**/*_test.go'
rg -n 'fs\.(ExpandHomeDir|Exists)' upstream/uncloud/cmd/uc --glob '*.go'
rg -n 'machine\.NewMachine' upstream/uncloud --glob '*.go' \
  --glob '!experiment/**'
```

The first command found account/ownership calls only in `internal/machine/**`;
the second found the two portable calls in `cmd/uc/main.go`; the third found
the production `Machine` construction only in `internal/daemon/daemon.go`.
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
   one-winner behavior. Run cache cases in isolated processes.
3. Streaming fixtures with huge prefixes and discarded suffixes, newline-free
   final records, injected open/read errors before a match, and an error hidden
   in a matched unread suffix but observed while draining a nonmatch. Do not
   impose an undocumented line cap.
4. A GNU/Linux fixture where native NSS resolves an account absent from the
   files, proving ambient GNU linkage cannot change the selected backend.
5. User-plus-group, user-only, group-only, and both-omitted ownership;
   user-before-group short-circuiting; missing/broken-symlink paths;
   non-UTF-8/NUL paths; final-symlink following; `Interrupted` retry;
   permission denial; and no syscall after lookup failure.
6. Privileged disposable Linux amd64/arm64 fixtures for real ID changes,
   all-ones collision, set-ID/capability clearing, and ctime.
7. Target-gating evidence: macOS `uc` builds with portable `ExpandHomeDir` and
   `Exists` while Linux-only daemon crates own lookup/ownership callers. Do not
   add an unsupported non-Linux lookup stub merely to make unrelated crates
   compile.
8. Rust 1.96 formatting, targeted all-target tests/checks, warnings-denied
   Clippy, relevant Go oracle/differential tests, and final release-artifact
   inspection for Linux amd64/arm64.

## Review state and unlock impact

Earlier reviews corrected two independent scope errors: package compilation
does not make macOS lookup observable, while a canonical cgo-enabled Linux
development build cannot be silently declared out of scope. They also exposed
the cgo-free process-global `Current()` behavior omitted by the first files-only
proposal. This revision keeps the accepted caller analysis, specifies that
cache/fallback contract, and leaves the actual build-scope choice with the
controller/user.

A fresh adversarial reviewer must confirm the unresolved authority gate,
current-user fidelity, conditional rustix hard gates, caller reachability,
parser/chown fidelity, evidence honesty, and sole-file scope.

Affected package: future `crates/ployz-internal-fs` / Go package
`upstream/uncloud/internal/fs`. It remains dependency-blocked until the
supported-build decision is explicit. Machine-level direct `os/user` lookup
may reuse a selected backend only if its caller/build profile is identical.
