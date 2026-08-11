# Dependency request: randomized exponential backoff

| Field | Value |
| --- | --- |
| Status | `research-required` |
| Capability | Configurable cancellable randomized exponential retry scheduling |
| Waiting packages | `internal/docker`, `internal/corrosion`, `internal/machine/corroservice`, `internal/machine/cluster`, `internal/cli`, `internal/machine/docker`, `internal/ucind`, `pkg/client` |

## Required behavior

Preserve the frozen cenkalti/backoff v4.3.0 scheduling contract used throughout
the non-experimental oracle: configurable initial interval, 0.5 default
randomization factor, 1.5 default multiplier, configurable maximum interval,
zero-as-unbounded or finite maximum elapsed time, reset, permanent-error stop,
value-returning retry, and immediate context cancellation. The random interval
must follow the oracle's inclusive range/formula and duration overflow/elapsed
boundary behavior; retries must not busy-loop or sleep after cancellation.

Build exact Go 1.26.1/Rust 1.96 differential probes for deterministic seeded or
injected randomness, boundary random values, reset/reuse, max-interval clamping,
elapsed-time cutoffs, unbounded retry, permanent errors, returned values, and
cancellation before/during a wait. Trace every direct use found repo-wide while
excluding `upstream/uncloud/experiment/**` entirely.

## Constraints

- Rust 1.96; shipped Linux/macOS amd64/arm64 targets.
- Tokio-compatible async cancellation without detached tasks; synchronous
  scheduling primitives may be selected separately only if they preserve the
  same contract without blocking async executors.
- Permissive license, active maintenance, acceptable RustSec result, exact
  version/features/lock, and no package-owned unsafe code.
- Prefer the popular idiomatic passing Rust solution after hard gates; a small
  package-owned policy is acceptable only if no maintained dependency passes and
  it is demonstrably simpler than an adapter that changes behavior.

## Research assignment

Compare all credible maintained Rust retry/backoff libraries using primary
source, exact manifests, adoption data, and executable differential probes.
Account for the complete repo-wide use matrix rather than only Docker's 100 ms,
1 s cap, unbounded case.

## Completion

Create `migration/dependencies/randomized-exponential-backoff.md` and obtain a
fresh adversarial dependency review before approval because scheduling and
cancellation affect multiple production network/control loops.
