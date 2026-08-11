# Dependency decision: `Docker/OCI image-reference parsing`

| Field | Value |
| --- | --- |
| Status | `approved` |
| Capability | Docker/OCI image-reference parsing compatible with the frozen Docker-reference oracle |
| Selected dependency | `oci-spec = { version = "=0.10.0", default-features = false, features = ["distribution"] }` |
| License | `Apache-2.0` |
| Research date | `2026-08-11` UTC |
| Request | No request file was present; capability was delegated directly for `upstream/uncloud/internal/cli/tui` |
| Blockers | None |

## Primary-source evidence

- The oracle calls `reference.ParseDockerRef`, styles `FamiliarName` and the tag separately, uses `FamiliarString` otherwise, and styles the original input unchanged when parsing fails: [`upstream/uncloud/internal/cli/tui/style.go`](../../upstream/uncloud/internal/cli/tui/style.go). The frozen Go dependency is `github.com/distribution/reference v0.6.0`: [`upstream/uncloud/go.mod`](../../upstream/uncloud/go.mod).
- `oci-spec` 0.10.0's [`Reference` documentation](https://docs.rs/oci-spec/0.10.0/oci_spec/distribution/struct.Reference.html) exposes synchronous `FromStr`/`TryFrom`, `registry`, `repository`, `tag`, `digest`, and canonical `Display`/`whole` APIs.
- Its [0.10.0 parser source](https://github.com/youki-dev/oci-spec-rs/blob/v0.10.0/src/distribution/reference.rs) normalizes Docker-familiar names to `docker.io`, adds `library/` for single-component Docker Hub repositories, converts `index.docker.io` to `docker.io`, adds `latest` to name-only references, distinguishes registries and ports, parses tags and digests, limits repository names to 255 bytes, and contains first-party good/bad reference tests.
- The [0.10.0 manifest](https://github.com/youki-dev/oci-spec-rs/blob/v0.10.0/Cargo.toml) has an optional `distribution` feature and no target-specific, async-runtime, subprocess, networking, native-library, or build-script dependency. The published [Apache-2.0 license](https://github.com/youki-dev/oci-spec-rs/blob/v0.10.0/LICENSE) is compatible with this project.
- The official [crates.io record](https://crates.io/api/v1/crates/oci-spec) reported 16,983,049 total downloads, 3,675,281 recent downloads, 39 releases, and non-yanked 0.10.0 published on 2026-05-27. The [reverse-dependency API](https://crates.io/api/v1/crates/oci-spec/reverse_dependencies?page=1&per_page=10) reported 64 dependents, including `oci-client`, `containerd-shim`, `containers-image-proxy`, and `libcontainer` in the returned pages.
- The official [GitHub repository metadata](https://api.github.com/repos/youki-dev/oci-spec-rs) reported 293 stars, 73 forks, an unarchived repository, and a latest push on 2026-07-21. These values and crates.io counts are a 2026-08-11 snapshot, not stable API guarantees.
- A clean registry-consumer probe compiled and ran under Rust 1.96.0 on Linux and cross-checked with `cargo check` for `x86_64-pc-windows-gnu` and `x86_64-apple-darwin` using only the `distribution` feature.
- `cargo metadata` found only permissively licensed normal dependencies (Apache-2.0, MIT-family, Zlib, Unlicense, and Unicode-3.0 terms). `cargo audit --no-fetch --deny warnings` found no vulnerabilities in the 38-package probe graph against RustSec advisory-db commit `d0861df1eab469d3c58d6b836ce48b5766e5f217` dated 2026-08-11.

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Docker-familiar-name normalization; registry host/port, tag, and digest parsing; validation; canonical and familiar presentation; original-string fallback on parse failure | `Reference` supplies normalized structured fields, validation, and canonical `Display`. The required integration policy below derives familiar display and closes the Unicode-tag, digest-case, tag-plus-digest, uppercase-leading-domain, and bracketed-IPv6 differences. Oracle fallback remains a caller-owned `Err => render(original)` branch. | `pass` |
| License and security | Permissive license compatible with Apache-2.0; untrusted references remain inert data | Manifest and license are Apache-2.0, all normal transitive licenses are permissive, and the current RustSec audit is clean. The selected path parses in-process and performs no I/O, network calls, process creation, FFI, or `unsafe`; untrusted input must enter only through `parse`/`TryFrom`, never unchecked `with_*` constructors. | `pass` |
| Platforms and targets | Linux, macOS, and Windows CLI targets | Rust 1.96 checks passed on Linux and for Windows GNU and Intel macOS targets. The selected source and dependency manifest contain no OS-specific implementation or native link requirement. | `pass` |
| Maintenance and Rust version | Active release history and Rust 1.96 compatibility | 0.10.0 was published 2026-05-27; repository pushed 2026-07-21. Clean Rust 1.96 consumer builds passed on all required target families. | `pass` |
| Architectural constraints | Parsing-only, synchronous, no shell subprocess, no ambient async runtime, modest build scope | Use exact 0.10.0 with defaults disabled and only `distribution`; its natural `Reference` value API is synchronous. No runtime, networking client, or system service is involved. | `pass` |

## Candidate comparison

Evidence counts below are official crates.io snapshots from 2026-08-11.

| Candidate | Behavior and fit | Maintenance and adoption | Build/license | Decision |
| --- | --- | --- | --- | --- |
| [`oci-spec` 0.10.0](https://crates.io/crates/oci-spec/0.10.0) | Direct Docker default-domain, official-repository, legacy-domain, and default-tag normalization; structured registry/repository/tag/digest fields; validation and canonical `Display`. Requires the small, explicit oracle-compatibility presentation/validation policy below. | 16,983,049 total / 3,675,281 recent downloads; 64 reverse dependencies; 39 releases; released 2026-05-27; repository active in 2026. | Apache-2.0; synchronous pure Rust; distribution-only consumer configuration. | **Selected:** most popular, idiomatic passing foundation by a wide margin. |
| [`container_image_dist_ref` 0.3.0](https://crates.io/crates/container_image_dist_ref/0.3.0) | Its [source](https://github.com/SKalt/container_image_dist_ref/blob/v0.3.0/src/lib.rs) closely tracks `distribution/reference`, is zero-copy/no-std, and handles bracketed IPv6, but preserves raw borrowed components and provides neither Docker Hub/default-tag normalization nor canonical/familiar `Display`. Its generic digest model also needs caller-side supported-algorithm validation. | 11,943 total / 731 recent downloads; 0 reverse dependencies; last release 2024-03-10. | Apache-2.0; zero normal dependencies. | Rejected: strong grammar parser, but substantially more normalization, validation, and display policy would remain local, with far lower adoption. |
| [`docker-image-reference` 0.1.0](https://crates.io/crates/docker-image-reference/0.1.0) | Its [source](https://github.com/mzohreva/docker-image-reference/blob/main/src/lib.rs) parses and redisplays the original grammar but does not normalize familiar names, split registry from repository, add `latest`, or validate supported digest algorithms/lengths. | 6,480 total / 253 recent downloads; 0 reverse dependencies; one release from 2021-10-02. | Apache-2.0; synchronous. | Rejected: missing required normalization and supported-digest validation, and stale/minimally adopted. |
| [`oci-client` 0.17.0](https://crates.io/crates/oci-client/0.17.0) | Reuses an `oci-spec` reference type, but is an OCI registry client rather than a parsing primitive. Its [manifest](https://github.com/oras-project/rust-oci-client/blob/v0.17.0/Cargo.toml) brings HTTP, TLS, JWT, futures, and Tokio-oriented functionality. | 5,931,951 total / 2,341,812 recent downloads; 69 reverse dependencies; released 2026-05-19. | Apache-2.0, but high unrelated build and runtime surface. | Rejected: fails the parsing-only/no ambient runtime architectural gate. |
| [`docker-image` 0.2.1](https://crates.io/crates/docker-image/0.2.1) | Its [source](https://github.com/sunsided/docker-image-rs/blob/v0.2.1/src/lib.rs) parses common names/tags/digests, but does not apply Docker familiar-name normalization and accepts a narrower registry grammar. | 39,578 total / 7,379 recent downloads; 0 reverse dependencies; released 2025-02-22. | [EUPL-1.2](https://github.com/sunsided/docker-image-rs/blob/v0.2.1/LICENSE.md); Rust 1.81 MSRV. | Rejected at license gate: EUPL-1.2 is outside the requested permissive Apache-2-compatible set. |
| [`use-oci-reference` 0.0.1](https://crates.io/crates/use-oci-reference/0.0.1) | Its [source](https://github.com/RustUse/use-oci/blob/v0.0.1/crates/use-oci-reference/src/lib.rs) provides typed OCI components but preserves absent Docker defaults, trims surrounding input, and does not implement Docker Hub `library/`/`latest` normalization. | First 0.0.1 release in 2026; no established adoption evidence. | MIT OR Apache-2.0; Rust 1.95. | Rejected: Docker-oracle semantics are missing and the crate is too new to outrank the established passing choice. |

Exact crates named `docker-reference` and `distribution-reference` were also checked in the crates.io registry and do not exist; they are misnomers rather than credible candidates.

## Selected integration

Use exactly:

```toml
oci-spec = { version = "=0.10.0", default-features = false, features = ["distribution"] }
```

The natural model is `input.parse::<oci_spec::distribution::Reference>()`. Keep the original input alongside the parse attempt because parse failure is intentionally non-fatal presentation behavior:

1. If the input starts with a bracketed IPv6 registry and optional numeric port followed by `/`, use a narrow compatibility pre-parse: validate the Go grammar's bracketed host/port prefix, substitute a neutral valid hostname only for the `Reference` parse, and retain the original bracketed registry for output. A malformed prefix remains an error. This is required because the selected parser's registry regex does not accept the oracle's bracketed-IPv6 form.
2. On `Err`, render the original input with the requested style, byte-for-byte, matching `FormatImage`.
3. On `Ok`, derive the familiar name from `registry()` and `repository()`: omit `docker.io`; additionally omit one leading `library/` only when the remainder is a single path component. Preserve every other registry (including ports) and repository path.
4. If `digest()` is present, display `familiar-name@digest` and ignore any parsed tag. This reproduces Go `ParseDockerRef`, which drops the tag from a tag-plus-digest input.
5. Otherwise display `familiar-name:tag`, styling the colon faintly and the name/tag with the requested style. The dependency supplies `latest` for a name-only input.
6. Before accepting the dependency parse as oracle-valid, require an ASCII-only Docker tag and lowercase hexadecimal digest text of the algorithm-specific length. This closes Rust-regex `\w` accepting Unicode tags and the dependency accepting uppercase digest hex, both of which the Go oracle rejects.
7. For an uppercase-leading first component before slash (for example `Foo/bar`), preserve that component as the registry in any canonical representation. The oracle treats it as a registry, while `oci-spec` normalizes it under Docker Hub; the familiar TUI string happens to be the same after Docker Hub elision.

### Known limitations

- `oci-spec` 0.10.0 does not parse bracketed IPv6 registry hosts such as `[fc00::1]:5000/repo`, which `distribution/reference` accepts. The required narrow compatibility pre-parse above must cover this case so the TUI does not incorrectly take the raw fallback and lose tag-separator styling.
- Rust `regex` treats `\w` as Unicode by default, while Go's reference tag grammar is ASCII. Apply the ASCII tag check above.
- Supported digests are limited to SHA-256, SHA-384, and SHA-512. This matches the algorithms registered by the frozen Go `go-digest` dependency in this program, but the Rust parser's uppercase-hex acceptance must be tightened as described above.
- A parsed tag-plus-digest reference retains both fields; Go `ParseDockerRef` returns the digest-only form. Presentation must prefer the digest and omit the tag.
- The public `Reference::with_*` constructors do not validate their strings. Do not use them for untrusted CLI or daemon-provided references.
- The crate does not declare `rust-version` in its manifest. Rust 1.96 compatibility is established by the probe, so any future upgrade needs a fresh toolchain check.
- The exact distribution-only feature set works as a normal crates.io dependency because Cargo caps dependency lints. Building the unpacked crate as the primary package with only `distribution` currently triggers its own `#![deny(warnings)]` on feature-disabled dead code; this does not affect the verified registry-consumer configuration but is an upstream feature-hygiene limitation.

### Verification command or minimal probe

The following minimal consumer was verified with Rust 1.96.0. Create a temporary crate with the selected manifest line and this `main.rs`:

```rust
use oci_spec::distribution::Reference;

fn main() {
    let digest = "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    let cases = [
        ("busybox", "docker.io", "library/busybox", Some("latest"), None),
        ("index.docker.io/ubuntu:24.04", "docker.io", "library/ubuntu", Some("24.04"), None),
        ("registry.example:5000/ns/app:Tag", "registry.example:5000", "ns/app", Some("Tag"), None),
        ("busybox@sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", "docker.io", "library/busybox", None, Some(digest)),
    ];
    for (input, registry, repository, tag, digest) in cases {
        let parsed: Reference = input.parse().unwrap();
        assert_eq!(parsed.registry(), registry);
        assert_eq!(parsed.repository(), repository);
        assert_eq!(parsed.tag(), tag);
        assert_eq!(parsed.digest(), digest);
    }
    assert!("INVALID/Repo".parse::<Reference>().is_err());
}
```

Run:

```sh
cargo +1.96.0 run --locked
cargo +1.96.0 check --locked --target x86_64-pc-windows-gnu
cargo +1.96.0 check --locked --target x86_64-apple-darwin
```

Package acceptance must additionally port oracle characterization cases for familiar display, invalid-input raw fallback, Unicode-tag rejection, lowercase digest validation, tag-plus-digest tag removal, uppercase-leading registry ambiguity, and bracketed IPv6 behavior.

## Review

This parsing-only dependency does **not** require a second critical-dependency researcher. It is a synchronous, side-effect-free data parser used for CLI presentation, not container control, networking, storage, cryptography, unsafe FFI, a runtime, or a production service. The hard-gate analysis found parity details that require ordinary package parity tests, but no critical dependency risk.

Affected package packet: `upstream/uncloud/internal/cli/tui` (a packet file was not present at research time). Its `FormatImage` output is consumed by `upstream/uncloud/pkg/client/deploy`, `cmd/uc/caddy`, `cmd/uc/ps`, `cmd/uc/machine`, `cmd/uc/service`, and `cmd/uc/image` display paths.
