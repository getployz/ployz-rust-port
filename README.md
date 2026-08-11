# Ployz Rust port

This repository is an in-progress pure Rust port of Uncloud. The frozen upstream
Go oracle is checked in at [`upstream/uncloud`](upstream/uncloud) and must remain
byte-for-byte identical to the pinned upstream tree. The product rename from
Uncloud to Ployz applies only to the Rust port.

- [`UPSTREAM_ORACLE.md`](UPSTREAM_ORACLE.md) records the immutable source pin,
  reproducible build/test environment, and observed baseline behavior.
- [`UPSTREAM_INVENTORY.md`](UPSTREAM_INVENTORY.md) accounts for the material in
  the upstream tree and distinguishes authored, generated, and third-party
  files.
- [`CONTEXT.md`](CONTEXT.md) defines the port vocabulary.
- [`PORTING.md`](PORTING.md) is the canonical per-package workflow.
- [`migration/CONTROLLER.md`](migration/CONTROLLER.md) defines wave scheduling
  and agent ownership.
- [`migration/dependencies/README.md`](migration/dependencies/README.md) defines
  the mandatory dependency research gate.

Rust code under `crates/` is migration work. A crate is complete only when the
package registry marks it `integrated` under the canonical workflow.
