# Dependency findings: `internal/cli/logs` time and options

| Field | Decision |
| --- | --- |
| Status | **Approve both capabilities** |
| Local-time timestamp formatting | `jiff = "=0.2.35"`, `default-features = false`, features `tz-system,tzdb-zoneinfo` |
| CLI option definition/parsing | `clap = "=4.6.6"`, `default-features = false`, features `derive,std,help,usage,error-context` |
| Licenses / MSRV | Jiff: `Unlicense OR MIT`, Rust 1.70; clap: `MIT OR Apache-2.0`, Rust 1.85 |
| Research date | 2026-08-12 UTC |
| Affected package | `internal/cli/logs` |

## Local-time timestamp formatting

Approve Jiff 0.2.35. Its [`Timestamp` accepts a fallible conversion from
`SystemTime`](https://docs.rs/jiff/0.2.35/jiff/struct.Timestamp.html#impl-TryFrom%3CSystemTime%3E-for-Timestamp),
and an instant becomes civil local time by combining it with a
[`TimeZone`](https://docs.rs/jiff/0.2.35/jiff/struct.Timestamp.html). Choose
`TimeZone::UTC` for `--utc`, otherwise `TimeZone::system()`. System discovery
honors `TZ`, reads `/etc/localtime` on Unix, and uses the Windows time-zone API
when applicable; its documented failure fallback behaves like UTC but is marked
unknown ([system-zone behavior](https://docs.rs/jiff/0.2.35/jiff/tz/struct.TimeZone.html#method.system)).

The exact format is `%b %e %H:%M:%S.%3f`. Jiff documents `%b` as the English
abbreviated month, `%e` as space-padded day, and the ordinary clock fields
([specifier table](https://docs.rs/jiff/0.2.35/jiff/fmt/strtime/index.html#conversion-specifications));
precision 3 prints exactly three fractional digits and truncates rather than
rounds ([format flags](https://docs.rs/jiff/0.2.35/jiff/fmt/strtime/index.html)).
This matches Go `Jan _2 15:04:05.000`, including `Jan␠␠1` and millisecond
truncation.

The exact [manifest](https://docs.rs/crate/jiff/0.2.35/source/Cargo.toml.orig)
declares version, license, MSRV 1.70, and the selected features. `tz-system`
enables system discovery; `tzdb-zoneinfo` permits named `TZ` resolution from the
host database without the bundled/static database, logging, Serde, JavaScript,
or proc-macro features. Jiff 0.2.35 was published 2026-07-25; current
[crates.io metadata](https://crates.io/api/v1/crates/jiff) reports about 158.6M
downloads and 932 dependent crates. A clean Rust 1.96 probe produced
`Jan  1 12:30:45.987` under `TZ=UTC` and `Jan  1 07:30:45.987` under
`TZ=America/New_York`, passed warnings-denied Clippy and all four shipped
Linux/macOS amd64/arm64 compile checks, and had no finding in the local 1,211-entry
RustSec database.

Credible alternatives:

- [`chrono 0.4.45`](https://crates.io/api/v1/crates/chrono) is the most adopted
  option (MIT OR Apache-2.0, MSRV 1.62) and its `clock` feature exposes
  [`Local`](https://docs.rs/chrono/0.4.45/chrono/struct.Local.html). It can render
  this format, but Jiff provides a more explicit system-time-zone value and
  documented `TZ`/TZif discovery/failure policy for this local-versus-UTC seam.
- [`time 0.3.55`](https://crates.io/api/v1/crates/time) is active and already
  appears transitively in the workspace (MIT OR Apache-2.0, MSRV 1.88), but needs
  `formatting,macros,local-offset`; local offset lookup is fallible rather than a
  full system-zone value ([API](https://docs.rs/time/0.3.55/time/struct.UtcOffset.html#method.local_offset_at)).
  Reusing a transitive crate does not avoid declaring this direct dependency.

Known gap: Go's `time.Local` and Jiff do not promise identical behavior for every
invalid or dynamically changed `TZ` environment. Use Jiff naturally, select the
zone once when constructing the formatter, accept its documented unknown/UTC
fallback, and test UTC plus a DST-observing named zone. No second dependency
review is required: this is synchronous presentation logic, not networking,
storage, cryptography, a runtime, container control, unsafe FFI, or a service.

## CLI option definition and parsing

Approve clap 4.6.6 with derive. Current [crates.io
metadata](https://crates.io/api/v1/crates/clap) identifies the non-yanked
2026-08-06 release, MSRV 1.85, license, very high adoption (over 1.0B downloads
and 44,000 dependent crates), and active maintenance. The crate
[`forbid`s unsafe](https://docs.rs/clap/4.6.6/src/clap/lib.rs.html).

Use an idiomatic derived Rust options struct: `bool` fields with `short`/`long`
for `follow` and `utc`; strings for `since`, `until`, and `tail` (default `100`);
and `Vec<String>` for `machine`. Clap derives `Vec<T>` as repeated `Append`
([field-type rules](https://docs.rs/clap/4.6.6/clap/_derive/index.html#field-types));
adding `value_delimiter = ','` splits every occurrence, so `-m a,b -m c`
produces `a,b,c` ([delimiter API](https://docs.rs/clap/4.6.6/clap/struct.Arg.html#method.value_delimiter)).
The selected minimal features retain typed derive and useful help/errors but omit
color and suggestions; the [feature reference](https://docs.rs/clap/4.6.6/clap/_features/)
documents the tradeoff. Builder-only integration could omit `derive`, but is less
clear for this fixed typed surface.

Semantic gaps from pflag must be deliberate:

- pflag `StringSlice` is CSV-aware, while clap's delimiter is a literal split.
  Ordinary comma/repeat forms match; quoted embedded commas require a custom
  per-occurrence parser if tests or callers establish that contract
  ([pflag source](https://github.com/spf13/pflag/blob/master/string_slice.go)).
- Derived clap booleans are bare `SetTrue`, not pflag's optional explicit
  `--follow=false`; repeated scalar flags conflict instead of pflag last-wins
  unless self-overrides are enabled
  ([`ArgAction`](https://docs.rs/clap/4.6.6/clap/builder/enum.ArgAction.html)).
- To retain pflag's `-n -1` string value, set `allow_hyphen_values = true` on
  `tail` ([API](https://docs.rs/clap/4.6.6/clap/struct.Arg.html#method.allow_hyphen_values)).

These gaps do not block the stated normal surface. Add focused parse tests before
integration; enable explicit-bool/self-override or CSV customization only if the
frozen CLI oracle requires those less-common pflag forms.

[`lexopt 0.3.2`](https://crates.io/api/v1/crates/lexopt) is the best minimalist
alternative (MIT, zero dependencies, active in 2026), but accumulation, comma/CSV
handling, defaults, help, and typed construction become application code.
[`argh 0.1.19`](https://crates.io/api/v1/crates/argh) is maintained and derives
help/repeated values (BSD-3-Clause), but still needs custom delimiter logic.
[`pico-args 0.5.0`](https://crates.io/api/v1/crates/pico-args) and
[`gumdrop 0.8.1`](https://crates.io/api/v1/crates/gumdrop) last released in 2022
and offer no compensating semantic advantage. Clap is the most popular,
maintained, idiomatic fit and should shape the Rust CLI rather than recreating a
pflag `FlagSet` API.
