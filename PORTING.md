# Package porting workflow

This is the canonical workflow for the Uncloud-to-Ployz Rust port. Historical
pilot instructions do not override it.

## Goal

Preserve the oracle's observable features and limitations. Use popular,
idiomatic Rust dependencies and let their APIs shape the internal Rust design.
The Go package defines an ownership boundary, not a Rust implementation shape.

Keep these stable:

- behavior used by callers;
- external formats and protocols;
- error conditions callers depend on;
- ordering, timing, and state transitions;
- observable platform behavior, flaws, and limitations.

Rust may change modules, file layout, functions, types, ownership, concurrency,
and internal algorithms. Record traceability in tests and the package packet,
not through Go-shaped compatibility helpers.

## Package state machine

Only the controller changes package state in `migration/PACKAGES.tsv`.

```text
catalogued
    -> contract-ready
    -> dependency-blocked -> contract-ready
    -> ready
    -> implementing
    -> reviewing
    -> fixing -> reviewing
    -> accepted
    -> integrated

any non-terminal state -> blocked -> prior state
```

`blocked` is an exceptional state for missing authority, ambiguous observable
behavior, unavailable verification infrastructure, or a dependency decision
with no passing candidate. Record the exact blocker in the package packet.

## 1. Prepare the package packet

The controller follows `migration/CONTROLLER.md` and creates a packet from
`migration/PACKAGE_TEMPLATE.md` under `migration/packages/`.

Completion criterion: every Go file in the package, direct caller, exported
symbol, upstream test, platform file, observable limitation, and external
capability is accounted for. The packet has no unresolved behavior question.

## 2. Pass the dependency gate

For every external capability, the controller checks
`migration/DEPENDENCIES.tsv`.

- An approved decision is added to the package packet.
- A missing decision creates a dependency request and dispatches a fresh
  research agent using `migration/dependencies/README.md`.
- Equivalent requests share one decision. Package agents do not repeat research.

The researcher first applies hard gates for required behavior, license,
platforms, security, maintenance, and Rust/toolchain compatibility. Among the
passing candidates, the default choice is the most idiomatic and widely adopted
Rust solution. The selected dependency shapes the Rust design; it is not hidden
behind an imitation of the Go dependency API.

Completion criterion: every non-standard dependency and external tool has an
approved decision referenced by the packet. No dependency question remains with
the implementor.

## 3. Implement one migration crate

The controller gives the implementor one ready packet and an isolated branch or
worktree. The implementor changes only the packet's owned crate directory.

The implementor:

1. Designs idiomatic Rust modules around the approved dependencies and behavior
   contract.
2. Ports upstream test cases and adds characterization tests only for required
   behavior the upstream tests do not cover.
3. Implements until the crate passes formatting, targeted tests, targeted
   all-target checks, and warnings-denied Clippy.
4. Completes every traceability row and performs a source-to-behavior
   self-review.
5. Commits only owned crate files and returns the commit plus deliberate
   behavior mappings to the controller.

If implementation exposes a new dependency capability, the implementor stops
and returns a dependency request. It does not select a crate or build a local
substitute.

Completion criterion: no production placeholder remains; every traceability row
points to a passing Rust test; targeted acceptance commands pass.

## 4. Review and fix

The controller dispatches two fresh agents in parallel with
`migration/REVIEW_TEMPLATE.md`:

- the parity reviewer compares the packet, oracle, callers, tests, and Rust
  behavior;
- the Rust reviewer checks idiomatic design, dependency usage, safety, async and
  platform behavior, errors, and maintainability.

Reviewers do not edit. The original implementor fixes all findings and reruns
targeted acceptance. Reviewers recheck their findings. Repeat until both reports
are clean. A reviewer cannot require matching Go structure; it can require
missing observable behavior.

Completion criterion: both fresh reviewers report no actionable findings and
targeted acceptance remains green. The controller marks the package `accepted`.

## 5. Integrate the wave

Only the integrator edits shared workspace manifests and lockfiles. It merges
accepted crate commits in dependency order and runs:

```sh
cargo fmt --all --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Run the relevant Go oracle tests in the environment documented by
`UPSTREAM_ORACLE.md`. When both implementations accept the same fixtures,
compare their results directly.

An integration failure returns the affected package to `fixing`; its original
implementor owns the repair and the relevant reviewers recheck it.

Completion criterion: all wave crates pass workspace acceptance, relevant Go
oracle tests pass with only recorded upstream behavior, and every package in the
wave is `integrated`.

## Human escalation

Ask for a decision only when no dependency passes hard gates, top dependency
candidates remain materially tied, observable Go behavior is unresolved after
checking callers and tests, parity conflicts with security or licensing, or
required infrastructure cannot be supplied. Record the decision in the packet
or dependency record before work resumes.
