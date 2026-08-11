# Dependency decision: `randomized-exponential-backoff`

| Field | Value |
| --- | --- |
| Status | `provisional` — selected stack passed bounded gates; mandatory fresh adversarial review is pending |
| Capability | Configurable cancellable randomized exponential retry scheduling |
| Selected dependencies | `backon = { version = "=1.6.0", default-features = false, features = ["std", "tokio-sleep"] }`; `fastrand = { version = "=2.5.0", default-features = false, features = ["std"] }` |
| License | Backon: `Apache-2.0`; fastrand: `Apache-2.0 OR MIT` |
| Research date | `2026-08-11` UTC |
| Request | [`requests/randomized-exponential-backoff.md`](requests/randomized-exponential-backoff.md) |
| Integration base | `7634959e3f4962f3c5298bab3b1f3c7d5670be57` |

## Decision

Select Backon 1.6.0 for idiomatic sync/async retry orchestration and its public
`BackoffBuilder` extension point. Use fastrand 2.5.0 in a small policy that
generates the frozen scheduling distribution. Tokio remains the separately
approved async runtime; do not enable Backon's default feature set.

Backon's built-in `ExponentialBuilder` is deliberately **not** approved for
this contract: its optional jitter adds `[0, current)` to the current delay,
producing `[1x, 2x)`, while the oracle uses the inclusive nanosecond formula
around the interval and defaults to `[0.5x, 1.5x]`. A package-local policy that
implements Backon's natural iterator/builder traits is smaller and more honest
than wrapping the dependency behind a Go-shaped API. It must preserve the
formula, interval cap, elapsed cutoff, zero-as-unbounded setting, and fresh
builder/reset behavior listed below. Seeded/injected fastrand is a test seam;
production uses an unseeded policy instance.

## Oracle and reachability

The external API symbol search covered all of `upstream/uncloud` while
excluding `upstream/uncloud/experiment/**`. Production uses are:

- `internal/docker`: unbounded 100 ms initial / 1 s cap, permanent non-connect
  failures, and context cancellation treated as successful shutdown;
- `internal/corrosion`: 2 s transport, 60 s resubscribe, value-returning retry,
  permanent errors, injected policies in tests, and reuse/reset;
- `internal/machine/cluster`: unbounded 100 ms initial / 5 s cap supervisor;
- `internal/machine/docker`: unbounded 1 s initial / 30 s cap and reset after a
  successful reconnect before the watched stream can fail again;
- `internal/machine/corroservice`: 50 ms initial / 1 s cap / 15 s elapsed;
- `internal/cli`, `internal/ucind`, and `pkg/client`: finite readiness waits,
  value-returning HTTP retry, permanent errors, and cancellation.

The frozen module pins `github.com/cenkalti/backoff/v4 v4.3.0`. Its defaults are
500 ms initial, `0.5` randomization, `1.5` multiplier, 60 s maximum interval,
and 15 minutes maximum elapsed. `MaxElapsedTime == 0` is unbounded; equality at
`elapsed + next == max` is allowed; retry runs the operation at least once;
permanent errors retain the operation value and stop; cancellation interrupts
the wait and returns the cancellation error.

## Primary-source evidence

- Backon's published [1.6.0 source](https://docs.rs/crate/backon/1.6.0/source/)
  exposes `BackoffBuilder`, value-returning async retry, error predicates,
  configurable sleepers, notifications, and adjustment. Its retry state polls
  the operation, then the sleep, without spawning a task. Dropping the retry
  future therefore drops the active operation/sleep in the caller.
- Its [published manifest](https://docs.rs/crate/backon/1.6.0/source/Cargo.toml)
  declares Apache-2.0, Rust 1.85, and independent `std` and `tokio-sleep`
  features. The exact selected feature set avoids blocking, browser, Embassy,
  and futures-timer sleepers.
- Backon's official [crates.io record](https://crates.io/api/v1/crates/backon)
  reported 69,419,708 total downloads and 220 reverse dependencies. The
  [repository](https://github.com/Xuanwo/backon) is unarchived, had 1,047 stars,
  and was pushed in June 2026.
- fastrand's published [2.5.0 source](https://docs.rs/crate/fastrand/2.5.0/source/)
  exposes an owned `Rng`, deterministic `with_seed`, and `f64()` in `[0, 1)`.
  Its [manifest](https://docs.rs/crate/fastrand/2.5.0/source/Cargo.toml) declares
  Rust 1.63 and `Apache-2.0 OR MIT`.
- fastrand's official [crates.io record](https://crates.io/api/v1/crates/fastrand)
  reported 812,575,322 total downloads, 186,876,647 recent downloads, and 1,617
  reverse dependencies; 2.5.0 was released 2026-07-19.
- The frozen Go implementation and tests are the primary behavioral source:
  `upstream/uncloud`'s module-cache copy of
  `github.com/cenkalti/backoff/v4@v4.3.0/{exponential,retry,context}.go`.

## Hard gates

| Gate | Evidence | Result |
| --- | --- | --- |
| Required behavior | Backon supplies retry/value/error-predicate/sleep orchestration and a natural custom-policy trait. The bounded Go 1.26.1/Rust 1.96 probe matched injected boundary formula cases, 100/150/225/337/506/759/1000 ms clamping, reset, the signed-64-bit cap path, elapsed equality/stop, unbounded mode, value return, permanent stop, and cancellation before/during wait. | `pass` with required policy below |
| Cancellation and tasks | The selected retry is one caller-owned future. Race it against the package cancellation future using `tokio::select!`; dropping it drops the sleep/operation and creates no detached task. Poll the retry branch first when cancellation is already ready so the oracle's guaranteed first operation still runs. | `pass` |
| License/security | Selected direct licenses are permissive. The exact nine-package probe lock had zero RustSec vulnerabilities and zero warnings. No package-owned `unsafe`, FFI, native library, crypto, network, or build script is required. | `pass` |
| Rust/platforms | Backon MSRV 1.85 and fastrand MSRV 1.63 are below Rust 1.96. Locked checks passed for Linux amd64/arm64 and macOS amd64/arm64. | `pass` |
| Maintenance/adoption | Both repositories are unarchived and active in 2026. Backon leads maintained retry candidates in adoption; fastrand is an established current RNG. | `pass` |

## Candidate comparison

| Candidate | Result |
| --- | --- |
| **Backon 1.6.0 + fastrand 2.5.0** | Selected. Most adopted actively maintained retry candidate; the public builder trait supports a small exact policy. Built-in additive jitter is rejected. |
| `backoff` 0.4.0 | Very popular historically (96.6M downloads), but last release/source commit was 2021. Its copied formula also rounds zero-randomization durations through `f64`, differing from Go near `i64::MAX`. Fails maintenance and exact overflow gates. |
| `maybe-backoff` 0.5.0 | Recent fork with nearly identical API/formula, but zero reverse dependencies and about 20k downloads. The probe exposed the same zero-randomization precision mismatch (`i64::MAX-7` became `2^63`). Its async retry also constructs the next operation future before sleeping. Rejected for behavior and adoption. |
| `tokio-retry` 0.3.2 | Active and popular historically, but its policy is integer-millisecond base-power growth with full jitter, no elapsed/reset policy, and no `1.5` multiplier. |
| `retry` 2.2.0 | Active sync retry crate; millisecond exponential iterator and full-jitter helpers do not preserve the distribution, elapsed limit, or async cancellation contract. |
| `exponential-backoff` 2.1.0 | Maintained policy generator, but uses integer growth factor, attempt count rather than elapsed cutoff, and different jitter semantics; no retry/cancellation orchestration. |

## Required integration policy

Implement the minimum Backon `BackoffBuilder`/`Iterator<Item = Duration>` policy
inside each assigned migration crate that needs this capability; do not expose
the Go dependency's types or names.

1. Store positive intervals in nanoseconds, with defaults `500 ms`, `0.5`,
   `1.5`, `60 s`, and `Some(15 min)`. Map oracle zero maximum elapsed to
   `None`.
2. If the randomization factor is zero, return the current interval without a
   floating-point round trip. Otherwise use the oracle formula
   `min + random * (max - min + 1 ns)` and validate injected random values are
   in `[0,1]`; production fastrand values are in `[0,1)`.
3. Cap the *next base interval*, not the randomized result. Before multiplying,
   compare `current >= max / multiplier`; set it directly to `max` on that
   branch to preserve the overflow behavior.
4. Stop only when finite `elapsed + randomized > max_elapsed`; equality is
   allowed. Use a monotonic clock and checked/saturating arithmetic that is
   characterized at the direct callers' positive duration domain.
5. Build a fresh policy for ordinary retries. Where the frozen long-lived
   watcher resets while its operation is running, keep explicit policy state in
   that crate's supervision loop and reset it after successful reconnect; do
   not assume Backon exposes an owned policy from inside the operation.
6. Use `.when(...)` for permanent errors and let Backon return the operation's
   value. Put attempt side effects inside the async operation body.
7. For cancellation, race the retry future with the package cancellation
   future and drop the loser in the same scope. Never spawn the retry solely to
   make it cancellable. A pre-cancelled call still performs one operation.

## Verification

Bounded probes were run outside the repository with exact Go 1.26.1 and Rust
1.96.0. The selected Rust lock pinned Backon 1.6.0, fastrand 2.5.0, and Tokio
1.53.1. Commands included:

```sh
/opt/go1.26.1/bin/go run /tmp/ployz-backoff-go-probe-20260811
cargo run --locked --manifest-path /tmp/ployz-backon-probe-20260811/Cargo.toml
cargo fmt --check --manifest-path /tmp/ployz-backon-probe-20260811/Cargo.toml
cargo clippy --locked --all-targets --manifest-path /tmp/ployz-backon-probe-20260811/Cargo.toml -- -D warnings
cargo check --locked --target x86_64-unknown-linux-gnu --manifest-path /tmp/ployz-backon-probe-20260811/Cargo.toml
cargo check --locked --target aarch64-unknown-linux-gnu --manifest-path /tmp/ployz-backon-probe-20260811/Cargo.toml
cargo check --locked --target x86_64-apple-darwin --manifest-path /tmp/ployz-backon-probe-20260811/Cargo.toml
cargo check --locked --target aarch64-apple-darwin --manifest-path /tmp/ployz-backon-probe-20260811/Cargo.toml
cargo audit --file /tmp/ployz-backon-probe-20260811/Cargo.lock
```

The fresh critical dependency review required by the request is pending. The
controller must not mark this row approved until that review accepts the exact
decision commit.

Affected packages: `internal/docker`, `internal/corrosion`,
`internal/machine/corroservice`, `internal/machine/cluster`, `internal/cli`,
`internal/machine/docker`, `internal/ucind`, and `pkg/client`.
