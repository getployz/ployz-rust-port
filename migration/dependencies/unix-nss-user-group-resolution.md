# Dependency decision: Unix account resolution and ownership changes

| Field | Value |
| --- | --- |
| Status | `blocked` — the released Linux behavior has a dependency-free pure-Rust design, but the shipped macOS `uc` resolves accounts through the native account database/Open Directory. No evaluated pure-Rust implementation can preserve that behavior and its error model. |
| Capability | Resolve a user name to UID and primary GID, resolve a group name to GID, preserve the artifact-specific account source and errors, and change pathname ownership while independently leaving either ID unchanged. |
| Selected dependency and exact version | **None.** No dependency is approved for the complete Linux/macOS capability. |
| Passing sub-design | Released Linux lookup: project-owned safe Rust parser with no dependency. Linux/macOS ownership: Rust 1.96 `std::os::unix::fs::chown` with no direct dependency. |
| Exact unblock condition | Either authorize a narrow macOS target-runtime FFI exception and freshly review direct `libc = "=0.2.189"` calls, or provide a pure-Rust macOS candidate that demonstrably preserves native/Open Directory lookup, byte and NUL behavior, typed failures, and concurrency. Dropping macOS or replacing native lookup with files-only lookup is not parity. |
| Research date | `2026-08-11` UTC |
| Research base | `c943d3e84914ae24fcce4cc2a19f92c5c04599bf` |
| Supersedes | The provisional Linux/macOS FFI split recorded through `0e7cf39c0a2768c0966f100fef3a01084f7ac025`; that split conflicts with the explicit pure-Rust target-runtime objective. |
| Request | Direct controller delegation for `upstream/uncloud/internal/fs` / future `crates/ployz-internal-fs`. |

## Decision

Do not approve a Unix NSS dependency or start `internal/fs` implementation yet.

The capability has two separable conclusions:

1. The released Linux artifacts can preserve lookup behavior with a small,
   dependency-free, safe Rust parser for `/etc/passwd` and `/etc/group`, plus
   `std::os::unix::fs::chown` for ownership. This sub-design passes the
   dependency gate but does not by itself unblock a package that must also ship
   on macOS.
2. The released macOS CLI uses Go's native account backend even when cgo is not
   available on Darwin. That backend reaches the macOS account database/Open
   Directory. Direct `libc`, `nix`, `users`, `uzers`, `pwd`, `etc-passwd`, and
   `objc2-open-directory` all reach native code through target-runtime FFI.
   Files parsing and subprocess adapters are pure-Rust application code but do
   not preserve the native lookup contract. No candidate passes both parity
   and the pure-Rust target-runtime gate.

Do not silently weaken this result to Linux-only support or files-only macOS.
Both would contradict the frozen release matrix and the port objective.

## Oracle and caller contract

- [`internal/fs/fs.go`](../../upstream/uncloud/internal/fs/fs.go) looks up a
  user before a group, parses returned textual IDs as Go `int`, treats an empty
  username/group as independently omitted only for `Chown`, calls `os.Chown`
  even when both are omitted, and wraps lookup, UID parse, GID parse, and chown
  failures separately. Its existing tests cover only home expansion.
- Direct callers require user-plus-group ownership, group-only ownership, and
  UID-plus-primary-GID rendering. See
  [`machine.go`](../../upstream/uncloud/internal/machine/machine.go),
  [`corroservice/config.go`](../../upstream/uncloud/internal/machine/corroservice/config.go),
  [`corromigrate/migrate.go`](../../upstream/uncloud/internal/machine/corromigrate/migrate.go),
  and
  [`caddyconfig/controller.go`](../../upstream/uncloud/internal/machine/caddyconfig/controller.go).
- Lookup and ownership are synchronous and uncached. There is no cancellation
  input. A native directory provider may block; the oracle does not add a
  timeout or cancellation path.
- Go 1.26.1's official
  [`lookup_unix.go`](https://go.dev/src/os/user/lookup_unix.go) selects the
  files backend on non-Darwin Unix when cgo is disabled. Its official
  [`cgo_lookup_unix.go`](https://go.dev/src/os/user/cgo_lookup_unix.go) selects
  native lookup for `(cgo || darwin)`, retries `ERANGE`, caps scratch storage at
  1 MiB, and treats `ENOENT` or success with a null result as not found.
- Apple's official
  [`getpwnam_r` manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/getpwnam.3.html)
  identifies the API as part of the standard C library. Apple's
  [Open Directory overview](https://developer.apple.com/library/archive/documentation/Porting/Conceptual/PortingUnix/additionalfeatures/additionalfeatures.html)
  states that Open Directory supplies local and remote administrative/user
  information, including LDAP-backed accounts.
- Rust's official
  [`std::os::unix::fs::chown`](https://doc.rust-lang.org/std/os/unix/fs/fn.chown.html)
  accepts `Option<u32>` IDs, follows the final symlink, documents privilege
  requirements, and preserves the kernel's set-ID-bit and capability effects.

## Exact target and build matrix

The account backend is an artifact property, not simply an operating-system
property.

| Frozen build path | Targets | Go account backend | Pure-Rust port consequence |
| --- | --- | --- | --- |
| GoReleaser `uc` | Linux amd64/arm64 | Explicit `CGO_ENABLED=0`; fixed files | No dependency; safe files parser. |
| GoReleaser `uncloudd` | Linux amd64/arm64 | Explicit `CGO_ENABLED=0`; fixed files | No dependency; safe files parser. |
| Docker `uncloudd` stage | Linux amd64/arm64 in Alpine | Explicit `CGO_ENABLED=0`; image files | No dependency; safe files parser. Final Rust artifact linkage remains an integration check, not a reason to add NSS. |
| GoReleaser `uc` | macOS amd64/arm64 | Explicit cgo plus Darwin-native backend; local and Open Directory accounts | **Blocked:** exact behavior requires a native API, but target-runtime FFI is not authorized by the pure-Rust objective. |
| `mise` `build:uncloudd-*` | Linux amd64/arm64 | No explicit CGO setting, so host/toolchain dependent. On this Linux/amd64 host the amd64 output used cgo/native glibc NSS and the arm64 cross-output used the files backend. | Development helper behavior is not a stable backend contract. If it is declared in scope, its native-NSS variant adds the same FFI conflict. |
| `mise` `uc` / ordinary `go run` | Host target | Native on cgo-enabled Linux; native on Darwin | Not a released artifact. If controller authority includes it, native Linux NSS also remains blocked under the pure-Rust gate. |

The frozen [`.goreleaser.yaml`](../../upstream/uncloud/.goreleaser.yaml) and
[`Dockerfile`](../../upstream/uncloud/Dockerfile) make the released rows
deterministic. [`mise.toml`](../../upstream/uncloud/mise.toml) does not set
`CGO_ENABLED` for its Linux build helpers, so it cannot justify one ambient
Rust backend.

## Approved Linux files-backend contract

The future implementation may use this design unchanged if the whole
capability is unblocked.

Use a small domain parser, not a general account-management abstraction. Open
the fixed files on each call. Reader-based private helpers provide tests. Do
not cache, consult `nsswitch.conf`, invoke libc/NSS, spawn `getent`, honor
environment overrides, or expand NIS `+`/`-` records.

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
   signed decimal `i64`, matching `strconv.Atoi` on every released 64-bit
   target. Skip a malformed or overflowing matched record so a later duplicate
   valid record may win. Return the first valid match. If none is valid, return
   typed unknown-user/unknown-group, not a parse error.
4. Preserve large-record streaming. Before the required colon count there is
   no invented line cap. Once enough fields are buffered, a match may return
   without reading its irrelevant suffix; a nonmatch must drain the suffix.
   Consequently an I/O error in an unread suffix is hidden after a match but
   observed while searching.
5. Distinguish file-open/read errors from not-found, with user/group context.
   Malformed records are not I/O errors. Ignore close errors, matching the
   oracle's deferred close.

### Linux byte and numeric edges

- Embedded NUL in the requested name is an ordinary byte on the files path.
  It is normally unknown, but a malformed account file containing that byte
  can match. Do not use `CString` on Linux.
- Accept signed values through `i64::MAX`, including negatives and values above
  `u32::MAX`. At ownership time, narrow with the oracle's modulo-2^32 result
  (`id as u32`); do not add validation the oracle lacks.
- `-1`, `4294967295`, and every value congruent to `u32::MAX` collide with the
  kernel all-ones "leave unchanged" sentinel. The named identity cannot be
  assigned by pathname `chown`; the other ID may still change. Preserve this
  limitation in tests.
- Account files are normally privileged, but malformed IDs and unbounded
  prefixes remain observable security/denial-of-service behavior. The parser
  contains no `unsafe`.

## Ownership contract

Use `std::os::unix::fs::chown` on Linux and macOS:

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
lookup and ownership. A group failure prevents ownership.

## Candidate comparison

| Candidate | Pure-Rust target runtime | Behavior result | Decision |
| --- | --- | --- | --- |
| Project-owned files parser + `std::chown` | Yes at application/dependency level; no direct FFI or target C dependency | Exact for released Linux files backend and pathname ownership | **Passing sub-design**, but cannot preserve shipped macOS lookup. |
| `libc = "=0.2.189"` on macOS | No; direct unsafe C ABI calls and `libSystem` linkage | Closest match to Go's `getpwnam_r`/`getgrnam_r`, including native directory providers and returned-code semantics | Reject under the current pure-Rust rule. This is the narrowest exception that could unblock macOS if explicitly authorized. |
| `objc2-open-directory = "=0.3.2"` | No; generated unsafe Objective-C/framework FFI | Reaches Open Directory, but exposes a different query/error/string model and cannot preserve arbitrary Unix name bytes or Go's embedded-NUL prefix behavior | Reject at pure-Rust and behavior gates. |
| `nix = "=0.31.3"`, `users = "=0.11.0"`, `uzers = "=0.12.2"`, `pwd = "=1.4.0"`, `etc-passwd = "=0.2.2"` | No; these wrap libc/native account APIs | Native Linux is wrong for released files-only artifacts; public APIs also collapse or alter required NUL/error/signed-ID behavior | Reject at scope and behavior gates. |
| Parse `/etc/passwd` and `/etc/group` on macOS | Yes | Loses non-files Open Directory accounts and provider failures shipped by `uc` | Reject at macOS parity gate. |
| Spawn `dscl`, `dscacheutil`, `id`, or `getent` | Rust process code is pure, but relies on external executables | Cannot pass embedded-NUL arguments, changes error/timeout/environment/process behavior, and has no cross-platform exact contract | Reject at behavior and architecture gates. |
| Implement Open Directory IPC/protocol directly | Hypothetically | No supported public wire protocol or maintained pure-Rust implementation was found; reimplementing private platform IPC is not a verifiable dependency choice | Reject as unsupported and unauditable. |

`cargo search` on the research date found `objc2-open-directory` as the only
credible Rust ecosystem binding specifically for the macOS framework. Its
published manifest and generated source declare external FFI bindings and
unsafe Objective-C methods. The other credible account crates publish libc
interfaces. Popularity cannot override a failed hard gate.

## Hard gates

| Gate | Evidence | Result |
| --- | --- | --- |
| Required behavior | Released Linux rows are files-only; shipped macOS `uc` is native/Open Directory. Files parsing preserves the former but not the latter. | `fail` for the complete capability |
| Pure-Rust architecture | Linux parser and `std::chown` require no direct target FFI. Every credible macOS native candidate uses C or Objective-C FFI; subprocess and private-protocol substitutes fail parity. | `fail` on macOS |
| License and security | The dependency-free Linux sub-design passes. No native dependency is selected, so no license/security claim can turn the complete decision into approval. Privileged ownership effects remain package/platform acceptance obligations. | `pass` for Linux sub-design; incomplete overall |
| Platforms | Released Linux amd64/arm64 and macOS amd64/arm64 are in scope. Only native Linux amd64 ran here. Cross-compilation is not macOS/Open Directory runtime evidence, and arm64 Docker execution failed without binfmt/QEMU. | `fail` for complete matrix |
| Maintenance and Rust version | Project code and Rust 1.96 `std` are sufficient for Linux. The only plausible macOS exception, `libc 0.2.189`, is not selectable without scope authority. | `pass` only for Linux sub-design |

Overall status is **blocked**. Do not add a Cargo dependency or unblock
`internal/fs` from this record.

## Executable verification

All generated artifacts remained outside the repository.

Using the repository-pinned Go 1.26.1 toolchain through `mise`:

```text
CGO_ENABLED=0 go test -count=1 os/user       pass
CGO_ENABLED=1 go test -count=1 os/user       pass
CGO_ENABLED=0 go test -count=1 ./internal/fs pass
CGO_ENABLED=1 go test -count=1 ./internal/fs pass
```

A focused `user.Lookup("root\0suffix")` probe produced typed unknown-user with
`CGO_ENABLED=0` and resolved `root` with `CGO_ENABLED=1`, proving that files and
native backends have observably different NUL handling.

On this Linux/amd64 host, the exact `mise` build environment reported:

```text
linux/amd64 CGO_ENABLED=1
linux/arm64 CGO_ENABLED=0
darwin/amd64 CGO_ENABLED=0
darwin/arm64 CGO_ENABLED=0
```

Building the two Linux daemon outputs with those defaults (plus
`-buildvcs=false` solely because this isolated worktree cannot be VCS-stamped)
produced a dynamically linked amd64 ELF requiring `libc.so.6`, and a statically
linked arm64 ELF. This confirms that the unqualified `mise` helper is
host-dependent and cannot define the released backend.

Docker 29.1.3 ran `alpine:3.23.3` natively on amd64 and resolved `root` from the
container account files. The multi-architecture manifest contains arm64, but
arm64 execution failed with `exec format error` because this host has no
binfmt/QEMU handler. No arm64 runtime result is claimed. No final Rust daemon
artifact exists yet, so this probe is environment evidence rather than final
release acceptance.

The earlier dependency-free ownership probe remains valid evidence for the
standard-library API: non-UTF-8 paths succeeded, NUL paths failed before a
syscall, `Some(u32::MAX)` matched the all-ones sentinel, final symlinks were
followed, and a same-owner call cleared set-ID bits. Rust 1.96 target checks
also proved `std::chown` declarations for Linux/macOS targets. These facts do
not solve native macOS account lookup.

## Required package acceptance after unblocking

1. Linux fixtures covering ASCII/Unicode whitespace, CRLF, comments, blank
   lines, invalid UTF-8, short rows, `+`/`-` rows, duplicate malformed/valid
   rows, embedded NUL, signed and plus-prefixed IDs, `i64` boundaries,
   overflow, and all-ones/modulo collisions.
2. Linux streaming fixtures with huge prefixes and discarded suffixes,
   newline-free final records, injected open/read errors before a match, and an
   error hidden in a matched unread suffix but observed while draining a
   nonmatch. Do not impose an undocumented line cap.
3. A GNU/Linux fixture where native NSS resolves an account absent from the
   files, proving the released backend remains files-only despite ambient GNU
   linkage.
4. User-plus-group, user-only, group-only, and both-omitted ownership;
   user-before-group short-circuiting; missing/broken-symlink paths;
   non-UTF-8/NUL paths; final-symlink following; `Interrupted` retry;
   permission denial; and no syscall after lookup failure.
5. Privileged disposable Linux amd64/arm64 fixtures for real ID changes,
   all-ones collision, set-ID/capability clearing, and ctime.
6. If a macOS FFI exception is authorized: native amd64/arm64 tests for local
   and non-files Open Directory accounts, typed not-found, returned errors,
   forced `ERANGE`, buffer cap, concurrency, embedded-NUL prefix behavior,
   safe copied scalar results, and privileged ownership effects. Cross-checks
   from Linux are insufficient.
7. Rust 1.96 formatting, targeted all-target tests/checks, warnings-denied
   Clippy, and final release-artifact inspection for every shipped target.

## Review state and unlock impact

This rewrite deliberately rejects the prior FFI selection instead of carrying
its D01-D03 approval path forward. The smallest remaining blocker is now the
explicit parity-versus-pure-Rust conflict for shipped macOS account lookup.

A fresh adversarial reviewer must confirm:

- the release/development matrix above;
- that no pure-Rust native macOS candidate was missed;
- that direct macOS `libc` is correctly rejected absent explicit authority;
- Linux parser, NUL, signed-ID, streaming, chown, and error fidelity; and
- that this record does not imply unverified macOS or arm64 runtime evidence.

Affected package: future `crates/ployz-internal-fs` / Go package
`upstream/uncloud/internal/fs`. The machine-level group lookup may reuse the
Linux sub-design only when it accepts the same artifact-specific contract.
