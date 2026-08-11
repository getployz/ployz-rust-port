# Dependency gate

Every new external capability requires a fresh research agent before a package
implementor can use a dependency. Reuse an approved decision for the same
capability; do not repeat research per package.

## Trigger

Create a dependency request when a packet or implementation needs a Rust crate,
system library, executable, service, generated binding, or direct protocol
implementation that has no approved row in `migration/DEPENDENCIES.tsv`.

The implementor stops at the gate. The controller dispatches the research agent.

## Dependency request

Create `migration/dependencies/requests/<capability>.md` from
`migration/dependencies/REQUEST_TEMPLATE.md` with:

- the domain capability, stated without a preferred crate;
- required observable behavior and limitations from packet rows;
- supported platforms and targets;
- sync, async, cancellation, performance, and safety constraints;
- allowed licenses and Rust/toolchain constraints;
- source and caller evidence;
- the packages waiting on the decision.

## Research process

The fresh research agent uses primary sources: official documentation, crate
source, manifests, license files, release history, issue trackers, and public
adoption data. It identifies all credible candidates and runs a minimal API
probe only when primary sources cannot distinguish them.

Apply hard gates first:

- required observable behavior;
- acceptable license and security posture;
- required platforms and targets;
- active maintenance and compatible Rust version;
- architectural constraints that cannot be adapted safely.

Among candidates that pass, choose the most idiomatic and widely adopted Rust
solution. Use crates.io downloads and dependents, established-project usage,
release activity, documentation, and ecosystem conventions as evidence. Then
prefer simpler integration and lower build cost when candidates remain close.

The dependency's natural API shapes the Rust implementation. Reject an adapter
whose main purpose is to imitate the Go dependency API.

Networking, storage, cryptography, runtimes, container control, unsafe FFI, and
production services require a second fresh adversarial dependency reviewer.

## Decision record

Write `migration/dependencies/<capability>.md` from
`migration/dependencies/DECISION_TEMPLATE.md` with:

```text
Status: approved | blocked | human-decision-required
Capability:
Selected dependency and exact version:
Primary-source evidence:
Hard-gate results:
Candidate comparison:
Rejected candidates and reasons:
Required features and configuration:
License and security notes:
Known limitations:
Verification command or probe:
Affected package packets:
Reviewer result when required:
```

The controller validates the record, updates `migration/DEPENDENCIES.tsv`, and
adds the decision to waiting packets. Only the integrator or dependency steward
edits root workspace dependency versions and the lockfile.

