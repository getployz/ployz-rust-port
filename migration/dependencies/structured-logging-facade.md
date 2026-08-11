# Dependency decision: `structured-logging-facade`

| Field | Value |
| --- | --- |
| Status | `approved` |
| Selected dependency | `tracing = { version = "=0.1.44", default-features = false, features = ["std"] }`; `tracing-subscriber = { version = "=0.3.23", default-features = false, features = ["fmt", "registry", "std"] }` |
| License | `MIT` for both direct dependencies |
| Research date | `2026-08-11` UTC |
| Request | Delegated capability request; no on-disk request exists at base `8f3d09845ff41fb7a1406d83beb293e2193f9134` |

## Required behavior and oracle evidence

The frozen oracle's `upstream/uncloud/internal/log/handler.go:11-16` defines the
custom output as `LEVEL MESSAGE key1=value1 key2=value2`. Its handler:

- accepts DEBUG, INFO, WARN, and ERROR records and defaults to INFO
  (`handler.go:63-70`);
- right-pads the level to five characters, writes `LEVEL MESSAGE ` first, then
  delegates structured fields and the newline to Go's text handler
  (`handler.go:73-81`);
- removes the built-in time, level, and message attributes from that delegated
  structured suffix (`handler.go:27-34`);
- preserves inherited attributes and named nested groups across derived
  handlers (`handler.go:47-60`); and
- synchronizes a direct handler call and returns an error from either the
  prefix write or the delegated structured write (`handler.go:72-81`).

The error requirement is specifically a **direct handler** contract. Go's
global `slog.Debug`/`Info`/`Warn`/`Error` path deliberately discards the error
returned by `Handler.Handle`, as shown by the official
[`slog.Logger.log` source](https://go.dev/src/log/slog/logger.go). Rust global
event macros may therefore remain fire-and-forget, but the port's low-level
formatter/handler entry point must return the original `io::Error` when it is
called directly. If the request instead means that an ordinary global logging
macro must return an I/O error to its caller, no candidate below passes and the
decision must return to human escalation; that is not behavior the oracle has.

Initialization has two observable paths:

- `upstream/uncloud/internal/log/env.go:10-18` lowercases, but does not trim,
  `DEBUG`; exactly `1`, `true`, or `yes` replaces the current default with the
  custom stderr handler at DEBUG, while every other value leaves the current
  default untouched. It then attempts the debug record `logger initialized`.
- `upstream/uncloud/cmd/uncloudd/main.go:19-23` unconditionally replaces the
  process default with the custom stderr handler at DEBUG.

Representative structured inheritance is not hypothetical. The metrics server
derives `component=metrics` in
`upstream/uncloud/internal/machine/metrics/server.go:29`; the DNS server derives
`component=dns-server` at `internal/machine/dns/server.go:97` and adds
`name`/`type` at line 188; `internal/machine/machine.go:1391-1401` successively
adds `stream_id` and `unit` or `container`. The oracle currently has no
production `WithGroup` call, but group preservation is part of the handler's
supported contract and must be covered by Rust characterization tests.

## Primary-source evidence

- [`tracing` events](https://docs.rs/tracing/0.1.44/tracing/macro.event.html)
  carry a level, message, and typed key-value fields. The DEBUG, INFO, WARN, and
  ERROR convenience macros use the same event model. Spans provide contextual
  parent/child structure and inherited fields rather than requiring a Go-shaped
  logger-cloning API; this matches the existing component then request-field
  call sites.
- [`FmtContext`](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/fmt/struct.FmtContext.html)
  exposes the event's span scope and root-to-leaf span visitation. The
  [`FormatEvent`](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/fmt/format/trait.FormatEvent.html)
  and `FormatFields` extension points permit an oracle-specific level/message,
  inherited-field, event-field, group, ordering, quoting, and newline format.
- [`MakeWriter`](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/fmt/trait.MakeWriter.html)
  produces `io::Write` values and has an implementation for `std::sync::Mutex<W>`.
  It supports a single shared synchronized sink and injected deterministic test
  writers without another crate.
- [`LevelFilter`](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/filter/struct.LevelFilter.html)
  directly supplies OFF, ERROR, WARN, INFO, DEBUG, and TRACE filtering. The
  oracle only needs the four record levels and INFO/DEBUG thresholds, so the
  heavier string-parsing `env-filter` feature is unnecessary.
- The [`reload` layer and handle](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/reload/index.html)
  replace a layer or filter behind a synchronized reload handle and rebuild
  call-site interest. This supplies repeatable internal handler/filter
  replacement after one process-owned global subscriber has been installed.
- The tracing
  [`set_global_default`](https://docs.rs/tracing/0.1.44/tracing/subscriber/fn.set_global_default.html)
  operation itself is intentionally one-shot. Scoped
  [`with_default`](https://docs.rs/tracing/0.1.44/tracing/subscriber/fn.with_default.html)
  restores the prior thread-local subscriber and is the deterministic unit-test
  mechanism. It does not propagate automatically to threads spawned inside the
  scope, so concurrent tests must install the dispatch explicitly in each
  spawned thread or test the low-level handler directly.
- The stock subscriber formatting layer is not sufficient for the direct-error
  contract. Its
  [`on_event` source](https://docs.rs/tracing-subscriber/0.3.23/src/tracing_subscriber/fmt/fmt_layer.rs.html#1017-1065)
  buffers a record, calls `write_all`, and only reports/ignores the failure
  because `Layer::on_event` returns `()`. The selected stack still passes because
  a custom fallible handler can return `io::Result` when invoked directly and
  its tracing layer can deliberately discard that result, exactly as the Go
  global logger does.
- The exact [`tracing` manifest](https://docs.rs/crate/tracing/0.1.44/source/Cargo.toml.orig)
  and [`tracing-subscriber` manifest](https://docs.rs/crate/tracing-subscriber/0.3.23/source/Cargo.toml.orig)
  declare MIT, Rust 1.65, and the selected features. Disabling defaults excludes
  `#[instrument]` proc macros, the `log` compatibility bridge, ANSI output,
  JSON/Serde, regex/env parsing, time crates, and optional formatting extras.
- Official crates.io records captured on 2026-08-11 reported
  [`tracing`](https://crates.io/api/v1/crates/tracing) 0.1.44 at 762,283,121
  total / 170,996,024 recent downloads and 39,405 reverse-dependent crates, and
  [`tracing-subscriber`](https://crates.io/api/v1/crates/tracing-subscriber)
  0.3.23 at 543,006,233 total / 131,527,108 recent downloads and 17,523
  reverse-dependent crates. The releases were published 2025-12-18 and
  2026-03-13 respectively. Cargo is a concrete established adopter; its
  official [debugging guide](https://doc.crates.io/contrib/implementation/debugging.html)
  documents Cargo's use of tracing.

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Four levels; structured fields; inherited attributes and nested context; INFO/DEBUG filtering; synchronized process-global dispatch; env and daemon replacement; exact custom format; direct handler write errors | Events, spans, registry context, custom formatting/writer traits, `LevelFilter`, scoped/global dispatch, and `reload` cover the model. The required custom fallible boundary and global-error discard are specified below. | `pass` |
| License and security | Permissive licenses; acceptable safety/security posture | Both direct crates are MIT; the exact resolved graph is MIT and/or Apache-2.0. `cargo audit` found no vulnerability in the probe lock. `tracing` 0.1.44 is newer than the fix in [`RUSTSEC-2023-0078`](https://rustsec.org/advisories/RUSTSEC-2023-0078.html), and subscriber 0.3.23 is newer than the fix in [`RUSTSEC-2025-0055`](https://rustsec.org/advisories/RUSTSEC-2025-0055.html). Selected APIs are safe; the mature crates contain reviewed internal unsafe code for dispatch/call-site machinery. | `pass` |
| Platforms and targets | Rust 1.96; Linux plus portable CLI/build targets | Both manifests declare MSRV 1.65. The exact feature set compiled with Rust 1.96.0 for `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-gnu`, and `x86_64-apple-darwin`; selected APIs have no OS service, FFI, or async runtime. | `pass` |
| Maintenance and Rust version | Maintained, mature, and compatible with Rust 1.96 | Both exact releases are current, non-yanked, maintained in the Tokio tracing repository, and have adoption several orders of magnitude above niche structured alternatives. | `pass` |
| Architectural constraints | Idiomatic Rust API; deterministic tests; replaceable internal configuration despite one-shot process global; custom format without an imitation of `slog` | Native events and spans shape call sites. The executable owns one global subscriber, while a reloadable same-type layer/filter supplies internal replacement. Scoped dispatch and injected writers isolate ordinary tests. | `pass` |

## Candidate comparison

Adoption values are official crates.io values captured on 2026-08-11 and are
comparison evidence, not future guarantees.

| Candidate | Hard-gate fit | Adoption / maintenance | Weight and ergonomics | Decision |
| --- | --- | --- | --- | --- |
| [`tracing` 0.1.44](https://crates.io/api/v1/crates/tracing) + [`tracing-subscriber` 0.3.23](https://crates.io/api/v1/crates/tracing-subscriber) | Passes through native structured events/spans, filters, registry context, reload, scoped dispatch, and a custom fallible handler. Global macros return no error, matching Go's global calls; direct error propagation stays in the handler. | 762.3M / 543.0M total downloads; 39,405 / 17,523 reverse dependents; current 2025-12 and 2026-03 releases; used by Cargo. | Two direct crates. The selected graph has nine third-party packages total and no async runtime, regex, Serde, ANSI, time, or proc macro. Best native model for inherited context and custom collection. | **Selected: most idiomatic and widely adopted passing stack.** |
| [`log` 0.4.33](https://crates.io/api/v1/crates/log) + [`env_logger` 0.11.11](https://crates.io/api/v1/crates/env_logger) | `log`'s optional [`kv`](https://docs.rs/log/0.4.33/log/kv/index.html) captures record fields, but it has no contextual span/child-logger model for inherited attributes and groups. [`set_logger`](https://docs.rs/log/0.4.33/log/fn.set_logger.html) is one-shot and [`Log::log`](https://docs.rs/log/0.4.33/log/trait.Log.html) returns `()`. `env_logger` custom formatting still discards print failures in its [logger source](https://docs.rs/env_logger/0.11.11/source/src/logger.rs). | `log`: 1.120B total downloads / 29,804 reverse dependents; `env_logger`: 523.2M / 11,723. Both active and extremely popular. | A small conventional logging stack, but adding local contextual state/groups and a replaceable facade would duplicate the missing dependency responsibility. | **Rejected on behavior and architecture hard gates despite greater raw downloads.** |
| [`slog` 2.8.2](https://crates.io/api/v1/crates/slog) + `slog-scope` 4.4.1 | [`Logger`](https://docs.rs/slog/2.8.2/slog/struct.Logger.html) strongly supports inherited key-values and [`Drain::log`](https://docs.rs/slog/2.8.2/slog/trait.Drain.html) is fallible. A usable root logger must nevertheless fuse or ignore drain errors, global macros still return `()`, groups/exact format remain custom, and process global behavior needs `slog-scope`. Its [global scope source](https://docs.rs/slog-scope/4.4.1/src/slog_scope/lib.rs.html#106-180) uses replaceable ArcSwap state but has guard/lifetime semantics absent from the oracle. | `slog`: 72.8M total / 413 reverse dependents; `slog-scope`: 25.8M / 58. Current releases exist, but slog's own [project documentation](https://docs.rs/crate/slog/2.8.2) directs new users toward tracing as the larger active ecosystem. | At least two direct crates before a terminal formatter; default nested values add Serde machinery. Direct drain ergonomics are good, contextual async/thread instrumentation and current adoption are materially weaker. | **Passing only with more glue; rejected in favor of the substantially more idiomatic/popular passing stack.** |
| [`fern` 0.7.1](https://crates.io/api/v1/crates/fern), `flexi_logger` 0.31.10, and [`log4rs` 1.4.0](https://crates.io/api/v1/crates/log4rs) | All are implementations around the `log` record/global model. They improve formatting, filtering, sinks, and in flexi_logger/log4rs reload, but cannot supply native inherited span attributes or nested groups, and global record calls cannot propagate I/O failures. | fern: 43.8M total / 465 reverse dependents; log4rs: 22.1M / 397; flexi_logger had an active 2026-08 release. | fern is light but behavior-incomplete. flexi_logger and log4rs bring rotation/configuration/color/regex/file machinery unrelated to the oracle. | **Rejected on behavior; heavier choices also fail dependency-weight/architecture gates.** |
| [`logforth` 0.30.1](https://crates.io/api/v1/crates/logforth) | Active extensible append/layout design, but its primary bridges use `log` or other tracing systems; it has no more direct match for inherited oracle groups and process-owned tracing spans than the selected stack. | 0.93M total downloads / 24 reverse dependents; current 2026-06 release; Rust 1.89; Apache-2.0. | Modular, but composing bridge, diagnostic, filter, layout, and appender crates adds surface without improving parity. | **Rejected: far lower adoption and no behavior or weight advantage.** |

## Selected integration

### Required dependencies and features

Use exactly:

```toml
tracing = { version = "=0.1.44", default-features = false, features = ["std"] }
tracing-subscriber = { version = "=0.3.23", default-features = false, features = ["fmt", "registry", "std"] }
```

The integrator, not the package implementor or dependency researcher, owns the
workspace manifest and lockfile change. Do not enable `tracing`'s `attributes`
or `log` features, or subscriber's `ansi`, `env-filter`, `json`, `serde`,
`tracing-log`, `time`, or default feature set, unless a later dependency request
approves a new capability. The `fmt` feature has no additional normal
dependencies beyond `registry` and `std` in this exact release; it exposes the
custom formatting context/traits.

The resolved normal graph in the verification probe was:

```text
tracing 0.1.44
├── pin-project-lite 0.2.17
└── tracing-core 0.1.36
    └── once_cell 1.21.4
tracing-subscriber 0.3.23
├── sharded-slab 0.1.7
│   └── lazy_static 1.5.0
├── thread_local 1.1.10
│   └── cfg-if 1.0.4
└── tracing-core 0.1.36
```

### Natural API and mandatory parity constraints

- Port call sites to `tracing::{debug, info, warn, error}` events with native
  event fields. Use entered spans for inherited component/request context and
  nested spans for nested groups. Do not expose an API whose principal purpose
  is to reproduce `slog.Logger`, `With`, or `WithGroup` method shapes.
- Preserve root-to-leaf inherited-field order followed by event-field order.
  Flatten named nested groups with the same dotted keys and reproduce Go
  `TextHandler` value quoting/escaping. Ignore tracing target, module, file,
  span name, and timestamps unless they represent an oracle attribute.
- Preserve the exact custom record shape, including five-column level padding,
  the space after the message even when there are no fields, and one final
  newline. DEBUG is disabled at INFO and enabled at DEBUG; INFO/WARN/ERROR are
  enabled at both thresholds.
- Own one shared `Mutex`-protected writer/handler across every derived context.
  Do not allocate an independent output mutex per span or derived logger.
- The low-level handler must have a directly callable fallible path returning
  `io::Result`. To preserve the oracle's direct write boundary, write the
  level/message prefix first and the structured suffix/newline second; return
  immediately on the first failure and return the second failure otherwise.
  A stock `fmt::Layer` alone is not acceptable because it buffers into one
  `write_all` and swallows the resulting error. A custom `Layer::on_event` may
  call the fallible handler and discard its result, matching Go's global logger.
- The executable boundary owns the one tracing global installation. Wrap the
  application layer and/or level filter in the official reload facility, retain
  its handle, and use that internal handle for same-type replacement. A false
  `DEBUG` value must leave an already configured internal default untouched; a
  true value installs/reloads the DEBUG stderr configuration, while daemon
  setup installs/reloads it unconditionally.
- Do not claim that tracing can replace an unrelated subscriber installed by
  another library. `set_global_default` cannot do so. Treat an unexpected prior
  global subscriber as an initialization error at the owning executable
  boundary rather than silently losing logs. This is compatible with the
  current binaries, which own initialization before application work.
- Use `with_default` plus injected writers for ordinary unit tests. Serialize
  the few tests that exercise the real process global, or run them in separate
  test processes. A thread-local scoped subscriber does not follow newly
  spawned threads; propagate a `Dispatch` explicitly in concurrency tests.
- Port direct tests for nil/default options, INFO filtering, DEBUG enabling,
  all four level labels, no-field trailing space, field ordering and quoting,
  inherited attributes, nested/empty groups, concurrent derived contexts, and
  first- and second-write failures. Port env tests for exact accepted values,
  case folding, no trimming, false-value no-op, and daemon replacement.

## Known limitations and risks

- Tracing's process global is install-once, unlike Go's freely replaceable
  `slog.SetDefault`. The approved adaptation requires application ownership of
  the initial subscriber plus a retained reload handle. It cannot take over an
  arbitrary third-party global subscriber later.
- Tracing field names are static call-site metadata. Current oracle call sites
  use static keys. Any later behavior requiring arbitrary runtime field/group
  keys needs explicit characterization; do not silently coerce it into a static
  tracing field or enable a serialization stack without returning to the
  dependency gate.
- The built-in tracing text formats do not match the oracle's exact level
  padding, suffix-only field rendering, Go quoting, group flattening, or
  two-write error boundary. The custom formatter/handler is required production
  code, not cosmetic configuration.
- Global tracing event APIs cannot report sink failures, but this matches Go's
  global logger. Only the directly invoked low-level handler is approved to
  promise error propagation.
- A scoped test dispatch is thread-local. Incorrect tests can appear green
  while spawned-thread events go to another/default subscriber.
- The Go wrapper creates a fresh outer mutex in `WithAttrs`/`WithGroup` while
  the embedded Go text handlers share their own output lock. Cross-derived
  concurrent prefix/suffix interleaving is therefore a potential oracle flaw
  without an upstream test. The package parity review must characterize it
  before deciding whether the Rust shared-mutex behavior is an allowed repair
  or observable behavior to preserve.
- The disabled `tracing-log` bridge means records emitted only through the
  legacy `log` crate are not captured. No current port capability requests
  third-party dependency logs; enabling that bridge requires a new decision.
- Audit the eventual workspace lock normally. A clean probe audit is evidence
  for this exact resolution on the research date, not a permanent guarantee
  about future advisories or lockfile changes.

## Verification command or minimal probe

A temporary binary using only the two exact manifest entries compiled and ran a
native event/span/reload/scoped-dispatch API probe with Rust 1.96.0. The exact
feature graph also compiled for Linux, Windows GNU, and macOS targets:

```sh
cargo +1.96.0 run --locked
cargo +1.96.0 check --locked --target x86_64-unknown-linux-gnu
cargo +1.96.0 check --locked --target x86_64-pc-windows-gnu
cargo +1.96.0 check --locked --target x86_64-apple-darwin
cargo tree --locked -e normal
cargo audit --file Cargo.lock
```

All checks passed. `cargo audit` loaded 1,211 RustSec advisories and reported no
vulnerability for the ten-package lockfile (the application plus nine
third-party packages).

Package acceptance must additionally run oracle-format and failing-writer tests
because a dependency-only compile probe cannot establish Ployz's custom
formatter parity.

## Review

A second fresh adversarial **dependency** reviewer is not required by
`migration/dependencies/README.md`: this is not networking, storage,
cryptography, a runtime, container control, unsafe FFI, or a production service.
The ordinary fresh parity and Rust reviews remain mandatory for the package.
They must focus on global install/reload races, false-DEBUG no-op behavior,
daemon replacement, spawned-thread test dispatch, static-key limitations,
group/order/quoting parity, shared writer synchronization, and both direct write
failure points.

Affected package packet: future `crates/ployz-internal-log` for
`upstream/uncloud/internal/log`; no package packet exists at the research base.
Its initialization callers are `cmd/uc` and `cmd/uncloudd`, and its structured
event consumers span the daemon/internal packages cited above.
