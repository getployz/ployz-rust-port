# Ployz port agent entry point

The frozen Go tree at `upstream/uncloud/` is the behavioral oracle. Keep it
byte-for-byte unchanged.

For every port task, read `CONTEXT.md` and `PORTING.md` before acting. Then use
the branch that matches your assigned role:

- **Controller or packetizer:** read `migration/CONTROLLER.md`. Own migration
  registries, wave plans, and package packets. Do not implement crates.
- **Dependency researcher:** read `migration/dependencies/README.md` and the
  assigned dependency request. Own only its decision record. Do not implement
  the package that requested it.
- **Package implementor:** read the assigned package packet. Own only the crate
  directory named by that packet. Use only approved dependencies. A missing
  dependency returns the package to the dependency gate.
- **Parity or Rust reviewer:** use `migration/REVIEW_TEMPLATE.md`. Review
  read-only; return findings to the original implementor.
- **Integrator:** merge accepted package commits and own shared workspace files
  such as root `Cargo.toml` and `Cargo.lock`.

One task performs one role. Use an isolated branch or worktree for every
implementor. A package is complete only in the `integrated` state defined by
`PORTING.md`.

