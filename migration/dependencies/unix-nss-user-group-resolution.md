# Dependency decision: Unix account resolution and ownership changes

| Field | Value |
| --- | --- |
| Status | `blocked` — fresh adversarial review rejected candidate `10cb71def83042c6db782896758f627a1dc87875`; D01 native macOS/privileged runtime evidence, D02 exact release-target/Docker artifact evidence, and D03 controller/human Linux-scope authority remain open |
| Capability | Resolve a user name to UID and primary GID, resolve a group name to GID, preserve the released target/build-specific account source and errors, and change pathname ownership while independently leaving either ID unchanged |
| Selected dependency and exact version | Linux lookup: none; macOS lookup only: `libc = "=0.2.189"`; Linux/macOS ownership: Rust 1.96 `std::os::unix::fs::chown` with no dependency |
| License | `MIT OR Apache-2.0` for the macOS-only `libc`; project-owned safe Rust and `std` elsewhere |
| Research date | `2026-08-11` UTC |
| Research base | `f927d1bf224142754bc2f818b88fdb46d7d70686` |
| Supersedes | Critically rejected proposal `58ea72e79a55c6ef56bf844ee503dd853588314f`; inspected, not applied |
| Request | Direct controller delegation for future `crates/ployz-internal-fs`; no request file or package packet exists at this base |

## Decision

Select a bounded Linux/macOS split, not a Unix-wide NSS wrapper:

- On `target_os = "linux"`, open and parse `/etc/passwd` and `/etc/group` in
  safe Rust with no dependency. This is the backend for all currently shipped
  Linux `uc` and `uncloudd` artifacts, on GNU and musl targets alike.
- On `target_os = "macos"`, call native `getpwnam_r`, `getgrnam_r`, and
  `sysconf` through exactly `libc` 0.2.189 in one crate-private safe boundary.
- On Linux and macOS, use `std::os::unix::fs::chown`; wrap it only to preserve
  lookup order, retry `Interrupted`, and add operation context.
- Do not use `cfg(unix)`. No behavior is approved here for BSD, AIX, Solaris,
  Android, Redox, WASI, or Windows.

This remains the researcher's provisional candidate, not an approved selection.
The completed adversarial review found that source analysis and cross-target
compilation do not close the runtime/platform and scope-authority gates recorded
as D01-D03 below.

This asymmetry is required behavior, not an optimization. Go's account backend
is selected at build time. Ordinary cgo-enabled Linux builds use libc/NSS, but
every shipped Linux release path in this repository disables cgo and therefore
uses Go's files-only backend. The macOS release uses the native backend. A
future Rust development mode intended to imitate ordinary cgo-enabled Linux
must make that backend an explicit artifact/profile choice and amend this
decision; ambient GNU linkage must not silently change released behavior.

## Oracle and artifact authority

- [`internal/fs/fs.go`](../../upstream/uncloud/internal/fs/fs.go) looks up the
  user before the group, parses textual IDs as Go `int`, treats an empty
  username/group as independently omitted only for `Chown`, calls `os.Chown`
  even when both are omitted, and wraps lookup, UID parse, GID parse, and chown
  failures separately. The package's existing test covers only home expansion.
- Direct callers require user+group ownership, group-only ownership, and
  UID+primary-GID rendering for the Corrosion container. See
  [`machine.go`](../../upstream/uncloud/internal/machine/machine.go),
  [`corroservice/config.go`](../../upstream/uncloud/internal/machine/corroservice/config.go),
  [`corromigrate/migrate.go`](../../upstream/uncloud/internal/machine/corromigrate/migrate.go),
  and
  [`caddyconfig/controller.go`](../../upstream/uncloud/internal/machine/caddyconfig/controller.go).
  The related socket setup also distinguishes unknown group from other lookup
  failures before performing a group-only chown.
- [`.goreleaser.yaml`](../../upstream/uncloud/.goreleaser.yaml) builds Linux
  `uc` and Linux-only `uncloudd` for amd64/arm64 with `CGO_ENABLED=0`, while the
  macOS `uc` builds use cgo. [`mise.toml`](../../upstream/uncloud/mise.toml) and
  [`Dockerfile`](../../upstream/uncloud/Dockerfile) pin Go 1.26.1; the Docker
  daemon build independently sets `CGO_ENABLED=0` and copies the result into an
  Alpine Docker-in-Docker image. The nightly workflow invokes this GoReleaser
  configuration.
- Go 1.26.1's official
  [`lookup_unix.go`](https://github.com/golang/go/blob/go1.26.1/src/os/user/lookup_unix.go)
  selects the files parser for non-Darwin Unix without cgo. Its official
  [`cgo_lookup_unix.go`](https://github.com/golang/go/blob/go1.26.1/src/os/user/cgo_lookup_unix.go)
  selects native lookup for cgo Unix and Darwin, uses `sysconf`, retries
  `ERANGE`, caps the buffer at 1 MiB, and treats `ENOENT` or success with no
  result as not found.
- Apple's official
  [`getpwnam_r` manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/getpwnam.3.html)
  and
  [`getgrnam_r` manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/getgrnam.3.html)
  specify caller-owned record/buffer storage, thread safety, zero on success,
  and `ERANGE` for insufficient storage. Apple describes
  [Open Directory](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/OSX_Technology_Overview/CoreOSLayer/CoreOSLayer.html)
  as access to local and network account databases, including LDAP and Active
  Directory providers.

### Supported artifacts and linkage

| Released artifact | Lookup contract | Dependency/linkage result |
| --- | --- | --- |
| Linux `uc`, amd64/arm64 | `/etc/passwd` and `/etc/group` only | Safe parser, no dependency. Dynamic GNU linkage must not activate NSS. |
| Linux `uncloudd`, amd64/arm64 | `/etc/passwd` and `/etc/group` only | Safe parser, no dependency. The normal release is a cgo-free Go artifact. |
| Docker `uncloudd`, amd64/arm64 | Files inside the Alpine-based image only | The lookup design introduces no glibc/NSS requirement. Final Rust musl/static packaging is still a release/integration authority gate. |
| macOS `uc`, Intel/Apple Silicon | Native account database/Open Directory | Target-only `libc` and normal `libSystem` linkage. Both targets compile; native runtime is unverified. |
| Ordinary cgo-enabled Linux development build | Native libc/NSS in Go, but not a shipped artifact | Not selected by this released-artifact decision. Add an explicit backend/profile only if the controller makes this a supported Rust artifact. |

## Required Linux files backend

Use a small domain parser, not a general account-management abstraction. The
production entry points open the fixed files on every call; private reader-based
helpers make the parser testable. Do not cache, consult `nsswitch.conf`, call
libc, spawn `getent`, honor environment overrides, or expand NIS `+`/`-`
records.

The parser must preserve these Go 1.26.1 rules:

1. Accept account names as Unix bytes (`&OsStr` is the natural boundary). Read
   logical lines as bytes. Apply Go `bytes.TrimSpace` behavior: trim the Unicode
   White_Space set at both ends while retaining invalid UTF-8 bytes; then skip
   blank lines and lines whose first remaining byte is `#`.
2. For users, accumulate through at least six colons and split at most seven
   ways. For groups, accumulate through at least three colons and split at most
   four ways. Compare field zero exactly; reject an empty name and records whose
   name begins `+` or `-`.
3. Validate a matched user's UID and primary GID, or a matched group's GID, as
   signed decimal `i64`, the `strconv.Atoi` range on every released 64-bit
   architecture. A malformed or overflowing matched record is skipped; a later
   duplicate valid record may win. Return the first valid match. If none is
   valid, return typed unknown-user/unknown-group, not a parse error.
4. Preserve the returned signed values through the natural Rust result model.
   The second `Atoi` in `internal/fs` cannot fail after the files backend has
   validated the fields, but UID-versus-primary-GID parse context remains a
   traceability case rather than a fabricated string round trip.
5. Preserve large-record behavior. Before the required colon count, growth is
   not subject to an invented line cap. After enough columns are buffered, a
   matching record can return without reading its irrelevant suffix; a
   nonmatch must drain that suffix. Thus an I/O error in an unread suffix is not
   observed after a match, while an error encountered while searching is.
6. File-open and read errors are distinct from not-found and receive
   user/group lookup context. Malformed records are not I/O errors. Drop/close
   errors are ignored, matching Go's deferred close.

Go's own 1.26.1 parser tests cover leading comments/space, `+`/`-` records,
missing fields, malformed IDs, negative IDs, and large group/member records.
The package must add the missing duplicate, overflow, I/O, NUL, invalid UTF-8,
and streaming-limit cases listed below.

### Linux byte and numeric edge contract

- A NUL in the requested account name is an ordinary byte on the files path.
  It is normally unknown, but a deliberately malformed account file containing
  that byte can match. Do not use `CString` or classify it as invalid input on
  Linux.
- Signed values including negatives and values above `u32::MAX` are accepted
  through `i64::MAX`. When ownership is attempted, narrow with the same
  two's-complement modulo-2^32 result as the released Go syscall path
  (`value as u32`); do not add validation the oracle lacks.
- Therefore `-1`, `4294967295`, and every value congruent to `u32::MAX` collide
  with the kernel all-ones “leave unchanged” sentinel. The requested identity
  cannot be assigned by pathname `chown`; the other ID may still change. This
  parity limitation is explicit and must be tested.

The account files are normally root-controlled, which limits attacker control
over malformed IDs and unbounded prefixes, but it does not erase these security
and denial-of-service semantics. The implementation must contain no `unsafe`.

## Required macOS native backend

Use direct reentrant calls behind one private safe module. Return only copied
numeric IDs; no libc record, string pointer, or scratch-buffer borrow may
escape.

1. Accept `&OsStr`, copy all bytes, and append one terminator. Deliberately do
   not use `CString::new`: Go's Darwin/native path also leaves embedded NUL
   bytes in that buffer, so `root\0suffix` is presented to libc as `root` and
   can resolve successfully. This prefix-truncation flaw differs from Linux's
   files path and is preserved, not normalized.
2. Use `sysconf(_SC_GETPW_R_SIZE_MAX)` or
   `sysconf(_SC_GETGR_R_SIZE_MAX)`. Exactly `-1` selects the Go fallback of
   1024 bytes; another nonpositive or greater-than-1-MiB suggestion selects
   1 MiB. On `ERANGE`, double with checked arithmetic and stop with a distinct
   buffer-limit error before exceeding 1 MiB.
3. On every attempt create a fresh initialized scratch buffer, a null result
   pointer, and `MaybeUninit::<libc::passwd/group>::zeroed()`. `libc`'s official
   [usage guidelines](https://docs.rs/libc/0.2.189/libc/#usage-guidelines)
   explicitly prohibit `MaybeUninit::uninit()` followed by whole-struct
   `assume_init` for libc structs because padding or future fields may remain
   uninitialized. Never call `assume_init`; after successful native lookup,
   read only `pw_uid`, `pw_gid`, or `gr_gid` through raw field pointers.
4. Classify the function's returned integer, not ambient `errno`: `0` plus
   null is typed not-found; `ENOENT` is typed not-found; `ERANGE` retries; any
   other nonzero code becomes `io::Error::from_raw_os_error(code)` with
   account context. A nonnull result on success is never dereferenced; if it
   is not the caller-owned record pointer, fail an internal ABI-invariant check.
5. Keep lookup synchronous, uncached, and safe for concurrent callers. Native
   directory lookup may block on IPC/network providers and has no portable
   cancellation mechanism; async callers must move it off latency-sensitive
   executor threads.

The rejected proposal failed this boundary by recommending uninitialized libc
records followed by `assume_init`, and it incorrectly selected native NSS for
all Unix targets. This repair removes all Linux lookup FFI and follows libc's
published initialization guidance on the sole native target.

## Ownership and returned outcomes

Rust's stable
[`std::os::unix::fs::chown`](https://doc.rust-lang.org/std/os/unix/fs/fn.chown.html)
accepts `&Path`, uses `Option<u32>` for independently unchanged IDs, follows a
final symlink, and documents privilege, set-ID-bit, and capability effects. Use
it on exactly Linux and macOS:

- preserve non-UTF-8 path bytes by accepting `&Path`;
- map an omitted name to `None`, and a found signed ID to `Some(id as u32)`;
- call `chown(path, None, None)` when both names are empty;
- retry only `io::ErrorKind::Interrupted`, matching Go `os.Chown`;
- use `chown`, never `lchown`, so the final symlink is followed and a broken
  link fails;
- treat a path containing NUL as a contextual chown/input failure without a
  syscall or prefix truncation. Go exposes `EINVAL`; Rust `std` naturally
  exposes `InvalidInput` with no raw errno. No caller performs errno matching,
  so the required contract is the distinct failure and original path context;
  do not add Linux libc merely to manufacture errno 22; and
- preserve kernel outcomes. Do not pre-authorize, retry permission errors,
  restore cleared set-user-ID/set-group-ID bits or file capabilities, suppress
  ctime changes, or emulate ownership in metadata.

Linux [`chown(2)`](https://man7.org/linux/man-pages/man2/chown.2.html) documents
the all-ones sentinel, symlink following, `CAP_CHOWN`/group restrictions,
set-ID-bit clearing, capability clearing, and filesystem/path errors. The
natural crate-private result model must retain:

| Operation | Required outcomes |
| --- | --- |
| Files lookup | Found signed IDs; typed unknown user/group; contextual file open/read error |
| Native lookup | Found native IDs; typed unknown user/group; contextual returned-code error; buffer limit; internal result-pointer invariant |
| Ownership | Contextual path/input or OS error retaining the `io::Error`; retry only `Interrupted` |

User lookup precedes group lookup, and either failure prevents all later work.
Group lookup never runs after user failure; chown never runs after either lookup
failure.

## Hard gates

| Gate | Evidence | Result |
| --- | --- | --- |
| Required behavior | Frozen release/build configuration establishes files-only shipped Linux and native macOS, but normal cgo-enabled Linux and `mise` workflows observe native NSS. The bounded split is technically coherent only after the supported-artifact authority chooses how that difference is scoped. | `blocked` by D03 |
| License and security | Linux lookup is safe project code. `libc` is `MIT OR Apache-2.0`; unsafe is confined to macOS `_r` calls/raw scalar reads with zeroed storage and bounded allocation. No shell, cache, Linux NSS module, or account-editing API is added. Privileged ownership side effects have not been exercised on every shipped architecture. | `blocked` by D01 despite passing source/license analysis |
| Platforms and targets | Linux amd64/arm64 and macOS amd64/arm64 are proposed. Cross-checks prove macOS declarations/type-checking only. Exact Rust release triples/linkage and running amd64/arm64 Docker artifacts are not defined or verified. No `cfg(unix)` claim. | `blocked` by D01 and D02 |
| Maintenance and Rust version | `libc` 0.2.189 was current on the research date, released 2026-07-21, declares Rust 1.65, and compiled under Rust 1.96 for both Apple architectures. `std::chown` has been stable since Rust 1.73. | `pass` |
| Architectural constraints | Synchronous, no runtime/process/service, fresh per-call state, minimal graph, natural Path/ID/error API. | `pass` |

Overall status remains **blocked**, not approved.

## Candidate comparison

Official crates.io metadata captured on 2026-08-11 is an adoption signal, not a
substitute for the hard gates.

| Candidate | Evidence and fit | Decision |
| --- | --- | --- |
| Safe Linux files parser + macOS-only [`libc` 0.2.189](https://crates.io/crates/libc/0.2.189) + `std::chown` | Exact released backend split; no Linux dependency/unsafe; only one highly adopted macOS binding crate. `libc` reported 1,466,932,142 total and 318,712,551 recent downloads, Rust 1.65 MSRV, and a fresh 2026-07-21 release. | **Selected provisionally:** smallest design that passes released behavior. |
| Direct `libc` on all Unix targets, rejected proposal | Native NSS mechanics can be correct, but Linux NSS observes providers and errors absent from every shipped cgo-free Linux artifact. `cfg(unix)` overclaims, and the proposed uninit/`assume_init` pattern violates libc's guidance. | Reject at behavior, platform, and safety gates. |
| [`nix` 0.31.3](https://crates.io/crates/nix/0.31.3), features `user,fs` | Popular (717,469,564 total / 158,901,341 recent downloads) and maintained, but native Linux lookup is wrong here. Its exact [lookup source](https://docs.rs/nix/0.31.3/src/nix/unistd.rs.html) uses uninit/`assume_init`, tests ambient `Errno::last()` instead of the `_r` return code on failure, and maps embedded-NUL names to `Ok(None)`. Rust `std` already supplies ownership. | Reject at behavior and exact-error/safety gates. |
| [`users` 0.11.0](https://crates.io/crates/users/0.11.0) | Native Linux lookup is wrong and `Option` collapses not-found, NUL, NSS/OS, and invariant errors. Last released in 2020. RustSec marks it [unmaintained with unpatched unsoundness and a privilege-escalation advisory](https://rustsec.org/packages/users.html). | Reject at behavior, security, and maintenance gates. |
| [`uzers` 0.12.2](https://crates.io/crates/uzers/0.12.2) | Maintained `users` fork, but still native on Linux and its public lookup API returns only `Option`, collapsing required errors and rejecting NUL instead of preserving target behavior. | Reject at behavior/error gate. |
| Parse account files on macOS too | No dependency or unsafe, but loses native/Open Directory accounts and provider failures shipped by macOS `uc`. | Reject at macOS behavior gate. |
| Spawn `getent` | Wrong for released files-only Linux, lacks an equivalent macOS contract, and adds executable discovery, environment, process, parsing, and cancellation surface. | Reject at behavior/platform/architecture gates. |

No general passwd-parser crate is selected. The required Go-compatible parser is
smaller than adapting a crate while retaining its unusual malformed-duplicate,
Unicode-trim, NUL, signed-ID, streaming, and I/O-error behavior.

## Required Cargo configuration

Only the integrator/dependency steward may add the workspace/crate manifest and
lockfile entry:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
libc = "=0.2.189"
```

Use libc's default `std` feature and no extra features. Do not add it under
`cfg(unix)` or as an unconditional dependency. Do not add `nix`, `users`,
`uzers`, a parser crate, or a subprocess dependency. A clean target probe
confirmed the Linux dependency tree contains no `libc`, while each macOS tree
contains only `libc 0.2.189` beyond the probe itself.

## Executable verification

All probe sources and targets were outside the repository under `/tmp`.

Using the repository-pinned Go 1.26.1 toolchain:

```text
CGO_ENABLED=0 go test -count=1 os/user                 pass
CGO_ENABLED=1 go test -count=1 os/user                 pass
CGO_ENABLED=0 go test -count=1 ./internal/fs           pass
CGO_ENABLED=1 go test -count=1 ./internal/fs           pass
```

A focused lookup probe resolved `root` in both modes. For
`root\0suffix`, cgo-disabled user/group lookup returned typed unknown errors,
while cgo-enabled lookup resolved `root`; this directly confirms the required
target/backend NUL split. `file`/`ldd` reported the cgo-disabled probe as a
static executable and the cgo-enabled probe as dynamically linked to glibc.
Both Go modes returned a `PathError` wrapping `EINVAL` for a NUL pathname.

A dependency-free Rust 1.96 ownership probe passed format and warnings-denied
Clippy. A syscall trace observed:

```text
chown(".../link", -1, -1) = 0
chown(".../target", -1, -1) = 0
chown(".../nonutf8-\377", -1, -1) = 0
```

The symlink target retained its IDs, `Some(u32::MAX)` generated the same
all-ones syscall sentinel as `None`, a non-UTF-8 path succeeded, and a NUL path
failed as Rust `InvalidInput` before a syscall. Chowning a mode-6755 file to its
existing UID/GID succeeded but changed its mode to 0755, confirming that even a
same-owner call has observable set-ID effects and must not be elided.

A clean Rust 1.96 target-only `libc` probe implemented zeroed storage,
returned-code classification, pointer validation, raw-field-only reads, checked
growth, and no `assume_init`. These all passed offline:

```text
cargo +1.96.0 fmt --check
cargo +1.96.0 clippy --locked --target x86_64-unknown-linux-gnu -- -D warnings
cargo +1.96.0 check  --locked --target x86_64-apple-darwin
cargo +1.96.0 clippy --locked --target x86_64-apple-darwin -- -D warnings
cargo +1.96.0 check  --locked --target aarch64-apple-darwin
cargo +1.96.0 clippy --locked --target aarch64-apple-darwin -- -D warnings
cargo +1.96.0 tree   --locked --target x86_64-unknown-linux-gnu
cargo +1.96.0 tree   --locked --target x86_64-apple-darwin
cargo audit --no-fetch --deny warnings
```

RustSec scanned the two-package all-target lock at advisory database commit
`d0861df1eab469d3c58d6b836ce48b5766e5f217` dated 2026-08-11 and reported no
vulnerability. Cross-compilation proves declarations/type-checking, not native
macOS runtime or Open Directory behavior. None of these focused probes built or
ran the final Rust release artifacts, so they do not satisfy D01 or D02.

## Required package acceptance

The future package must add tests for:

1. Linux file fixtures with ASCII/Unicode whitespace, CRLF, comments, blank
   lines, invalid UTF-8, empty/short rows, `+`/`-` rows, duplicate malformed then
   valid rows, duplicate valid rows, embedded NUL, signed/plus-prefixed IDs,
   `i64` boundaries/overflow, and all-ones/modulo collisions.
2. Linux streaming fixtures with huge prefixes, huge discarded suffixes,
   newline-free final records, injected open/read errors before a match, and an
   error in a suffix that is skipped after a match but observed while draining
   a nonmatch. Do not impose an undocumented line cap.
3. A GNU/Linux fixture with a successful non-files NSS user/group that remains
   unknown to the selected package backend, proving ambient dynamic linkage
   cannot change released lookup semantics.
4. macOS native current/local and directory-service user/group success,
   unknown results, `name\0suffix` prefix lookup, absent/excessive `sysconf`,
   forced `ERANGE`, non-`ERANGE` returned errors, `ENOENT`, cap exhaustion,
   unexpected result pointer, and concurrent calls on a real host.
5. User+group, user-only, group-only, and both-omitted calls; user-before-group
   short-circuiting; missing and broken-symlink paths; non-UTF-8 and NUL paths;
   final-symlink following; `Interrupted` retry; permission denial; and no
   syscall after lookup failure.
6. Privileged disposable Linux and macOS fixtures for actual UID/GID changes,
   all-ones identity collision, set-ID-bit/capability clearing, and ctime. These
   cases must not be weakened because ordinary CI is unprivileged.
7. Rust 1.96 formatting, targeted all-target tests/checks, and warnings-denied
   Clippy for Linux amd64/arm64 and macOS amd64/arm64, plus release artifact
   inspection proving Linux uses the files backend and Docker packaging has no
   accidental glibc/NSS requirement.

## Completed adversarial review and blocking gates

Fresh adversarial review of candidate commit
`10cb71def83042c6db782896758f627a1dc87875` returned **REJECTED**. The reviewer
did not invalidate the source-derived parser/native/chown design or the focused
probes above. It rejected approval because those probes cannot establish the
runtime artifact contract and because the normal cgo-enabled Linux scope has no
controller/human authority.

The record remains blocked on exactly these gates:

### D01 — native macOS and privileged ownership runtime evidence

Supply native, executable evidence on both shipped macOS architectures
(`x86_64-apple-darwin` and `aarch64-apple-darwin`) for local and non-files Open
Directory user/group lookup, typed not-found, returned errors, forced `ERANGE`,
concurrent calls, embedded-NUL prefix behavior, and the zeroed/raw-field unsafe
boundary. Cross-compilation is insufficient.

Also supply disposable privileged chown evidence on every shipped Linux and
macOS architecture. It must exercise real UID/GID changes, user-only,
group-only, both unchanged, all-ones collision, symlink following and broken
links, non-UTF-8/NUL paths, permission failures, `Interrupted` handling,
set-ID-bit/file-capability effects, and ctime. Record the host/runner, target,
commands, and observed metadata/errors; do not infer one architecture's result
for another.

### D02 — exact Rust release targets, linkage, and Docker artifacts

The controller/integrator must define the exact Rust target triples and linkage
for Linux `uc`, installed `uncloudd`, Docker `uncloudd`, and both macOS
architectures. “Linux”, “GNU/musl independent”, and a target-only Cargo graph
are not release artifact definitions.

Then build and inspect the actual Rust release outputs. For Docker, build both
amd64 and arm64 images from the intended release pipeline, inspect the embedded
binary's ELF interpreter/dynamic dependencies and architecture, start each
image on its matching architecture, and execute lookup plus ownership paths in
the Alpine runtime. Evidence must show the daemon starts, uses only the
container's `/etc/passwd` and `/etc/group`, does not acquire host/glibc NSS
behavior, and performs required ownership changes. Any emulation used must be
declared; emulation alone cannot establish architecture-specific privileged
filesystem behavior.

### D03 — authority for normal cgo-enabled Linux and `mise` scope

The frozen repository's normal same-host cgo-enabled Linux behavior and
`mise.toml` development workflows use native libc/NSS, unlike the proposed
always-files Rust Linux backend. Source inspection cannot decide whether those
observable development builds are in the port contract.

Obtain an explicit controller/human decision choosing one of:

1. released artifacts are authoritative and normal cgo-enabled Linux/`mise`
   builds are expressly excluded, so every supported Rust Linux artifact uses
   the files backend; or
2. those builds remain supported, in which case define an explicit native-NSS
   backend/profile, its Cargo feature and release-profile matrix, its `libc`
   target configuration, differential tests, and which artifacts select it.

Do not infer option 1 from GoReleaser or silently enable option 2 based on GNU
linkage. Option 2 materially expands the dependency/platform design and needs
fresh adversarial review.

After D01-D03 are recorded, a fresh adversarial reviewer must recheck their
closure plus parser fidelity, NUL/all-ones parity authority, the unsafe proof,
current `libc` license/release/RustSec evidence, and the exact artifact matrix.

Reviewer result: `rejected — candidate 10cb71def83042c6db782896758f627a1dc87875
cannot be approved until D01-D03 are closed and freshly rereviewed`.

Affected package: future `crates/ployz-internal-fs` / Go package
`upstream/uncloud/internal/fs`. The separate machine-level group lookup may
reuse this decision only if its package adopts the same exact artifact split
and edge contract.
