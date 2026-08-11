# Controller workflow

This file is the single authoritative scheduler and integration workflow for
the port. `migration/PACKAGES.tsv`, `migration/DEPENDENCIES.tsv`, and
`migration/TASKS.tsv` are the restartable machine-readable state. Per-package
packet, review, worker, and result Markdown files are not part of this workflow.

The controller schedules packages and dependencies and serially integrates
accepted work. It does not implement package crates or review its own changes.

## Build and maintain the package graph

Catalog every in-scope Go package from the frozen oracle, including nested Go
modules, generated packages, platform-specific packages, and tests. Record one
row per package in `migration/PACKAGES.tsv`. The production import DAG is defined
by `direct_internal_dependencies`; `direct_callers` is its reverse index.
Test-only imports are recorded separately and remain acceptance inputs.

`upstream/uncloud/experiment/**` is explicitly out of scope by user direction
as of 2026-08-11. Skip it entirely: do not catalog, research, schedule, port, or
use it as acceptance evidence.

The ready queue is dynamic. A package is dependency-ready only when every
production internal dependency is integrated and every external capability it
needs has an approved decision. The worker reads the oracle package, its tests,
and callers directly; the controller does not create a prose packet.

## Human-approved provisional dependency policy

`migration/AUTHORITIES.tsv` records explicit user authority that overrides a
dependency record's earlier blocked hard-gate verdict for scheduling. As of
2026-08-11, every currently known non-terminal capability may use the strongest
popular idiomatic candidate as an approved provisional seam. Documented
behavior, platform, security, or lifecycle deviations are accepted migration
limitations for this run; exhaustive dependency parity proof is not a
prerequisite to package implementation. `migration/DEPENDENCIES.tsv` is the
controlling scheduler status, while the detailed decision record remains the
evidence for limitations the implementor must preserve or isolate.

This authority applies only to the capability rows explicitly present in
`migration/AUTHORITIES.tsv`. A capability discovered later remains unapproved
until its own exact crates, versions, features, deviations, and user-approval
date are selected and recorded. Dependency research and review for the approved
rows may improve the decision in parallel but must not consume or delay an
available implementation slot.

The terminal exceptions were resolved by explicit user authority on 2026-08-11:
all three existing terminal rows select exact iocraft 0.8.4 with only
`unstable-output-streams`. This does not approve any future terminal or other
capability.

## Scheduler pools

### Package implementation pool

Maintain a target of **8 concurrent package implementation or fixer tasks**
whenever 8 dependency-ready packages exist.

- Count a task only while it is writing or fixing package code.
- Dependency research, adversarial review, integration, waiting-to-merge, and
  supervisor work do not consume implementation slots.
- When an implementation becomes dependency-blocked, enters review, completes,
  or fails, release its implementation slot and immediately refill it from the
  dependency-ready DAG queue.
- A package returning from review with findings re-enters the implementation or
  fixer queue and consumes a slot while its code is being changed.
- Never fabricate work to reach 8. If fewer than 8 packages are dependency-ready,
  use the shortfall to clear dependency and DAG blockers.

Every package writer gets a fresh task, one exclusive crate path, and an exact
integration base commit. A worker may commit only its crate path. Shared root
Cargo files, the lockfile, registries, decisions, and the integration branch are
controller/integrator-owned.

### Native package-owner topology

The normal implementation pool consists of eight fresh native Codex worktree
threads in addition to the supervisor. Each package has exactly one native
package-owner thread, and that owner controls the package's entire lifecycle:
oracle/caller inspection, implementation, targeted gates, two fresh read-only
adversarial reviews, fixes, rerun gates, fresh clean re-reviews, the final
crate-only commit, and one structured final result.

The package owner creates and controls its own parity and Rust-quality review
contexts. Those review contexts are children of the package lifecycle; the
supervisor does not launch, monitor, or adjudicate package-level reviews as
separate workflow tasks. A package is not accepted merely because it compiles
or because the supervisor obtained an independent review.

The supervisor catalogs and schedules packages, monitors only package-owner
threads, handles structured dependency requests, verifies accepted commits,
integrates serially, runs workspace/oracle gates, pushes, archives, and refills
native lanes. New package work must never be launched in the supervisor's
collaboration subagent pool. Legacy collaboration package owners already active
on 2026-08-11 may finish their current package under the same complete
review/fix/re-review contract; replace each with a fresh native package-owner
thread when it finishes.

### Dependency pool

Cap newly launched dependency-research tasks at **4 concurrent tasks**. Let
already-running research above the cap finish; do not cancel useful work.
Deduplicate requests by capability. Critical capabilities require a fresh
adversarial second researcher before approval. Dependency research and
adversarial review never consume implementation slots.

Each dependency capability is likewise owned end-to-end by one dedicated
native dependency thread, including its bounded research and required internal
review. The supervisor records and integrates only the thread's explicit final
decision.

### Review pool

After targeted checks are green and the worker has a crate-only review commit,
the package leaves the implementation pool. It must obtain two fresh read-only
reviews: behavior parity and Rust quality. Reviews and waiting for them do not
consume implementation slots. The original worker or a fresh fixer edits any
findings, reruns checks, and obtains fresh re-reviews.

## Dynamic worker prompt

Construct each worker prompt from repository state, not a packet file. It must
contain:

- Go package path and exclusive Rust crate path;
- exact integration base commit;
- direct production dependencies, test-only dependencies, and direct callers;
- approved dependency decisions and exact versions/features;
- package and workspace acceptance commands; and
- the implement, targeted checks, two reviews, fix, and fresh re-review contract.

The worker reads the frozen source, tests, and callers itself. Missing external
capability returns a structured dependency request and moves the package to
`dependency-blocked`; it must not select an unapproved dependency.

## Package states

Every transition requires a concrete artifact:

| State | Required evidence |
| --- | --- |
| `catalogued` | package row and complete DAG edges |
| `waiting-internal` | one or more production internal dependencies not integrated |
| `dependency-blocked` | exact deduplicated external capability blocker |
| `ready` | all internal dependencies integrated and external decisions approved |
| `implementing` | exclusive writer task and exact base assigned |
| `reviewing` | targeted checks green and crate-only review commit exists |
| `fixing` | actionable review or integration findings and an active writer |
| `accepted` | two fresh clean reviews on the corrected crate commit |
| `integrating` | accepted commit being verified in the primary checkout |
| `integrated` | merge, workspace/oracle gates, integration commit, and GitHub push all succeeded |

Compilation alone is never acceptance or parity evidence.

## Serial integration and push

Root integration remains serial:

1. Verify the structured worker result, exact parent, scope, and commit.
2. Cherry-pick the crate-only commit into the primary checkout.
3. Apply controller-owned root workspace, lockfile, registry, and decision fixes.
4. Run at minimum `cargo fmt --all --check`,
   `cargo check --workspace --all-targets`,
   `cargo test --workspace --all-targets`, and
   `cargo clippy --workspace --all-targets --all-features -- -D warnings`, plus
   relevant Go oracle/differential/platform checks.
5. Commit integration-only changes and normally push the integration branch.
6. Only after the push succeeds, mark the package `integrated`, archive/recycle
   its task safely, and refill the implementation slot.

If integration fails, the package returns to `fixing`; do not archive it as
complete. Push after every successful worker merge. Never force-push, stash, or
discard unrelated/user work.

## Restart and recovery

After every scheduler transition, update the package row with state, owner or
thread, base, commit, and blocker, and update `migration/TASKS.tsv` with each
live package-owner or dependency-owner task's thread ID, host ID, cursor, role,
package, worktree/branch, state, base, and current artifact commit. Child review
contexts remain under their owner and are not supervisor task rows. Dependency
rows record the durable decision and active owner task when applicable. On
restart, rebuild readiness from the import DAG plus these rows, verify current
Git state, reconcile live owner tasks, and refill native writers before
launching optional new research.

Repeated mistake classes update global `PORTING.md` or this controller and
trigger a targeted re-audit of affected earlier crates. No package-specific
prose state is created.
