# Dependency decision: build-time VCS metadata

| Field | Value |
| --- | --- |
| Status | `approved` |
| Selected dependency | build dependency `vergen-gitcl = "=10.0.2"` |
| License | `MIT OR Apache-2.0` |
| Research date | `2026-08-11` UTC |
| Request | Controller delegation for future `crates/ployz-internal-version`; no request file exists at base `910fbea` |

## Capability

Obtain the full Git revision, dirty state, and commit timestamp at build time for
the port of `upstream/uncloud/internal/version`, while preserving the oracle's
injected-value precedence and invalid-dirty fallback. Builds must be safe in Git
worktrees, shallow clones, source archives, and environments without `git`; must
support Linux and macOS on amd64 and arm64 with Rust 1.96; and must not introduce
runtime or network access or permit Cargo directive injection.

## Oracle contract

The observable contract comes from:

- [`upstream/uncloud/internal/version/version.go`](../../upstream/uncloud/internal/version/version.go):
  `runtime/debug.BuildInfo` supplies `vcs.revision`, `vcs.modified`, and
  `vcs.time`; nonempty injected commit/date values replace those fallbacks, but
  injected dirty replaces the fallback only for exact `"true"` or `"false"`.
- `github.com/caarlos0/go-version@v0.2.2/version.go` in the Go module cache:
  missing metadata becomes `"unknown"`, `vcs.modified` maps to
  `"dirty"`/`"clean"`, and a valid VCS time is rendered as
  `YYYY-MM-DDTHH:MM:SS` without a zone suffix.
- [`upstream/uncloud/.goreleaser.yaml`](../../upstream/uncloud/.goreleaser.yaml):
  release builds explicitly inject version, commit, raw dirty boolean, commit
  date, and builder.
- Direct callers expose this data in the two version commands and use the
  version string in gRPC compatibility, metrics, and machine reporting. There
  are no package-local oracle tests.

This decision covers metadata acquisition only. It does not add branch, tag,
author, build-clock, dependency-tree, or remote-repository behavior.

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Full revision, tracked/untracked dirty state, commit timestamp, non-failing absence, injected precedence, invalid-dirty fallback | [`Gitcl` source](https://docs.rs/crate/vergen-gitcl/10.0.2/source/src/gitcl/mod.rs) uses `git rev-parse HEAD`, `git status --porcelain`, and `git log -1 --pretty=format:%cI`; its builder independently enables only SHA, dirty, and timestamp. The [published tests](https://docs.rs/crate/vergen-gitcl/10.0.2/source/tests/git_output.rs) cover shallow clones, missing worktrees, dirty/untracked combinations, and overrides. Local probes covered normal, shallow, linked-worktree, missing-command, override, and incremental cases. Package-specific injected inputs remain separate from `VERGEN_GIT_*`, as specified below. | `pass`, with required restamp and precedence configuration |
| License and security | Maintained acceptable license; no Cargo directive injection | The [published manifest](https://docs.rs/crate/vergen-gitcl/10.0.2/source/Cargo.toml) declares `MIT OR Apache-2.0`. [`vergen-lib`'s emitter](https://docs.rs/crate/vergen-lib/10.0.2/source/src/emitter.rs) filters LF from environment values, warnings, and watched paths before writing directives. Package-specific injected values are consumed with `option_env!` and never printed by `build.rs`; the required dynamic restamp path rejects CR/LF before printing. | `pass` |
| Platforms and targets | Linux/macOS, amd64/arm64 | This is a host-side Rust build dependency using `std::process::Command` and Git; no target code or FFI is linked. The selected source has Unix and Windows command paths and no architecture-specific code. Linux x86_64 was probed. macOS and arm64 remain CI acceptance checks. | `pass` |
| Maintenance and Rust version | Active release compatible with Rust 1.96 | [crates.io release metadata](https://crates.io/crates/vergen-gitcl/10.0.2) records release `10.0.2` on 2026-08-03, 6,467,743 total downloads, 2,418,945 recent downloads, and declared `rust-version = "1.96.0"`; the official API reported 59 reverse dependencies on 2026-08-11. | `pass` |
| Architectural constraints | Build-only, local-only, no runtime/network; absent Git/repository must not fail | Default features are empty. Keep `allow_remote` disabled. `Emitter::default()` has `fail_on_error = false`, so missing Git or repository warns and leaves requested values unset; a local probe with `SHELL=/bin/false` compiled and emitted `None` for all three values. `option_env!` turns absence into the oracle's unknown fallback. | `pass` |

## Candidate comparison

Adoption figures are from the official crates.io API on 2026-08-11. Reverse
dependency counts are direct crates.io counts. A download is supporting evidence,
not a quality guarantee.

| Candidate | Adoption and maintenance | Hard-gate result | Decision |
| --- | --- | --- | --- |
| `built 0.8.1` with `gix` | 54,051,002 total / 15,033,119 recent downloads; 235 reverse dependencies; released 2026-05-21; MIT; MSRV 1.87 | Exposes revision and dirty state but no commit timestamp. | Reject: fails required behavior despite highest adoption. |
| `vergen-gitcl 10.0.2` | 6,467,743 / 2,418,945; 59 reverse dependencies; released 2026-08-03; MIT OR Apache-2.0; MSRV 1.96 | Passes with the required configuration. Missing Git is non-fatal. The focused lock contained 29 packages. | **Select:** most widely downloaded lightweight member of the passing `vergen` backends, with the smallest build graph. |
| `vergen-gix 10.0.2` | 1,753,617 / 542,234; 62 reverse dependencies; released 2026-08-03; MIT OR Apache-2.0; MSRV 1.96 | Functionally passes and avoids the Git executable, but its focused lock contained 172 packages. It has the same incremental dirty/ref freshness limitation as `gitcl`. | Reject: materially higher build and audit cost without additional required behavior. |
| `vergen-git2 10.0.2` | 5,267,852 / 1,351,971; 34 reverse dependencies; released 2026-08-03; MIT OR Apache-2.0; MSRV 1.96 | Functionally viable, but adds native libgit2 FFI and its security/audit surface. | Reject: unnecessary FFI and would trigger critical second research. |
| `shadow-rs 2.0.0` | 9,871,102 / 1,887,714; 97 reverse dependencies; released 2026-04-23; `MIT AND Apache-2.0`; no declared MSRV | Generates a much broader metadata surface than requested; defaults to libgit2 and its CLI fallback can collapse missing Git to clean/default values. | Reject: excessive surface and weaker absence parity. |
| `git-version 0.3.9` | 22,019,646 / 4,162,809; 136 reverse dependencies; last released 2023-12-13; BSD-2-Clause; no declared MSRV | Produces describe/hash plus dirty suffix, but no separate commit timestamp and requires Git. | Reject: fails required behavior. |
| `build-data 0.3.3` | 1,498,748 / 239,908; 12 reverse dependencies; released 2025-06-03; Apache-2.0; no declared MSRV | Can run Git for revision, status, and `%ct`, and escapes command output, but missing Git returns errors for caller handling and its worktree rerun helper watches `$GIT_DIR` paths rather than shared refs. | Reject: lower adoption, undeclared MSRV, and more caller-owned edge handling. |
| Standard-library `build.rs` | No dependency or runtime cost | Viable with fixed-argument `std::process::Command` calls, explicit status/error handling, timestamp normalization, worktree discovery, safe directive emission, and deliberate rerun behavior. | Reject: recreates maintained parsing/fallback/sanitization behavior; project policy prefers the popular passing idiom. A small standard-library restamp trigger is still required around the selected crate. |

Primary candidate manifests and sources were inspected from the exact crates.io
packages downloaded by `cargo info`; no secondary crate review was used to
establish behavior.

## Selected integration

Only the integrator may add this to the workspace manifest and lockfile:

```toml
[build-dependencies]
vergen-gitcl = { version = "=10.0.2", default-features = false }
```

Required features: **none**. In particular, do not enable `allow_remote`,
`vcs_info`, `build`, `cargo`, `cargo_metadata`, `emit_and_set`, `rustc`, `si`, or
`unstable`.

The package build script configures only the requested values:

```rust
let git = vergen_gitcl::Gitcl::builder()
    .sha(false)
    .dirty(true)
    .commit_timestamp(true)
    .build();

vergen_gitcl::Emitter::default()
    .add_instructions(&git)?
    .emit()?;
```

`dirty(true)` is mandatory: Git's [porcelain status](https://git-scm.com/docs/git-status)
includes tracked, staged, and untracked changes by default, matching Go's VCS
stamping behavior. Do not call `fail_on_error()` or `default_on_error()`; absent
metadata must remain unset so the package can map it to `"unknown"`.

### Required restamp behavior

Vergen watches `HEAD`, one ref path, and `build.rs`, but it does not watch the
working tree/index. Its ref path is also incomplete for a linked worktree because
the branch ref lives in the common Git directory. Cargo states that once any
`rerun-if-changed` instruction is emitted it watches only the named paths; see
the [Cargo build-script change-detection rules](https://doc.rust-lang.org/cargo/reference/build-scripts.html#change-detection).

Therefore the build script must also emit a watched path that is deliberately
absent under `OUT_DIR`, forcing this small build script to rerun on every Cargo
invocation. This is required parity, not optional optimization: it is the only
simple trigger that observes a newly created untracked file as well as
tracked/staged changes, cleanup, commits, detached HEAD changes, and linked
worktree shared refs. Reject non-UTF-8 `OUT_DIR` and reject either CR or LF in the
rendered path before printing it. The sentinel file must never be created.

A focused probe confirmed Cargo reported the package dirty because the sentinel
was missing, reran the build script, and refreshed both dirty state and a linked
worktree's SHA after an otherwise empty commit. Cost risk: this intentionally
reruns the package build script and recompiles its crate each Cargo invocation;
the selected CLI backend minimizes that cost relative to the gix backend.

### Injected-value precedence

Do **not** use `VERGEN_GIT_SHA`, `VERGEN_GIT_DIRTY`, or
`VERGEN_GIT_COMMIT_TIMESTAMP` as the release injection contract. Vergen treats
any present dirty value as a complete override, so an invalid value would erase
the oracle's automatically discovered fallback.

Use separate package-specific compile-time variables (for example
`PLOYZ_GIT_COMMIT`, `PLOYZ_GIT_DIRTY`, and `PLOYZ_BUILD_DATE`) and consume them
directly with `option_env!`; do not copy or print their values from `build.rs`.
Cargo has automatically tracked `env!`/`option_env!` inputs since 1.46, per the
[Cargo reference](https://doc.rust-lang.org/cargo/reference/build-scripts.html#rerun-if-env-changed).

At runtime construction of the immutable version info:

1. Start with optional `VERGEN_GIT_SHA`, `VERGEN_GIT_DIRTY`, and
   `VERGEN_GIT_COMMIT_TIMESTAMP` fallback values.
2. Normalize the generated timestamp to the oracle's fallback form
   `YYYY-MM-DDTHH:MM:SS`; missing or malformed generated data becomes
   `"unknown"`.
3. Replace commit and date only when their package-specific injected strings are
   nonempty.
4. Replace dirty only for exact injected `"true"` or `"false"`; empty or any
   other string retains the generated fallback. Map the final boolean to
   `"dirty"` or `"clean"`, otherwise `"unknown"`.

Direct `option_env!` use preserves injected bytes without routing attacker input
through Cargo's line-oriented build-script protocol. Vergen's own generated
SHA, boolean, and timestamp are constrained, and its emitter removes LF before
printing. Any additional dynamic Cargo directive must reject CR and LF rather
than merely stripping them.

## Known limitations and risks

- `git` and a Unix shell are build-host tools for the selected backend. Missing
  or unusable tools safely yield unset values, but automatic discovery is then
  unavailable; release injection still works.
- An always-missing rerun sentinel trades incremental build performance for
  correct dirty/ref freshness. Omitting it is a parity failure.
- The selected patch release is recent and sets its MSRV exactly at the
  workspace's Rust 1.96 floor. Pin exactly `10.0.2` and rerun all acceptance
  checks before any upgrade.
- Linux x86_64 was exercised locally. Linux arm64 and both macOS architectures
  require CI/host verification. The build dependency is host-only and contains
  no architecture-specific or target-linked code.
- The commit timestamp supplied by Vergen is UTC with fractional seconds and a
  `Z`; the package must test its normalization to the oracle's fallback format.
- A source archive or absent repository intentionally has unknown automatic
  revision/dirty/date unless the release pipeline injects explicit values.

## Verification and acceptance checks

Focused Rust 1.96 probes performed on Linux x86_64:

- normal committed repository: full SHA, `false`, and UTC commit timestamp;
- depth-one shallow clone: all three values present and correct;
- linked worktree: all three values present; without supplementation both
  `vergen-gitcl` and `vergen-gix` stayed stale after an empty commit;
- linked worktree plus always-missing sentinel: SHA refreshed after the next
  empty commit without `cargo clean`;
- missing command simulated with `SHELL=/bin/false`: build succeeded and all
  three values were unset;
- explicit Vergen override containing
  `fixed\ncargo:rustc-cfg=INJECTED`: emitted SHA became the single safe value
  `fixedcargo:rustc-cfg=INJECTED`, with no second directive;
- focused lock comparison: 29 packages for `vergen-gitcl` versus 172 for
  `vergen-gix`.

The package acceptance suite must additionally verify, without `cargo clean`:

1. clean -> modified tracked -> staged -> restored clean;
2. clean -> new untracked -> removed untracked -> clean;
3. ordinary checkout, linked worktree, detached HEAD, and depth-one clone;
4. absent `.git`, missing `git`, and unusable `$SHELL` all compile and map to
   unknown rather than failing;
5. generated revision is the full current SHA and generated timestamp is the
   current commit time normalized to the oracle format;
6. nonempty injected commit/date win, empty values do not, exact dirty booleans
   win, and invalid/empty dirty retains every generated fallback state;
7. injected LF, CR, and CRLF payloads cannot create an extra Cargo directive or
   cfg; dynamic watched paths fail closed on CR/LF;
8. Rust 1.96 formatting, targeted tests/check/Clippy, and host builds on Linux
   amd64/arm64 and macOS amd64/arm64.

Suggested focused commands after implementation:

```sh
cargo +1.96.0 check -p ployz-internal-version --all-targets
cargo +1.96.0 test -p ployz-internal-version --all-targets
cargo +1.96.0 clippy -p ployz-internal-version --all-targets --all-features -- -D warnings
```

## Dependency implications

- One exact, featureless **build dependency** only; no normal/runtime dependency.
- The resolved build graph includes `vergen`, `vergen-lib`, `time`, `bon`,
  `anyhow`, and their transitive dependencies. It does not include libgit2/gix
  or a network transport.
- The build host should normally provide Git, but its absence is supported and
  non-fatal.
- Only the integrator/dependency steward may update root `Cargo.toml`,
  `Cargo.lock`, and the dependency registry.

## Review

This capability is local build metadata, not networking, storage,
cryptography, a runtime, container control, unsafe FFI, or a production service.
No critical second dependency research is required. An independent read-only
research pass corroborated the Vergen family and highlighted the incremental
dirty-state requirement; the final backend choice follows official adoption and
focused build-cost evidence.

Affected package: future `crates/ployz-internal-version` (Go oracle package
`upstream/uncloud/internal/version`). No package packet exists at base `910fbea`.
