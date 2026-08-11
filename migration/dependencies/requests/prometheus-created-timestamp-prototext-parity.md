# Dependency request: `prometheus-created-timestamp-prototext-parity`

| Field | Value |
| --- | --- |
| Status | `research-required` |
| Capability | Prometheus counter creation timestamps and exact legacy protobuf text formatting |
| Waiting packages | `internal/machine/metrics` |

## Required behavior

Match frozen Go client_golang v1.22 metric-family output for delimited protobuf,
legacy protobuf text, and compact text. Counter DTOs require observable
`created_timestamp`; text forms must reproduce Go prototext syntax and spacing.
Prometheus text 0.0.4 routing, gzip, bind/serve/drain, and shutdown behavior are
already implemented behind the package seam.

## Constraints

The solution must support Rust 1.96, the existing metrics registry behavior and
supported targets, allowed licenses, active maintenance, deterministic encoding,
and exact counter creation instants without unsound access. `prometheus` 0.14.0
cannot represent the Go DTO timestamp and its `MetricFamily` display syntax is
not byte-compatible.

## Research assignment

Compare current Prometheus DTO/runtime options, protobuf adapters, and a narrow
formatting/metadata seam. Identify whether exact parity is feasible without a
new feature or whether explicit user acceptance of a scoped output deviation is
required. A fresh adversarial internal review is required.

## Completion

Record exact crates/versions/features or a precise human decision, primary
evidence, golden probes, accepted/rejected deviations, and clean review in
`migration/dependencies/prometheus-created-timestamp-prototext-parity.md`.
