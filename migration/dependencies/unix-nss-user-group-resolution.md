# Dependency decision: Unix account resolution and ownership changes

| Field | Value |
| --- | --- |
| Status | `approved` — no external dependency. Observable shipped callers of account lookup and ownership run only in Linux `uncloudd`, whose release and Docker builds use `CGO_ENABLED=0`; use a project-owned safe Rust files parser plus Rust 1.96 `std::os::unix::fs::chown`. |
| Capability | Resolve a user name to UID and primary GID, resolve a group name to GID, preserve the shipped account source and errors, and change pathname ownership while independently leaving either ID unchanged. |
| Selected dependency and exact version | None. Rust 1.96 standard library only. |
| Required configuration | Linux-only lookup/ownership implementation. Do not add `libc`, `nix`, `users`, `uzers`, a passwd parser, or a subprocess dependency. |
| License | Project license and Rust standard library; no third-party package is selected. |
| Research date | `2026-08-11` UTC |
| Research base | `c943d3e84914ae24fcce4cc2a19f92c5c04599bf` |
| Supersedes | The provisional Linux/macOS FFI split recorded through `0e7cf39c0a2768c0966f100fef3a01084f7ac025`. That proposal inferred macOS lookup reachability from package compilation, but frozen shipped callers do not execute lookup or ownership in macOS `uc`. |
| Request | Direct controller delegation for `upstream/uncloud/internal/fs` / future `crates/ployz-internal-fs`. |

## Decision

Approve a dependency-free Linux design:

- Parse `/etc/passwd` and `/etc/group` in project-owned safe Rust for
  `LookupUIDGID` and the name-resolution portion of `Chown`.
- Use `std::os::unix::fs::chown` for the pathname ownership operation.
- Compile these account/ownership entry points only for Linux callers. Keep
  `ExpandHomeDir` and `Exists` portable; those are the only `internal/fs`
  behaviors reached by shipped macOS `uc`.
- Do not add a native NSS/Open Directory backend. It would add behavior and FFI
  for a path that no shipped macOS caller executes.

This scope follows observable reachability, not Go package membership. The
macOS CLI imports packages that eventually compile `internal/fs`, but its only
calls are `fs.ExpandHomeDir` and `fs.Exists` in `cmd/uc/main.go`. Every
production call to `fs.LookupUIDGID` or `fs.Chown` is in machine/daemon code.
The only shipped daemon is Linux `uncloudd`, and all of its release paths set
`CGO_ENABLED=0`.

If a future shipped macOS daemon or CLI path begins calling lookup/ownership,
return to the dependency gate. Native/Open Directory behavior cannot be added
under this approval.

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

## Exact target and build matrix

| Frozen build path | Targets | Account backend for observable lookup callers | Port decision |
| --- | --- | --- | --- |
| GoReleaser `uncloudd` | Linux amd64/arm64 | Explicit `CGO_ENABLED=0`; fixed files | Selected safe Rust files parser plus `std::chown`. |
| Docker `uncloudd` | Linux amd64/arm64 in Alpine | Explicit `CGO_ENABLED=0`; container files | Selected safe Rust files parser plus `std::chown`. |
| GoReleaser Linux `uc` | Linux amd64/arm64 | `CGO_ENABLED=0`, but shipped CLI callers do not reach lookup/ownership | No account backend is needed by the CLI path. |
| GoReleaser macOS `uc` | macOS amd64/arm64 | Go compiles a native account backend, but shipped CLI callers do not reach lookup/ownership | No Rust NSS/Open Directory dependency. Portable `ExpandHomeDir` and `Exists` remain required. |
| `mise` `build:uncloudd-*` | Linux amd64/arm64 | Host-dependent because the task does not set CGO. On this Linux/amd64 host, amd64 was native glibc NSS and arm64 cross-build was files-only. | Not a shipped artifact under this delegation's observable-shipped-behavior scope. If promoted to a supported artifact, return to the gate for an explicit backend profile. |
| Ordinary development `go run` | Host target | Native on cgo-enabled Linux and Darwin | Not a shipped lookup/ownership contract. |

The last two rows are recorded to prevent ambient linkage from silently
changing the Rust design. They do not override the deterministic release rows.

## Required Linux files-backend contract

Use a small domain parser, not a general account-management abstraction. Open
the fixed files on every call. Reader-based private helpers provide tests. Do
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
are synchronous and uncached; there is no cancellation input, timeout, or
background work to orphan.

The natural Rust error model must distinguish:

| Operation | Required outcomes |
| --- | --- |
| Files lookup | Found signed IDs; typed unknown user/group; contextual file-open/read error |
| Ownership | Contextual path/input or OS error retaining the `io::Error`; retry only `Interrupted` |

## Candidate comparison

| Candidate | Behavior fit | Decision |
| --- | --- | --- |
| Project-owned safe Rust files parser + `std::chown` | Matches all observable shipped lookup/ownership callers; no dependency, direct FFI, cache, process, runtime, or service. | **Selected.** |
| `libc = "=0.2.189"` / direct NSS | Native Linux sees NSS providers and errors absent from cgo-free released daemon artifacts. macOS lookup is unreachable in shipped CLI paths. | Reject: wrong on Linux and unnecessary on macOS. |
| `nix = "=0.31.3"`, `users = "=0.11.0"`, `uzers = "=0.12.2"`, `pwd = "=1.4.0"`, `etc-passwd = "=0.2.2"` | Wrap native libc lookup, change NUL/error/signed-ID behavior, and add dependency/unsafe surface. | Reject at behavior and architecture gates. |
| General passwd/group parser crate | None evaluated preserves Go's Unicode trimming, invalid UTF-8, malformed-duplicate, signed-ID, streaming, late-I/O-error, and no-line-cap behavior. Adapting one is larger than the domain parser. | Reject at behavior and integration-cost gates. |
| Spawn `getent`, `id`, or platform tools | Adds executable discovery, environment, process, parsing, timeout, and cancellation behavior; native NSS is wrong for releases. | Reject at behavior and architecture gates. |

## Hard gates

| Gate | Evidence | Result |
| --- | --- | --- |
| Required behavior | Frozen production callers reach lookup/ownership only in Linux daemon/machine code. Released daemon and Docker artifacts are cgo-free files-only builds. The parser and `std::chown` preserve that reachable contract. | `pass` |
| License and security | No dependency or package-owned unsafe. Root-controlled account files limit attacker control but do not erase malformed-record/large-prefix behavior. Privileged ownership effects remain explicit acceptance cases. | `pass` |
| Platforms and targets | Observable lookup/ownership targets are Linux amd64/arm64. Parser code is GNU/musl independent; `std::chown` is available on both targets. Native amd64 evidence passed; arm64 execution remains package/release acceptance, not a dependency uncertainty. | `pass` |
| Maintenance and Rust version | Project code plus Rust 1.96 stable `std::chown` (stable since Rust 1.73). No third-party maintenance risk. | `pass` |
| Architectural constraints | Synchronous, no cache/runtime/process/service, per-call files, natural Path/ID/error API, no FFI abstraction. | `pass` |

## Cargo configuration

No workspace dependency or lockfile entry is required. The future package must
not add an account/NSS/parser/process crate under this approval.

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
2. Streaming fixtures with huge prefixes and discarded suffixes, newline-free
   final records, injected open/read errors before a match, and an error hidden
   in a matched unread suffix but observed while draining a nonmatch. Do not
   impose an undocumented line cap.
3. A GNU/Linux fixture where native NSS resolves an account absent from the
   files, proving ambient GNU linkage cannot change the selected backend.
4. User-plus-group, user-only, group-only, and both-omitted ownership;
   user-before-group short-circuiting; missing/broken-symlink paths;
   non-UTF-8/NUL paths; final-symlink following; `Interrupted` retry;
   permission denial; and no syscall after lookup failure.
5. Privileged disposable Linux amd64/arm64 fixtures for real ID changes,
   all-ones collision, set-ID/capability clearing, and ctime.
6. Target-gating evidence: macOS `uc` builds with portable `ExpandHomeDir` and
   `Exists` while Linux-only daemon crates own lookup/ownership callers. Do not
   add an unsupported non-Linux lookup stub merely to make unrelated crates
   compile.
7. Rust 1.96 formatting, targeted all-target tests/checks, warnings-denied
   Clippy, relevant Go oracle/differential tests, and final release-artifact
   inspection for Linux amd64/arm64.

## Review state and unlock impact

The first rereview of `b1a8da8` rejected an inferred macOS lookup requirement:
frozen callers prove that package compilation did not make it observable. This
revision scopes the capability to shipped reachable behavior and selects the
dependency-free design.

A fresh adversarial reviewer must confirm caller reachability, release scope,
Linux parser/chown fidelity, hard-gate closure, evidence honesty, and sole-file
scope before the controller marks the decision approved.

Affected package: future `crates/ployz-internal-fs` / Go package
`upstream/uncloud/internal/fs`. This decision should unlock its dependency gate
once the fresh review is clean. Machine-level direct `os/user` lookup may reuse
the files backend only if its own shipped caller/build scope is identical.
