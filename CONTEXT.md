# Port vocabulary

Use these terms in packets, task prompts, reviews, and status reports.

- **Oracle:** the immutable Go source and tests under `upstream/uncloud/`.
- **Go package:** one directory of Go files with the same package declaration.
  Nested directories are separate packages.
- **Migration crate:** the temporary Rust crate owned by one package
  implementor. One Go package maps to one migration crate for parallel
  ownership; this does not prescribe the final Rust architecture.
- **Behavior contract:** the observable inputs, outputs, state changes, errors,
  ordering, timing, formats, platform behavior, flaws, and limitations that the
  Rust port must preserve.
- **Package packet:** the complete assignment for one migration crate. It names
  ownership, the behavior contract, traceability, approved dependencies, and
  acceptance commands.
- **Traceability:** a mapping from each required Go behavior to source evidence
  and a Rust test. Traceability does not require matching files or functions.
- **Dependency request:** a capability request created when no approved Rust
  dependency covers a need.
- **Dependency decision:** primary-source research that selects the popular,
  idiomatic Rust solution after hard gates pass.
- **Dependency gate:** the pause between a dependency request and an approved
  dependency decision.
- **Wave:** packages that can be implemented concurrently after their contracts
  and dependency decisions are ready.
- **Controller:** the agent that creates packets, schedules waves, assigns
  ownership, and advances package state.
- **Implementor:** the agent that designs and implements one migration crate.
- **Parity reviewer:** a fresh read-only agent that checks the Rust behavior
  against the oracle and package packet.
- **Rust reviewer:** a fresh read-only agent that checks idiomatic Rust design,
  approved dependency use, safety, and maintainability.
- **Integrator:** the agent that merges accepted crates and verifies the whole
  wave.
- **Accepted:** both reviewers are clean and the crate acceptance commands pass.
- **Integrated:** the accepted crate is merged and the workspace and wave tests
  pass.

