# Dependency decision: `in-process-ssh-client`

| Field | Value |
| --- | --- |
| Status | `human-decision-required` |
| Selected dependency | `russh = 0.62.6`; existing runtime dependency `tokio = 1.53.1` |
| License | `Apache-2.0`; `MIT` |
| Research date | `2026-08-12` UTC |
| Request | [`migration/dependencies/requests/in-process-ssh-client.md`](requests/in-process-ssh-client.md) |

`russh` is the strongest popular, idiomatic in-process candidate and the only
established maintained candidate found that exposes every required protocol
primitive through public async Rust APIs. It is the conditional technical
selection, but it is **not approved**: no authority accepts the oracle's
insecure host-key policy, the RSA feature has an unresolved transitive RustSec
advisory, and the selected secure algorithms reject the oracle's legacy DSA
private-key behavior.

## Oracle and caller contract

The frozen oracle establishes these dependency-level requirements:

- [`ssh.go`](../../upstream/uncloud/internal/sshexec/ssh.go) defaults port 22,
  uses the current OS user while ignoring lookup failure, applies a five-second
  connection timeout, tries `SSH_AUTH_SOCK` first, closes the agent connection,
  then redials with an unencrypted private key. It deliberately uses
  `ssh.InsecureIgnoreHostKey()` and distinguishes agent-connect, agent-dial,
  key-read, key-parse, and private-key-dial errors.
- [`remote.go`](../../upstream/uncloud/internal/sshexec/remote.go) reuses one
  client for multiple sessions; collects stdout and stderr together in receive
  order or streams them independently; sends remote `SIGINT` on cancellation;
  and closes each session and the client.
- [`connector/ssh.go`](../../upstream/uncloud/pkg/client/connector/ssh.go) reuses
  that client and dials a Unix-domain socket on the remote host through SSH.
- [`internal/cli/cli.go`](../../upstream/uncloud/internal/cli/cli.go) may reuse
  the provisioning connection or deliberately create a fresh connection after
  group membership changes. The shell quoting and system-SSH executor remain
  separate capabilities.

Repository-wide symbol/caller search covered `internal/sshexec`, direct CLI
callers, and `pkg/client/connector`; `upstream/uncloud/experiment/**` was
excluded as directed.

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Agent and unencrypted file-key authentication; reusable command sessions; stdout/stderr events; remote SIGINT; direct Unix-socket tunnel; explicit cleanup | `russh::client::Handle` opens reusable [session and direct-streamlocal channels](https://docs.rs/russh/0.62.6/russh/client/struct.Handle.html); [`Channel`](https://docs.rs/russh/0.62.6/russh/struct.Channel.html) exposes `exec`, `signal`, `wait`, `close`, and `into_stream`; the exact encoder sends `direct-streamlocal@openssh.com` ([source](https://docs.rs/russh/0.62.6/src/russh/client/session.rs.html#90-99)); [`AgentClient`](https://docs.rs/russh/0.62.6/russh/keys/agent/client/struct.AgentClient.html) and [`load_secret_key`](https://docs.rs/russh/0.62.6/russh/keys/fn.load_secret_key.html) cover authentication. A local OpenSSH integration probe passed Ed25519 and RSA file keys, agent auth, two channel kinds on a reusable handle, real remote SIGINT, and Unix-socket echo. The selected features intentionally omit DSA. | `human-decision-required` for DSA key parity |
| License and security | Permissive license, maintained cryptography, no secret logging or application private/unsafe access, explicit host verification | The published manifest and repository declare [Apache-2.0](https://github.com/Eugeny/russh/blob/v0.62.6/LICENSE.txt). Default preferences exclude SHA-1 KEX/MAC and CBC; the application need not enable `des`, `dsa`, or compression. `0.62.6` is newer than the `>=0.60.3` fix for the agent-frame allocation vulnerabilities [RUSTSEC-2026-0153](https://rustsec.org/advisories/RUSTSEC-2026-0153.html) and [RUSTSEC-2026-0154](https://rustsec.org/advisories/RUSTSEC-2026-0154.html). However, `rsa` resolves to `rsa 0.10.0-rc.18`, still covered by [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html), and accepting every server key enables active MITM. | `human-decision-required` |
| Platforms and targets | Linux and macOS, amd64 and arm64 | The Rust-only public integration probe checked on `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`. `ring 0.17.14` contains build targets for Apple x86_64/aarch64 and Linux x86_64/aarch64 ([build source](https://docs.rs/crate/ring/0.17.14/source/build.rs)); `russh` uses Tokio Unix sockets for the agent. Linux-hosted Apple cross-checks reached `ring` but could not run because the VM lacks the macOS SDK/compiler, so native checks remain mandatory. | `pass`, native macOS CI required |
| Maintenance and Rust version | Maintained and compatible with Rust 1.96 | `russh 0.62.6` declares Rust 1.85, was released 2026-08-11, and includes current security fixes ([release](https://github.com/Eugeny/russh/releases/tag/v0.62.6)). The Rust 1.96 probe builds and passes Clippy. | `pass` |
| Architectural constraints | Tokio-native, bounded cancellation and cleanup, public APIs, no subprocess, no Go-shaped dependency adapter | `Handle` multiplexes channels; `ChannelMsg::Data` and `ExtendedData` preserve stream identity; `Channel::signal(Sig::INT)` supports cancellation; `ChannelStream` implements Tokio I/O and closes on drop; `Handle::disconnect` provides connection shutdown. A canceled direct-streamlocal open needs connection-level retirement because dropping the pending open future alone can strand a registered channel. All used APIs are public. | `pass`, only with the supervised-open policy and leak test below |

## Required human decisions

No deviation is accepted by this record.

1. **Host-key policy.** Exact oracle parity is a handler whose
   `check_server_key` always returns `Ok(true)`. This defeats SSH server
   authentication and permits an active machine-in-the-middle attack; SSH's
   security architecture describes server authentication as the protection
   against this attack ([RFC 4251 section 9.3.4](https://www.rfc-editor.org/rfc/rfc4251#section-9.3.4)).
   A human must explicitly choose either insecure oracle parity or authorize a
   behavior deviation using `russh::keys::known_hosts` or a pinned host key.
2. **On-disk RSA private keys.** The oracle's generic private-key parser accepts
   RSA keys. Matching that behavior requires `russh` feature `rsa`, which pulls
   `rsa 0.10.0-rc.18`; `cargo audit --deny warnings` reports
   RUSTSEC-2023-0071 with no fixed version. The advisory describes RSA private
   key recovery through timing leakage. Russh uses the key for SSH signing, not
   RSA decryption/padding; reduced reachability is an inference, not clearance.
   A human must explicitly accept this residual risk, or authorize the
   `ring`-only deviation that rejects on-disk RSA keys. RSA keys held by an SSH
   agent continue to work in the `ring`-only configuration because signing is
   delegated to the agent.
3. **On-disk DSA private keys.** Go's `ssh.ParsePrivateKey` accepts unencrypted
   DSA keys, while this selection deliberately omits russh's `dsa` feature and
   the legacy `ssh-dss`/SHA-1 algorithm. Rejecting those keys is a recommended
   security deviation, but no authority in the request accepts it. A human must
   authorize that deviation or separately approve and probe `dsa`; even enabling
   the feature does not by itself prove parity with every legacy PEM encoding.

The status remains `human-decision-required` until all three choices are
recorded.

## Candidate comparison

Adoption figures are bounded snapshots from the primary
[crates.io API](https://crates.io/api/v1/crates/russh) on 2026-08-12; they are
used only after hard gates.

| Candidate | Adoption and maintenance | Gate result |
| --- | --- | --- |
| [`russh 0.62.6`](https://github.com/Eugeny/russh/tree/v0.62.6) | About 5.65M total/2.41M recent crates.io downloads, 203 reverse dependencies, 1,821 GitHub stars, and a release on 2026-08-11. Tokio-native, pure Rust SSH implementation with the complete public primitive set. | Conditional winner; human security decisions above remain. |
| [`russh-extra 0.1.7`](https://docs.rs/russh-extra/0.1.7/russh_extra/) | High-level `russh` layer with agent, shell, signal, timeout, and streamlocal facilities, but only 433 downloads and no reverse dependencies at research time. It adds policy and integration surface, including stricter key-file permissions. | Technically capable, but loses to direct `russh` on adoption, maturity, and smallest safe surface. |
| [`ssh2 0.9.6`](https://docs.rs/ssh2/0.9.6/ssh2/) | About 7.33M downloads and 176 reverse dependencies; maintained Rust bindings to libssh2. It has reusable sessions, agent/file auth, and `channel_direct_streamlocal`. | Hard fail: its public [`Channel`](https://docs.rs/ssh2/0.9.6/ssh2/struct.Channel.html) can read a remote exit signal but cannot send the required remote signal; synchronous C/shared-session operation also complicates bounded async cancellation. |
| [`async-ssh2-lite 0.5.0`](https://docs.rs/async-ssh2-lite/0.5.0/async_ssh2_lite/) | Async wrapper over `ssh2`/libssh2; last published 2024. | Hard fail: no direct-streamlocal wrapper and inherits the missing send-signal API. |
| [`async-ssh2-tokio 0.13.0`](https://docs.rs/async-ssh2-tokio/0.13.0/async_ssh2_tokio/struct.Client.html) | Maintained higher-level `russh` wrapper with agent/file auth and command streaming. | Hard fail: exposes only direct TCP channels; the underlying handle is private, so callers cannot open direct-streamlocal or send SIGINT. |
| [`async-ssh2-russh 0.3.0`](https://docs.rs/async-ssh2-russh/0.3.0/async_ssh2_russh/) | Small high-level command wrapper over `russh 0.55`. | Hard fail: no agent, remote signal, or direct-streamlocal API, and its base predates the agent-frame security fix. |
| [`makiko 0.2.5`](https://docs.rs/makiko/0.2.5/makiko/) | Pure Tokio SSH client with reusable sessions and remote signals. | Hard fail: no SSH-agent client and no direct-streamlocal channel. |
| [`thrussh 0.44.0`](https://docs.rs/thrussh/0.44.0/thrussh/) | Maintained predecessor of `russh`, but much lower current adoption. | Hard fail: no direct-streamlocal client channel; weaker key/agent integration than its maintained fork. |
| [`ssh-rs 0.5.0`](https://docs.rs/ssh-rs/0.5.0/ssh/) | Pure Rust client, last release 2023. | Hard fail: no agent authentication, direct-streamlocal, or remote-signal API; maintenance gate also fails. |
| [`libssh-rs 0.3.8`](https://docs.rs/libssh-rs/0.3.8/libssh_rs/) | Maintained synchronous wrapper with agent, file-key, send-signal, and `Channel::open_forward_unix` support, but only two reverse dependencies. | Hard fail: its native libssh dependency is LGPL rather than permissively licensed. Its synchronous C FFI would also require a blocking/thread cancellation seam. |
| [`openssh 0.11.6`](https://github.com/openssh-rust/openssh/tree/v0.11.6) and other process wrappers | Popular and maintained wrappers around the system `ssh` program. | Hard fail: not an in-process protocol client and explicitly disallowed for the tunneled connector path. |
| `anvil-ssh 1.1.0`, `rustssh2 9.0.0`, `wezterm-ssh 0.4.0`, obsolete `ssh 0.1.4` | GPL candidate; low-adoption republish of russh; old application-internal crate; obsolete crate, respectively. | Rejected for license, provenance/adoption, maintenance, or unnecessary application-specific surface before preference ranking. |

## Conditional selected integration

Use the following exact stack only if a human (1) chooses insecure host-key
parity, (2) accepts the RSA advisory risk, and (3) authorizes the secure
deviation that rejects on-disk DSA keys:

```toml
russh = { version = "=0.62.6", default-features = false, features = ["ring", "rsa"] }
tokio = { version = "=1.53.1", default-features = false, features = ["io-util", "macros", "net", "rt", "sync", "time"] }
```

`tokio` is already approved by the async runtime decision. The exact feature
set includes `macros` for cancellation coordination and async tests. Do not enable `russh`
features `aws-lc-rs`, `flate2`, `des`, `dsa`, or its default features. The
`ring` backend avoids the larger AWS-LC build surface; no compression matches
the oracle. If the human rejects local RSA risk, omit `rsa` and record the
resulting key-format deviation before implementation. If the human instead
requires DSA parity, this selection remains unresolved pending separate
`dsa` feature, key-format, and security probing; the displayed stack does not
cover that branch.

Follow the dependency's natural model:

1. Resolve the default username through the integrated `internal/fs` behavior
   and default port 22. Preserve Go's exact timeout scope: wrap only
   `TcpStream::connect` in a five-second `tokio::time::timeout`, then pass the
   connected stream to `russh::client::connect_stream`. The oracle's
   `ssh.ClientConfig.Timeout` reaches `net.DialTimeout` only; it does not bound
   SSH key exchange or authentication. Any broader handshake/auth timeout is a
   separate desirable hardening change that requires behavior-deviation
   authority.
2. For the agent attempt, first connect with `AgentClient::connect_env`. If
   agent setup succeeds, create a fresh TCP transport, enumerate identities, use
   `authenticate_publickey_with`/`authenticate_certificate_with`, and use
   `best_supported_rsa_hash().await?.flatten()` for RSA agent identities. Drop
   the agent immediately after success or authentication failure so
   `SSH_AUTH_SOCK` closes. If agent setup fails, do not open the agent-attempt
   TCP transport; proceed directly to private-key setup. Preserve distinct
   agent-connect and agent-auth/dial error contexts.
3. If agent setup or authentication fails, close/drop that transport and make a
   fresh transport for private-key auth, preserving the oracle's second dial
   and distinct error stages. Read the already home-expanded path, parse with
   `load_secret_key(path, None)`, never log key bytes, select the server's best
   RSA hash where applicable, then call `authenticate_publickey`.
4. Own one authenticated `client::Handle` and open one channel per command or
   tunneled stream. For combined output, append `ChannelMsg::Data` and
   `ExtendedData { ext: 1 }` in receive order; for streaming, route them to the
   separate writers. Continue draining after exit status until channel close.
   The `Run` result trims leading and trailing Unicode whitespace exactly as
   the oracle does and, on a remote exit error, returns that trimmed combined
   output together with the error.
5. Race command completion against cancellation. On cancellation send
   `channel.signal(Sig::INT)`, then explicitly close/drain the channel under a
   bounded cleanup timeout. If queuing SIGINT fails, return that send error; if
   it succeeds, return an error wrapping the context cancellation. The SSH
   signal request has no acknowledgement, so successful queuing cannot prove
   remote process termination. Dropping a bare command `Channel` is not the
   primary cleanup path.
6. Implement tunneled Unix dialing with a connection-owner task supervising
   `handle.channel_open_direct_streamlocal(path)`. If the caller context wins,
   keep ownership of the pending open: close a channel that arrives during a
   short cleanup grace period. If no reply arrives, send `disconnect`, retire
   and await the whole russh connection under a bound, then drop it as the hard
   fallback; the reusable client is no longer usable. Never merely drop the
   open future, because russh registers and sends the open before waiting for
   confirmation. On success, `into_stream()` yields Tokio
   `AsyncRead + AsyncWrite` and closes the channel on drop. Add a test server
   that withholds channel-open confirmation, cancel the caller, and assert the
   connection task and descriptors terminate without a stranded channel.
7. On owner shutdown, close live channels and call
   `handle.disconnect(Disconnect::ByApplication, "", "")` under a bound; drop
   is the final fallback. Keep system-SSH CLI execution and shell quoting out of
   this protocol-client seam.

## Known limitations and unaccepted deviations

- Insecure host-key acceptance and the RSA advisory are unresolved, not
  implicitly approved limitations.
- The oracle's five-second timeout covers TCP connection establishment only;
  `russh::Config::inactivity_timeout` is not an equivalent replacement.
- Agent authentication may try several identities inside the same attempt; it
  must remain bounded and its final error must not erase the distinct
  agent-connect versus agent-auth/dial context.
- `russh-cryptovec` contains internal reviewed unsafe code and documents panic
  possibilities on allocator or `mlock`/`munlock` failure. Application code
  needs no unsafe or private access, but this remains a rare availability risk.
- Native macOS amd64 and arm64 compilation/integration checks remain required;
  Linux-to-Apple cross compilation without an Apple SDK is not evidence.

## Verification

The minimal probe lived outside the repository at `/tmp/ployz-ssh-probe` and
left no repository artifact. With Rust 1.96 it compiled the exact public APIs
and ran against local OpenSSH 9.6 plus a real `ssh-agent` and remote Unix socket.
It passed Ed25519 and RSA file authentication, agent authentication, reusable
session command execution, a trapped remote SIGINT, split stderr, direct
streamlocal echo, channel cleanup, and disconnect.

The integration run used an unprivileged loopback OpenSSH configuration at
`/tmp/ployz-ssh-probe/sshd_config`, generated Ed25519/RSA client keys and an
Ed25519 host key, loaded the Ed25519 key into a fresh `ssh-agent`, and started
`socat UNIX-LISTEN:/tmp/ployz-ssh-probe/echo.sock,fork EXEC:/bin/cat`. From the
probe directory, `/usr/sbin/sshd -f ./sshd_config -E ./sshd.log` established
the server; `cargo +1.96.0 run` performed the assertions. The recorded sshd
PID, socat process, and agent were stopped afterward. The configuration names
both generated public keys in `AuthorizedKeysFile`, disables
password/interactive authentication, and permits forwarding.

The probe's `sshd_config` was:

```text
Port 32222
ListenAddress 127.0.0.1
HostKey /tmp/ployz-ssh-probe/host_ed25519
PidFile /tmp/ployz-ssh-probe/sshd.pid
AuthorizedKeysFile /tmp/ployz-ssh-probe/id_ed25519.pub /tmp/ployz-ssh-probe/id_rsa.pub
StrictModes no
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM yes
AllowAgentForwarding no
AllowTcpForwarding yes
PermitOpen any
PermitListen any
LogLevel VERBOSE
```

Commands run or required (all probe-relative commands run after
`cd /tmp/ployz-ssh-probe`):

```sh
ssh-keygen -q -t ed25519 -N '' -f ./host_ed25519
ssh-keygen -q -t ed25519 -N '' -f ./id_ed25519
ssh-keygen -q -t rsa -b 3072 -N '' -f ./id_rsa
eval "$(ssh-agent -s)"
ssh-add ./id_ed25519
/usr/sbin/sshd -f "$PWD/sshd_config" -E "$PWD/sshd.log"
socat UNIX-LISTEN:/tmp/ployz-ssh-probe/echo.sock,fork EXEC:/bin/cat &
probe_socat_pid=$!
cargo +1.96.0 fmt --manifest-path /tmp/ployz-ssh-probe/Cargo.toml --check
cargo +1.96.0 check --manifest-path /tmp/ployz-ssh-probe/Cargo.toml --all-targets
cargo +1.96.0 clippy --manifest-path /tmp/ployz-ssh-probe/Cargo.toml --all-targets -- -D warnings
cargo +1.96.0 check --manifest-path /tmp/ployz-ssh-probe/Cargo.toml --target x86_64-unknown-linux-gnu
cargo +1.96.0 check --manifest-path /tmp/ployz-ssh-probe/Cargo.toml --target aarch64-unknown-linux-gnu
cargo +1.96.0 run
cargo audit --file /tmp/ployz-ssh-probe/Cargo.lock --deny warnings
kill "$(cat ./sshd.pid)"
kill "$probe_socat_pid"
ssh-agent -k
```

All compile, Clippy, Linux-target, and integration commands passed. The audit
with `ring,rsa` intentionally failed only on RUSTSEC-2023-0071; the otherwise
identical `ring`-only lockfile passed. Do not add an audit ignore unless the
human RSA decision explicitly authorizes it. On native macOS CI, run the same
checks for `x86_64-apple-darwin` and `aarch64-apple-darwin` and add an OpenSSH
integration test covering agent success, agent failure then file fallback,
stdout/stderr ordering, long-command cancellation, streamlocal echo, timeouts,
and descriptor cleanup. Add the adversarial withheld-confirmation tunnel-open
cancellation/no-leak case before package acceptance; the existing probe does
not assert that path.

## Review

This is a critical networking/cryptography capability. A fresh adversarial
researcher independently reviewed the full oracle/caller contract and candidate
set. Its pre-draft result agreed that `russh 0.62.6` is the conditional winner,
required `human-decision-required` for host-key policy and RSA risk, identified
the command-channel explicit-close nuance, and required truthful macOS and audit
reporting. Its formal first pass found five gaps: incorrect TCP-timeout scope,
unsurfaced DSA key incompatibility, unsafe cancellation of a pending streamlocal
open, incomplete probe reproduction instructions, and missing command-result
edge semantics. The record was corrected and the probe rerun. The next full
pass found two inconsistencies: the exact stack condition did not cover all
three human choices, and agent setup was ordered after rather than before the
agent attempt's TCP dial. Both were corrected. A final fresh full re-review on
2026-08-12 returned **CLEAN** with no actionable findings.

Affected packages: `internal/sshexec`; future direct consumer
`pkg/client/connector`.
