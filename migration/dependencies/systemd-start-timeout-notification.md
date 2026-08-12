# systemd start-timeout notification

## Decision

Approve Linux-targeted `libsystemd = 0.7.2` with default features. Call
`libsystemd::daemon::notify(false, &[NotifyState::Other(
"EXTEND_TIMEOUT_USEC=30000000".into())])`; `false` preserves `NOTIFY_SOCKET`.
Keep the dependency under `cfg(target_os = "linux")` and provide a no-op adapter
on other targets.

The package owns only the Tokio scheduling policy: notify every 10 seconds while
the pull is pending, stop after the first notification failure, stop promptly on
pull completion/cancellation, and cap extension at five minutes. The task must be
joined rather than detached.

## Evidence

- The official [`daemon::notify` source](https://github.com/lucab/libsystemd-rs/blob/v0.7.2/src/daemon.rs)
  translates a leading `@` in `NOTIFY_SOCKET` to Linux's abstract namespace,
  accepts `unset_env = false`, returns `Ok(false)` when the variable is absent,
  and permits exact `KEY=VALUE` messages through `NotifyState::Other`.
- The official [systemd notification protocol](https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html)
  defines `EXTEND_TIMEOUT_USEC=` and requires the first extension before the
  original timeout expires.
- The published manifest declares Rust 1.69, MIT/Apache-2.0, and a pure-Rust
  implementation. Ployz pins Rust 1.96.

## Rejected alternatives and limits

- `sd-notify 0.5.0` uses `UnixDatagram::connect(path)` directly and does not
  translate systemd's leading `@` abstract-socket notation.
- `libsystemd` does not compile as a general macOS/Windows dependency. Targeting
  it only on Linux preserves all shipped-target checks; non-Linux adapters are
  intentionally no-op because systemd notification is Linux runtime behavior.
- No FD passing, journal, login, or daemonization API is authorized.

