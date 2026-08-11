# Dependency request: `lossless-docker-engine-progress-streaming`

| Field | Value |
| --- | --- |
| Status | `research-required` |
| Capability | Lossless Docker Engine pull/push JSON progress streaming |
| Waiting packages | `internal/docker` |

## Required behavior

Preserve ordered Docker progress messages including `id`, `status`, textual
`progress`, `progressDetail`, and `errorDetail`. Direct caller
`upstream/uncloud/pkg/client/image.go` uses message IDs for per-layer rendering.
Embedded stream errors, EOF, blocked reads, cancellation, response closure, and
registry-auth behavior must remain observable in the same order as the frozen
Go oracle.

## Constraints

The solution must work with the approved asynchronous runtime and Docker Engine
transport policy on supported targets, avoid orphan tasks on cancellation, use
Rust 1.96, and have an allowed license and maintained security posture. Bollard
0.21.0 is only a candidate: its `PushImageInfo` drops `id` and textual
`progress`, while its raw response stream is private.

## Research assignment

Compare maintained Docker clients, a bounded low-level Docker HTTP/JSON stream,
and an upstream Bollard correction. Select the most popular idiomatic option
that passes behavior, transport, cancellation, platform, license, maintenance,
and Rust-version gates. A fresh adversarial internal review is required.

## Completion

Record the exact dependency/version/features, supported transports, Engine API
compatibility, cancellation policy, deviations, probes, and clean review in
`migration/dependencies/lossless-docker-engine-progress-streaming.md`.
