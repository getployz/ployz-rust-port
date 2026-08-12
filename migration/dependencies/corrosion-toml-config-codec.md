# Corrosion TOML config codec

## Decision

Approve `toml = 1.1.4+spec-1.1.0` with `default-features = false` and features
`["display", "parse", "serde", "std"]`, alongside the already-approved
`serde = 1.0.229` with `derive,std`.

The package should model the complete Corrosion configuration as ordered Rust
structs, use `skip_serializing_if` for zero `max_mtu`, serialize with
`toml::to_string`, and parse readiness files with `toml::from_str`. Struct field
order makes emitted output deterministic without the `preserve_order` feature.

## Evidence

- The official [`toml` 1.1.4 manifest](https://github.com/toml-rs/toml/blob/toml-v1.1.4/crates/toml/Cargo.toml)
  exposes the exact `display`, `parse`, `serde`, and `std` features and declares
  Rust 1.85 through the workspace; Ployz pins Rust 1.96.
- The official [`toml` API](https://docs.rs/toml/1.1.4+spec-1.1.0/toml/)
  provides Serde-backed `to_string` and `from_str`, including TOML escaping,
  arrays, nested tables, paths, IP strings, and token strings.
- The project is the Rust TOML reference implementation, MIT OR Apache-2.0,
  pure Rust, with no runtime service, FFI, build script, or application `unsafe`.

## Rejected alternatives and limits

- `toml_edit` is unnecessary because no caller requires comment or source-layout
  preservation.
- Hand-built TOML text risks incorrect quoting and nested-table behavior.
- Serialization stability is the declared Rust struct order, not byte parity with
  Go's encoder. Semantic TOML parity is required and malformed readiness files
  remain errors.

