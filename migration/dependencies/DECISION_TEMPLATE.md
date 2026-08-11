# Dependency decision: `<capability>`

| Field | Value |
| --- | --- |
| Status | `approved`, `blocked`, or `human-decision-required` |
| Selected dependency | `<crate/system/API and exact version>` |
| License | `<SPDX expression>` |
| Research date | `<UTC date>` |
| Request | `<request path>` |

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | | | `pass` or `fail` |
| License and security | | | |
| Platforms and targets | | | |
| Maintenance and Rust version | | | |
| Architectural constraints | | | |

## Candidate comparison

Compare primary-source evidence for idiomatic ecosystem fit, crates.io downloads
and dependents, established-project usage, maintenance, documentation,
integration complexity, and build cost. Record every credible rejected candidate
and the concrete reason it lost.

## Selected integration

Record required features, configuration, natural API model, known limitations,
and the verification command or minimal probe. The Rust implementation will
follow this dependency's design rather than reproduce the Go dependency API.

## Review

Record the fresh adversarial dependency reviewer for critical capabilities, its
findings, fixes, and final result. List every affected package packet.

