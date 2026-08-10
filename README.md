# Ployz Rust port

This repository is a pure Rust port of Uncloud. The frozen upstream Go oracle
is checked in at [`upstream/uncloud`](upstream/uncloud) and must remain
byte-for-byte identical to the pinned upstream tree. The only intentional
product-level difference in the future Rust implementation is the
Uncloud-to-Ployz rename.

- [`UPSTREAM_ORACLE.md`](UPSTREAM_ORACLE.md) records the immutable source pin,
  reproducible build/test environment, and observed baseline behavior.
- [`INHERITED_UPSTREAM_BUGS.md`](INHERITED_UPSTREAM_BUGS.md) records confirmed
  flaws that the Rust port intentionally preserves for behavioral parity.
- [`UPSTREAM_INVENTORY.md`](UPSTREAM_INVENTORY.md) accounts for the material in
  the upstream tree and distinguishes authored, generated, and third-party
  files.

No Rust implementation is present in this baseline.
