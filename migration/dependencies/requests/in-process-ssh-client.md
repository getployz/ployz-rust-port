# Dependency request: `in-process-ssh-client`

| Field | Value |
| --- | --- |
| Status | `research-required` |
| Capability | In-process SSH protocol client with agent and private-key authentication, reusable sessions, signals, and tunneled stream dialing |
| Waiting packages | `internal/sshexec`; future direct consumer `pkg/client/connector` |

## Required behavior

- Connect over TCP using port 22 when omitted and a five-second connection
  timeout.
- When the SSH user is omitted, consume the process-global current-user result
  from the integrated `internal/fs` crate and preserve its ignored-error/empty
  username limitation.
- Try the Unix `SSH_AUTH_SOCK` agent first, close its socket after the dial
  attempt, and fall back to an unencrypted private key only after agent setup or
  authentication fails.
- Parse the private key without exposing its contents and preserve distinct
  read, parse, agent-connect, agent-dial, and private-key-dial error context.
- Preserve the oracle's insecure host-key acceptance unless an explicit human
  decision records a security deviation.
- Reuse a connected client for multiple sessions. Sessions must support
  combined output, independently streamed stdout/stderr, remote SIGINT on
  cancellation, session cleanup, and connection close.
- Support the direct connector caller's SSH-tunneled Unix-socket dialing over
  the reusable client; a system `ssh` subprocess is not a substitute for this
  path.
- Keep shell quoting and the separately observable system-SSH CLI executor
  independent of the protocol-client selection.

Oracle evidence: `upstream/uncloud/internal/sshexec/ssh.go`, `remote.go`,
`executor.go`, `sshcli.go`, and `sshcli_test.go`. Direct caller evidence:
`upstream/uncloud/internal/cli` and
`upstream/uncloud/pkg/client/connector/ssh.go`.

## Constraints

Hard gates: Rust 1.96; Linux and macOS on amd64 and arm64; permissive license;
maintained cryptography and protocol implementation; no application-level
unsound/private access; no secret logging; bounded cancellation and cleanup;
natural reusable client/session/stream APIs rather than a Go API imitation.
This is a networking and cryptography capability, so a fresh adversarial second
researcher/reviewer is required.

Exact crates, versions, features, accepted security deviations, and integration
seams require a new explicit decision. No prior user authority covers this
capability.

## Research assignment

Compare all credible maintained Rust SSH clients and protocol stacks using
primary sources and executable probes. Apply behavior, security, platform,
maintenance, Rust-version, license, cancellation, and tunnel-stream hard gates
before considering adoption. Among passing candidates, select the popular,
idiomatic solution with the smallest safe integration surface. Do not select a
subprocess wrapper or build a package-local SSH protocol.

## Completion

Record the exact dependency/version/features, transport and authentication
model, host-key policy, limitations, probes, and clean adversarial review in
`migration/dependencies/in-process-ssh-client.md`.
