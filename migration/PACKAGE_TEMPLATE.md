# Package packet: `<go/package>`

## Assignment

| Field | Value |
| --- | --- |
| Go package | `<path under upstream/uncloud>` |
| Migration crate | `<crate name>` |
| Owned path | `crates/<crate>/**` |
| Base commit | `<full SHA>` |
| Wave | `<wave identifier>` |
| State | `catalogued` |

The implementor owns only the path above. The integrator owns root workspace
files. The controller owns this packet and registries.

## Oracle inventory

List every direct Go source, test, generated file, platform file, direct caller,
and internal import. State explicitly when a category is empty.

## Behavior contract

For each observable behavior, record:

| ID | Input or event | Required result | Errors, ordering, timing, or limitation | Evidence |
| --- | --- | --- | --- | --- |
| `<B01>` | | | | `<file:line or test>` |

Include external formats, persistence, concurrency, platform behavior, and
observable upstream flaws. Resolve questions through callers and tests. Mark an
unresolved question as a blocker; do not guess.

## Rust design freedom

Record constraints that are truly external. Internal modules, functions, types,
ownership, concurrency, and algorithms remain free to follow idiomatic Rust and
the approved dependencies.

## Dependency capabilities

| Capability | Decision record | Status |
| --- | --- | --- |
| | `migration/dependencies/<name>.md` | `approved` or `research-required` |

Every dependency must reference an approved decision before this packet becomes
`ready`.

## Test traceability

| Behavior ID | Go test or source evidence | Required Rust test | Result |
| --- | --- | --- | --- |
| `<B01>` | | | `pending` |

Port test cases, not Go test mechanics. Add characterization cases only where a
required behavior has no upstream test.

## Acceptance commands

List exact targeted formatting, check, test, and Clippy commands plus any
platform, privileged, fixture-comparison, or oracle command required for this
package.

## Handoff

The implementor records its commit, deliberate behavior mappings, and check
results. Reviewers record findings using `migration/REVIEW_TEMPLATE.md`. The
controller records state changes and blockers.

