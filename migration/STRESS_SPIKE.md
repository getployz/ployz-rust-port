# Historical three-crate stress spike

This document records a pilot. It is not a package packet, dependency decision,
or current agent instruction. `PORTING.md` and `AGENTS.md` are authoritative.

This spike tests one Go package per temporary Rust crate with three concurrent
implementors.

| Go package | Rust crate | Owns |
| --- | --- | --- |
| `internal/secret` | `ployz-internal-secret` | `crates/ployz-internal-secret/**` |
| `internal/cli/config` | `ployz-internal-cli-config` | `crates/ployz-internal-cli-config/**` |
| `internal/machine/network` | `ployz-internal-machine-network` | `crates/ployz-internal-machine-network/**` |

The pilot tested exclusive crate ownership, concurrent implementation, and
workspace integration. Its earlier source-file-correspondence rule is
superseded by behavior/test traceability. Current ports use idiomatic Rust
dependencies and internal structure while preserving the oracle's observable
behavior and limitations.

## Frozen shared contract

`ployz-internal-secret` owns the implementation but must retain these public
capabilities for concurrent consumers:

- `Secret` is cloneable, comparable, hashable, and defaults to empty.
- `Secret::from_hex_string`, `Secret::to_hex_string`, `Secret::as_bytes`,
  `Secret::is_empty`, and `Secret::equal`.
- `Secret` has text-like `Display`, `FromStr`, Serde serialization, and Serde
  deserialization using lowercase hexadecimal.
- `new(length)`, `new_id()`, and `random_alphanumeric(length)` return
  cryptographically random values or `SecretError`.

The frozen contract above is historical evidence from the pilot. It does not
grant `accepted` or `integrated` status under the current workflow.
