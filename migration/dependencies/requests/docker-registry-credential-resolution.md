# Dependency request: Docker registry credential resolution

| Field | Value |
| --- | --- |
| Status | `research-required` |
| Capability | Resolve registry authentication from the local Docker CLI configuration |
| Waiting packages | `internal/docker`, `internal/machine/docker`, `pkg/client` |

## Required behavior

Preserve the frozen Docker CLI credential behavior for image references: load
the default config path and `DOCKER_CONFIG` override; normalize Docker Hub and
registry names exactly; select per-registry inline auth, global `credsStore`, or
per-registry `credHelpers`; invoke helpers with the Docker credential-helper
protocol; preserve helper/config stderr and error behavior; and emit Docker's
base64 URL-encoded JSON auth token. An auth object containing no username,
password, auth field, identity token, or registry token becomes empty. Pull
silently ignores lookup failure; push falls back to encoded empty auth; direct
callers may propagate lookup errors.

Build exact Go 1.26.1/Rust 1.96 differential fixtures for missing/malformed
config, inline credentials, Docker Hub aliases, ports, helper precedence,
helper success/not-found/malformed/failure, identity/registry tokens, empty
objects, Unicode/path edge cases, and exact encoded bytes/error boundaries.
Search the API symbols and callers repo-wide while excluding
`upstream/uncloud/experiment/**` entirely.

## Constraints

- Rust 1.96; shipped Linux/macOS amd64/arm64 targets.
- No shell parsing; helper subprocess I/O must be bounded, cancellable where the
  caller supplies cancellation, and must not leak credentials to logs/errors.
- Permissive license, active maintenance, acceptable RustSec result, exact
  version/features/lock, and no package-owned unsafe code.
- Prefer the popular idiomatic passing Rust solution after hard gates. Return to
  the gate before implementing a private helper protocol if a maintained crate
  does not cover it.

## Research assignment

Compare all credible maintained Rust Docker config/credential-helper libraries
and natural lower-level components with primary source, exact manifests,
adoption data, and executable Go/Rust probes. Treat any crate named by a caller
as a candidate, not a decision.

## Completion

Create `migration/dependencies/docker-registry-credential-resolution.md`. Obtain
a fresh adversarial dependency review before approval because credentials and
subprocess boundaries are security-sensitive.
