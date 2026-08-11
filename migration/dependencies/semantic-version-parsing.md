# Dependency decision: `semantic-version-parsing`

| Field | Value |
| --- | --- |
| Status | `approved` |
| Capability | Parse, normalize, format, and compare untrusted binary/metadata versions for the frozen `internal/grpcversion` behavior |
| Selected dependency | `semver = { version = "=1.0.28", default-features = false, features = ["std"] }` |
| License | `MIT OR Apache-2.0` |
| Research date | `2026-08-11` UTC |
| Request | No request file was present; capability was delegated directly for `upstream/uncloud/internal/grpcversion` |
| Blockers | None for the scoped package contract |

## Required oracle contract

- The frozen package uses `github.com/Masterminds/semver v1.5.0` only through
  `NewVersion`, `MustParse`, `String`, and `LessThan`: [source](../../upstream/uncloud/internal/grpcversion/interceptor.go),
  [tests](../../upstream/uncloud/internal/grpcversion/interceptor_test.go), and
  [module pin](../../upstream/uncloud/go.mod).
- Missing, empty, syntactically invalid, or overflowing versions become
  `0.0.0`; the error's type and text are not exposed. Parsed versions are
  compared only against the stable constants `0.20.0`. The current version's
  normalized string is emitted in metadata and error text.
- The directly used Masterminds 1.5.0 grammar accepts an optional lowercase
  `v`, one to three ASCII-decimal core components, leading zeroes, omitted
  minor/patch components, ASCII prerelease/build identifiers, and numeric
  prerelease identifiers with leading zeroes. It rejects uppercase `V`,
  whitespace, empty identifiers, a fourth core component, non-ASCII
  identifiers, and any core number above signed 64-bit range. Its `String`
  drops `v`, fills missing core parts, canonicalizes core numbers, and preserves
  prerelease/build text. These facts are defined by the frozen
  [`version.go`](https://github.com/Masterminds/semver/blob/v1.5.0/version.go)
  parser and formatter and its
  [`version_test.go`](https://github.com/Masterminds/semver/blob/v1.5.0/version_test.go).
- Numeric core ordering, prerelease-before-stable ordering, and build metadata
  being ignored are required. No frozen `grpcversion` path compares two
  prereleases with the same core. The unrelated Caddy caller prefilters tags to
  `2.x.x`, so it needs only ordinary stable numeric ordering:
  [`caddy.go`](../../upstream/uncloud/pkg/client/caddy.go).

## Primary-source evidence

- `semver` 1.0.28's
  [`Version` documentation](https://docs.rs/semver/1.0.28/semver/struct.Version.html)
  exposes an owned, synchronous value with `parse`, canonical display, public
  core/prerelease/build fields, and `cmp_precedence`, which explicitly ignores
  build metadata. Its strict parser rejects missing core components, leading
  core zeroes, and leading-zero numeric prereleases; the compatibility policy
  below deliberately closes only those oracle-required differences.
- The published
  [1.0.28 manifest](https://docs.rs/crate/semver/1.0.28/source/Cargo.toml.orig)
  declares Rust 1.68, `MIT OR Apache-2.0`, default feature `std`, and only an
  optional Serde dependency. With defaults disabled and `std` enabled, the
  resolved normal graph contains no transitive package.
- The exact
  [parser source](https://docs.rs/crate/semver/1.0.28/source/src/parse.rs),
  [precedence implementation](https://docs.rs/crate/semver/1.0.28/source/src/impls.rs),
  and [tests](https://docs.rs/crate/semver/1.0.28/source/tests/test_version.rs)
  establish strict ASCII syntax, checked numeric parsing, numeric prerelease
  ordering, and structured parse errors.
- The official [crates.io record](https://crates.io/api/v1/crates/semver) on
  2026-08-11 reported 901,638,639 total downloads, 191,526,817 recent
  downloads, non-yanked 1.0.28 published 2026-04-04, and MSRV 1.68. The
  [reverse-dependency API](https://crates.io/api/v1/crates/semver/reverse_dependencies?page=1&per_page=1)
  reported 3,408 dependents.
- Official [repository metadata](https://api.github.com/repos/dtolnay/semver)
  on 2026-08-11 reported 672 stars, 142 forks, an unarchived repository, and a
  latest push on 2026-06-24. The published crate identifies source commit
  `7625c7aa3f0e8ba21e099d1765bcebcb72aa8816`.

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Preserve the Masterminds 1.5.0 accepted/rejected inputs, normalized wire string, invalid-to-zero policy, stable-threshold comparisons, prerelease precedence, and build-insensitive comparison used by `grpcversion` | `semver::Version` supplies the owned value and correct precedence engine. The narrow compatibility parse policy below reproduced all 1,559 generated oracle parse/format/`0.20.0` comparison cases exactly. | `pass` |
| License and security | Permissive, Apache-2-compatible license; safe parsing of untrusted metadata; no unnecessary native or runtime surface | Selected crate is `MIT OR Apache-2.0`, has no resolved transitive package with `std` only, no build script or FFI, and needs no application `unsafe`. `cargo audit --no-fetch --deny warnings` found no advisory in the two-package probe lock using 1,211 RustSec advisories at DB commit `d0861df1eab469d3c58d6b836ce48b5766e5f217` dated 2026-08-11. | `pass` |
| Platforms and targets | Linux amd64/arm64 daemon; macOS amd64/arm64 and Windows amd64 CLI builds | Rust 1.96 locked offline all-target checks passed for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, and `x86_64-pc-windows-gnu`. The crate is pure Rust and documents `no_std` support when `std` is absent. | `pass` |
| Maintenance and Rust version | Maintained release compatible with workspace Rust 1.96 | 1.0.28 was published 2026-04-04, its repository was pushed 2026-06-24, it declares MSRV 1.68, and exact consumers compiled on Rust 1.96. | `pass` |
| Architectural constraints | Synchronous in-process parsing, deterministic comparison, small graph, no I/O/runtime/global state | `Version`/`Prerelease`/`BuildMetadata` are owned synchronous values. Selected graph is one third-party package and `std` is its only enabled feature. | `pass` |

## Candidate comparison

Counts are official crates.io snapshots from 2026-08-11. Differential counts
below are diagnostics over the same deliberately adversarial 1,559-case corpus,
not popularity or quality scores; a difference means direct parsing did not
match the complete scoped Go parse/format/threshold tuple.

| Candidate | Behavior and fit | Adoption and maintenance | Build/license | Disposition |
| --- | --- | --- | --- | --- |
| [`semver` 1.0.28](https://crates.io/crates/semver/1.0.28) | Correct owned semantic-version model and explicit build-insensitive `cmp_precedence`; strict direct parsing differed on 675 cases, all addressed for the scoped contract by the narrow input compatibility policy. | 901,638,639 total / 191,526,817 recent downloads; 3,408 reverse dependencies; current 2026 release and repository activity. | `MIT OR Apache-2.0`; MSRV 1.68; one-package selected graph. | **Selected:** overwhelmingly idiomatic Rust choice and the simplest passing graph after the required boundary normalization. |
| [`lenient_semver` 0.4.2](https://crates.io/crates/lenient_semver/0.4.2) | Directly fills omitted parts and accepts `v`, but also accepts uppercase `V`, surrounding whitespace, fourth components, and several non-oracle spellings; it rewrites leading-zero prereleases. Direct parsing differed on 1,047 corpus cases. | 3,553,192 / 572,529 downloads; 24 reverse dependencies; last release 2021-06-06 and last source push 2023-02-13. | `MIT OR Apache-2.0`; undeclared MSRV; adds parser/builder crates plus `semver`. | Rejected: materially over-permissive and stale, with lower adoption and a larger graph than the selected boundary policy. |
| [`node-semver` 2.2.0](https://crates.io/crates/node-semver/2.2.0) | Accepts leading zeroes and `v`, but requires three core parts, also accepts `V`, `v 1.2.3`, and attached prereleases, canonicalizes numeric build text, caps core values at JavaScript `MAX_SAFE_INTEGER`, and caps input at 256 bytes. It differed on 782 corpus cases. | 982,898 / 296,692 downloads; 37 reverse dependencies; released/pushed 2025-02-07. | Apache-2.0; MSRV 1.70; brings Nom, Miette, Serde, Thiserror, and related transitives. | Rejected: JavaScript-specific limits and syntax fail required behavior; much heavier and far less adopted. |
| [`versions` 7.0.0](https://crates.io/crates/versions/7.0.0) | Its `SemVer` is strict, uses `u32` core values, and neither accepts `v` nor omitted parts; differed on 676 corpus cases. Its broader descriptive `Version` accepts formats outside this protocol. | 10,427,988 / 2,048,245 downloads; 39 reverse dependencies; released 2025-02-24; repository pushed 2026-06-23. | MIT; MSRV 1.85; Nom dependency. | Rejected: core range and parser contract fail, with no integration advantage. |
| [`semver_rs` 0.2.0](https://crates.io/crates/semver_rs/0.2.0) | NPM-style versions/ranges rather than a Masterminds-compatible primitive. | 18,441 / 623 downloads; two reverse dependencies; last release 2021-11-22. | crates.io reports `non-standard`/no declared license expression and no MSRV. | Rejected at license, maintenance, and adoption gates. |

## Selected integration

Use exactly:

```toml
semver = { version = "=1.0.28", default-features = false, features = ["std"] }
```

Use the crate's natural owned `Version` and `Version::cmp_precedence` model.
Do not use derived `Ord` for compatibility decisions because `semver`'s total
ordering includes build metadata while Masterminds `Compare` ignores it.

At the untrusted string boundary, implement one package-local compatibility
parser with this deliberately narrow policy:

1. Validate the complete input against Masterminds 1.5.0's grammar: optional
   lowercase `v`; one to three ASCII-decimal core parts; optional nonempty
   dot-separated ASCII alphanumeric/hyphen prerelease and build identifiers;
   no whitespace, uppercase prefix, Unicode, empty identifiers, trailing text,
   or fourth core part.
2. Parse every supplied core part as nonnegative `i64`; reject overflow. Fill
   omitted minor/patch with zero and canonicalize core digits. This preserves
   the Go dependency's signed-64-bit boundary rather than `semver`'s wider
   `u64` boundary.
3. Retain a canonical wire string containing that normalized three-part core
   plus the original prerelease/build identifier text. This preserves leading
   zeroes in prerelease/build output exactly.
4. For the precedence `Version`, remove leading zeroes from all-numeric
   prerelease identifiers (`00` becomes `0`) and parse the otherwise unchanged
   canonical value with `semver::Version::parse`. This is equivalent for every
   comparison actually made by `grpcversion`: core ordering and prerelease
   versus the stable `0.20.0` threshold. Leave build metadata unchanged.
5. Represent a parsed protocol version as the semantic `Version` plus its wire
   string. Invalid input maps to the pre-parsed `0.0.0` value and wire string,
   matching `parseVersionOrZero`/`extractVersion`. Emit the retained wire
   string; compare with `cmp_precedence`.

This boundary policy is protocol compatibility, not a general permissive
semantic-version parser and not an imitation of the Go dependency API.

## Verification

Two isolated probes were run outside the repository with Go 1.22 and Rust
1.96.0:

- The Go side imports the frozen Masterminds 1.5.0 source. The Rust side pins
  `semver` 1.0.28 and implements the integration policy above.
- A generated 1,559-case matrix varied prefixes, one/two/three core parts,
  leading zeroes, signed-64-bit boundaries, prerelease/build identifiers,
  invalid separators, trailing characters, whitespace, Unicode, canonical
  display, and comparison with `0.20.0`. The selected integration produced zero
  tuple differences. Direct `semver`, `lenient_semver`, `node-semver`, and
  `versions::SemVer` results supplied the candidate diagnostic counts above.
- A separate 20,736-pair matrix characterized arbitrary same/different-core
  prerelease comparisons. Normalized `semver::cmp_precedence` differed on 160
  pairs, all outside the frozen call pattern, including the oracle's
  non-antisymmetric comparisons between numerically equal spellings such as
  `0` and `00`, and its lexical treatment of numeric prerelease identifiers
  above `u64::MAX`.
- The exact selected graph passed formatting, locked offline all-target check,
  warnings-denied Clippy, runtime assertions, all five target-family checks,
  feature inspection, license inspection, and the offline RustSec audit.

Reproduction commands for the minimal selected consumer are:

```sh
cargo +1.96.0 generate-lockfile --offline
cargo +1.96.0 fmt --check
cargo +1.96.0 check --locked --offline --all-targets
cargo +1.96.0 clippy --locked --offline --all-targets -- -D warnings
cargo +1.96.0 run --locked --offline
cargo +1.96.0 check --locked --offline --all-targets --target x86_64-unknown-linux-gnu
cargo +1.96.0 check --locked --offline --all-targets --target aarch64-unknown-linux-gnu
cargo +1.96.0 check --locked --offline --all-targets --target x86_64-apple-darwin
cargo +1.96.0 check --locked --offline --all-targets --target aarch64-apple-darwin
cargo +1.96.0 check --locked --offline --all-targets --target x86_64-pc-windows-gnu
cargo audit --no-fetch --deny warnings
```

## License and security notes

- The exact normal dependency graph is only `semver` 1.0.28. Serde is not
  enabled. The two published license files implement its `MIT OR Apache-2.0`
  expression.
- Parsing is local CPU/memory work with no I/O, subprocess, network, native
  library, build script, FFI, or runtime. Application code needs no `unsafe`.
- This record does not approve future compatible-range upgrades. Re-evaluate
  behavior, MSRV, features, and RustSec status before changing the exact pin.

## Known limitations

- Direct `Version::parse` is intentionally too strict for the frozen wire
  protocol; all untrusted protocol versions must pass through the boundary
  policy above. Constants such as `0.20.0` may use direct strict parsing.
- `semver::Version` uses `u64` core numbers while the oracle uses `i64`; the
  boundary parser must enforce `i64::MAX`.
- The retained wire string is necessary because normalizing a leading-zero
  numeric prerelease for `semver` precedence would otherwise change emitted
  metadata/error text.
- Approval covers exactly the comparisons exercised by `grpcversion` and the
  stable numeric Caddy tags identified above. It does not approve arbitrary
  Masterminds-compatible prerelease-to-prerelease ordering. Any new caller
  needing that must expand this dependency decision and preserve or explicitly
  waive the characterized v1.5.0 comparator flaws.

## Review

Mandatory fresh adversarial review: pending on the exact decision commit.

Affected package: `upstream/uncloud/internal/grpcversion`. No package packet was
present at research time. The existing stable-only `pkg/client/caddy.go` use is
also compatible with the selected dependency but is not the blocking package
for this request.
