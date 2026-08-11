# Dependency decision: `terminal-output-profile`

| Field | Decision |
| --- | --- |
| Status | **blocked** |
| Capability | Detect a destination stream's five-level terminal color profile and transform already-rendered ANSI at write time with parity to `github.com/charmbracelet/colorprofile v0.4.3` |
| Selected dependency and exact version | **None**. No reviewed Rust candidate passes the behavior gate. |
| Affected package | Go `upstream/uncloud/internal/cli/tui`; future migration crate `crates/ployz-internal-cli-tui` (no package packet exists at base `147dbcf`) |
| Research base | `147dbcf4e336d45adee266b68b1cdc2eb05c3a81` |

## Verdict

Do not add a dependency for this capability yet. `termprofile 0.2.4` is the
clear leading Rust-native component: it was inspired by Charmbracelet's
`colorprofile`, exposes the same five conceptual profiles, and covers terminfo,
tmux, Windows-version detection, and structured color/style adaptation. It does
not transform already-rendered ANSI, however, and exact probes show different
environment semantics and an RGB-to-ANSI16 mismatch.

Pairing `termprofile 0.2.4` with `anstyle-parse 1.0.0` would supply detection,
structured conversion, and ANSI parsing. It still requires an application-owned
writer with a fresh per-write state machine that interprets every relevant SGR
form, preserves or strips other terminal sequences by profile, maintains exact
write/error semantics, and works around detector/conversion mismatches. That is
most of the requested compatibility layer, not a modest adapter shaped by a
complete dependency.

This is a hard behavior blocker, not a tie between otherwise passing crates.
The controller needs either a narrowed behavior contract or explicit authority
for a bespoke compatibility implementation using separately approved
components.

## Exact oracle capability

The frozen module selects `github.com/charmbracelet/colorprofile v0.4.3`
directly in [`go.mod`](../../upstream/uncloud/go.mod), overriding Lip Gloss
2.0.1's v0.4.2 minimum. The owning primary sources are the exact v0.4.3
[`env.go`](https://github.com/charmbracelet/colorprofile/blob/v0.4.3/env.go),
[`writer.go`](https://github.com/charmbracelet/colorprofile/blob/v0.4.3/writer.go),
and [`profile.go`](https://github.com/charmbracelet/colorprofile/blob/v0.4.3/profile.go).

Observable behavior required by actual callers is:

1. `Detect(writer, env)` distinguishes `NoTTY`, `ASCII`, `ANSI`, `ANSI256`,
   and `TrueColor`. Every frozen Ployz call targets stdout; arbitrary-stream and
   stderr support exist in the Go dependency but are not required by a current
   caller.
2. Detection combines descriptor TTY state, `TTY_FORCE`, `TERM`,
   `COLORTERM`, `NO_COLOR`, `CLICOLOR`, `CLICOLOR_FORCE`, Windows version and
   legacy-console variables, terminfo `Tc`/`RGB`, and `tmux info`. The exact
   implementation uses Go `strconv.ParseBool`, not mere presence or
   non-emptiness, for the four Boolean variables.
3. `NoTTY` strips ANSI sequences including decorations. `ASCII` removes color
   SGR parameters but retains non-color attributes. `ANSI` converts indexed and
   RGB colors to 16 colors. `ANSI256` preserves indexed colors and converts RGB
   to the 256-color palette. `TrueColor` passes bytes through unchanged.
4. For `ASCII`, `ANSI`, and `ANSI256`, non-SGR terminal sequences are passed
   through; the layer is not a terminal-control sanitizer. The separate
   terminal-control parity/security decision in
   [`terminal-styling-layout.md`](terminal-styling-layout.md) remains open.
5. A successful transformed write reports the input length, not the transformed
   byte count, and propagates the underlying write error. The Go writer creates
   fresh parser state per `Write` call.
6. Lip Gloss's package-global stdout writer detects once during package
   initialization. Its `Println` calls therefore use that captured profile.
   The direct image-list `Detect` call reads the then-current environment.
7. Raw `fmt.Print*` paths bypass this layer. Only the `lipgloss.Println` table
   and tree paths use profile-aware output.

The actual profile-aware calls are the table/tree outputs in
`cmd/uc/{ps,volume,service,machine,image,context}` and `cmd/uc/wg`; the direct
profile query in [`cmd/uc/image/ls.go`](../../upstream/uncloud/cmd/uc/image/ls.go)
chooses whether platform pills receive powerline borders. Raw style rendering
elsewhere remains full-fidelity ANSI and must not silently acquire stream
detection.

### Characterized common cases

The exact Go v0.4.3 probe produced the following results for
`ESC[1;38;5;152mX ESC[m`:

| Destination/environment | Profile | Output |
| --- | --- | --- |
| pipe, no variables | `NoTTY` | `X` |
| pipe, `TERM=xterm` | `NoTTY` | `X` |
| pipe, `TERM=xterm CLICOLOR=1` | `NoTTY` | `X` |
| pipe, `TERM=xterm CLICOLOR=0` | `NoTTY` | `X` |
| pipe, `TERM=xterm CLICOLOR_FORCE=1` | `ANSI` | `ESC[1;96mX ESC[m` |
| pipe, `TERM=xterm-256color NO_COLOR=1 CLICOLOR_FORCE=1` | `ANSI256` | `ESC[1;38;5;152mX ESC[m` |
| forced TTY, `TERM=dumb` | `NoTTY` | `X` |
| forced TTY, `TERM=xterm` | `ANSI` | `ESC[1;96mX ESC[m` |
| forced TTY, `TERM=xterm CLICOLOR=0` | `ANSI` | `ESC[1;96mX ESC[m` |
| forced TTY, `TERM=xterm-256color` | `ANSI256` | unchanged |
| forced TTY, `TERM=xterm COLORTERM=truecolor` | `TrueColor` | unchanged |
| forced TTY, `TERM=xterm-256color NO_COLOR=1` | `ASCII` | `ESC[1mX ESC[m` |
| forced TTY, `TERM=xterm-256color NO_COLOR=1 CLICOLOR_FORCE=1` | `ASCII` | `ESC[1mX ESC[m` |
| forced TTY, `TERM=dumb CLICOLOR_FORCE=1` | `ANSI` | `ESC[1;96mX ESC[m` |
| forced TTY, no `TERM`, `CLICOLOR=1` | `NoTTY` | `X` |
| forced TTY, `TERM=xterm NO_COLOR=on` | `ANSI` | `ESC[1;96mX ESC[m` |
| forced TTY, `TERM=dumb CLICOLOR_FORCE=on` | `NoTTY` | `X` |

Two apparent oddities are real v0.4.3 behavior: `CLICOLOR=0` does not disable
color on an otherwise capable TTY, and a redirected stream with both
`NO_COLOR=1` and `CLICOLOR_FORCE=1` is color-enabled because `NO_COLOR` is only
applied in the TTY branch.

`termprofile 0.2.4` matched the first fourteen rows but returned `Ansi16` for
the no-`TERM`/`CLICOLOR=1` row, `NoColor` for `NO_COLOR=on`, and `Ansi16` for
`CLICOLOR_FORCE=on`. The latter two differences follow its documented
`1|true|yes|on` truth set, whereas Go `strconv.ParseBool` does not accept
`yes`/`on`. Source review also found candidate-only `FORCE_COLOR` and CI rules,
different TERM/TERM_PROGRAM and terminfo-max-color rules, normalization of
environment values, and different `WT_SESSION`/ConEmu handling.

## Hard-gate results

| Gate | Result | Evidence |
| --- | --- | --- |
| Five profiles and stdout TTY detection | `termprofile pass conceptually; exact policy fail` | `termprofile` supplies five ordered profiles and injected TTY/environment sources, but exact matrix rows and additional platform rules differ. Other candidates do not supply the five-level model. |
| Exact environment and platform policy | **fail** | `termprofile` is closest but differs on Boolean parsing, absent `TERM` plus `CLICOLOR`, extra force/CI rules, terminfo max colors, and platform special cases. |
| Already-rendered ANSI transformation | **fail** | `termprofile` adapts structured `anstyle` colors/styles only. `anstream` strips or passes. `anstyle-parse` emits low-level parser actions; a complete custom writer/performer is still required. |
| Exact color conversion | **fail** | `termprofile` matches sampled indexed conversions but mismatches at least one RGB-to-ANSI16 fixture; `anstyle-lossy` has broader mismatches. |
| No subprocess | **conflicts with strict parity** | Oracle v0.4.3 executes `tmux info` when `TMUX` is set on a qualifying TTY. Exact tmux RGB behavior and the delegated no-subprocess constraint cannot both hold. |
| Linux/macOS/Windows | `compile pass for evaluated stack; runtime open` | Exact features compiled for the three requested target triples. No native macOS or Windows terminal process was available. |
| License | `pass for reviewed candidates` | `termprofile`, `anstyle-*`, and `anstream` are MIT OR Apache-2.0; `supports-color` is Apache-2.0. |
| Security | `pass for evaluated graph; selection still blocked` | RustSec reported no advisory in the 46-package lock graph using database commit `d0861df1eab469d3c58d6b836ce48b5766e5f217` dated 2026-08-11. No package-owned unsafe is necessary for the probe. |
| Maintenance/MSRV | `pass` | Current stable releases are non-yanked. `termprofile` declares MSRV 1.88; the other candidates declare 1.66 or 1.70, all below Rust 1.96. |

## Candidate comparison

| Candidate at exact version | Primary-source result | Adoption and maintenance | Decision |
| --- | --- | --- | --- |
| [`termprofile 0.2.4`](https://docs.rs/termprofile/0.2.4/termprofile/) with `convert`, `terminfo`, `windows-version` | Exact [`detect.rs`](https://docs.rs/termprofile/0.2.4/src/termprofile/detect.rs.html) provides injected TTY/environment sources, five profiles, terminfo, tmux, and Windows rules. Exact [`convert`](https://docs.rs/termprofile/0.2.4/termprofile/enum.TermProfile.html) adapts structured colors/styles but has no rendered-ANSI writer. Matrix and RGB conversion mismatches are below. `query-detect` was correctly disabled because the oracle performs no active DCS query. | 3,981 total / 1,066 recent downloads and 2 reverse dependents in [crates.io](https://crates.io/api/v1/crates/termprofile); released 2026-05-02, Rust 1.88, source commit `09dffeb`. Maintained but very new and lightly adopted. | **Leading component, rejected as the complete capability: exact policy/conversion fail and the writer remains custom.** |
| [`anstream 1.0.0`](https://docs.rs/crate/anstream/1.0.0) | [`AutoStream`](https://docs.rs/anstream/1.0.0/anstream/struct.AutoStream.html) chooses pass-through, strip, or legacy Windows conversion. Its exact [`choice`](https://docs.rs/anstream/1.0.0/src/anstream/auto.rs.html#197-221) gives `NO_COLOR` priority over force, treats `CLICOLOR=0` as disabling, considers `CI`, and has no color depth or downsampling. | 604.6M total / 162.6M recent downloads and 316 direct reverse dependents in the [crates.io record](https://crates.io/api/v1/crates/anstream); released 2026-02-11 from source commit `3048fe7`. | **Reject: hard behavior failure despite strongest adoption and stream API.** |
| [`anstyle-query 1.1.5`](https://docs.rs/anstyle-query/1.1.5/anstyle_query/) | Its exact [source](https://docs.rs/anstyle-query/1.1.5/src/anstyle_query/lib.rs.html) supplies useful low-level queries only. It treats `NO_COLOR`/force as nonempty, `CLICOLOR` as any value except `0`, recognizes fewer `COLORTERM` values, does not accept an arbitrary environment, and reports neither 256-color depth nor the five-level profile. | 578.6M total / 151.9M recent downloads and 16 direct reverse dependents in [crates.io](https://crates.io/api/v1/crates/anstyle-query); current release from 2025-11-13, source commit `368a871`. | **Reject: not the requested detector.** |
| [`supports-color 3.0.2`](https://docs.rs/crate/supports-color/3.0.2) | Exact [source](https://docs.rs/supports-color/3.0.2/src/supports_color/lib.rs.html) returns basic/256/16m, but force precedes `NO_COLOR`, `NO_COLOR=0` is special, `FORCE_COLOR` and CI affect policy, and no ANSI transformer exists. | 58.1M total / 13.5M recent downloads and 135 reverse dependents in [crates.io](https://crates.io/api/v1/crates/supports-color); last release 2024-11-26, source commit `26ad8d2`. | **Reject: detection and transformation behavior fail.** |
| [`anstyle-parse 1.0.0`](https://docs.rs/anstyle-parse/1.0.0/anstyle_parse/) + [`anstyle-lossy 1.1.5`](https://docs.rs/anstyle-lossy/1.1.5/anstyle_lossy/) | The parser exposes CSI/OSC/DCS actions and can recognize SGR. Lossy exposes RGB/256/16 conversion, but uses its own palette and low-cost distance metric. It supplies no profile detector or writer. | Parser: 591.3M total / 161.1M recent downloads, 14 reverse dependents, released 2026-02-11. Lossy: 5.1M / 1.4M, 4 reverse dependents, released 2026-03-13. See official [parse](https://crates.io/api/v1/crates/anstyle-parse) and [lossy](https://crates.io/api/v1/crates/anstyle-lossy) records; source commits `3048fe7` and `8ed0608`. | **Reject as a complete capability: most behavior remains custom and conversion fails parity. `anstyle-parse` may be reconsidered only after bespoke-implementation authority.** |
| `vte 0.15.0` + custom logic | A general parser for terminal-emulator implementations; does not provide profile policy, stream adaptation, or color conversion. | Maintained and established, but broader than the style-only parser already used by the most-adopted Rust CLI stack. | Reject: weaker fit and still behavior-incomplete. |
| `termcolor 1.4.1` | Styles through structured write calls and selects whether to emit colors. It is not an adapter for composed ANSI and does not expose this five-profile conversion policy. | Established and permissive, but targets a different architecture. | Reject: hard API/behavior mismatch. |

`anstream` remains the idiomatic choice only if the required contract is
deliberately narrowed to binary ANSI pass/strip. `termprofile` is the leading
candidate if a bespoke five-profile compatibility writer is authorized.

## Differential color-conversion probe

The revised probe compared exact Go `colorprofile.Profile.Convert` results with
`termprofile 0.2.4` and `anstyle-lossy 1.1.5` using a conventional xterm
16-color palette. Representative results:

| Input | Go v0.4.3 `ANSI` | `termprofile` | `anstyle-lossy` |
| --- | --- | --- | --- |
| indexed 21 | bright blue (12) | bright blue (12) | bright blue (12) |
| indexed 152 | bright cyan (14) | bright cyan (14) | white (7) |
| indexed 196 | bright red (9) | bright red (9) | bright red (9) |
| RGB `(95,135,175)` | blue (4) | cyan (6) | bright black (8) |
| RGB `(128,200,215)` | bright cyan (14) | bright cyan (14) | white (7) |
| RGB `(255,0,255)` | magenta (13) | bright magenta (13) | bright magenta (13) |

RGB-to-256 agreed for the sampled values, and `termprofile`'s sampled
indexed-to-16 table agreed. Its RGB-to-16 path first quantizes to 256, which is
not equivalent to the oracle's direct `Convert16`; the `(95,135,175)` mismatch
is sufficient to fail strict parity.

## Required features and configuration

None are approved while status is blocked. The proposed leading production
stack, if a bespoke layer receives authority, is:

```toml
termprofile = { version = "=0.2.4", features = ["convert", "terminfo", "windows-version"] }
anstyle-parse = { version = "=1.0.0", features = ["utf8"] }
```

These are the exact leading-stack features, not an approval. `convert` supplies
structured downsampling, `terminfo` supplies the oracle's database lookup, and
`windows-version` supplies OS-build thresholds. Do not enable `query-detect`,
`color-cache`, or Ratatui features. `anstyle-parse`'s default `utf8` feature is
required for non-ASCII caller data. `termprofile` brings `anstyle`, `palette`
(including a proc macro), and `termini`; Windows also brings `os_info` and
`windows-sys`.

For transparent candidate comparison, the actual probe manifest additionally
declared direct `anstyle 1.0.14`, `anstyle-lossy 1.1.5`, and
`anstyle-query 1.1.5`. `anstyle-lossy` was invoked only for the differential
table; `anstyle-query` was compile-only evidence for its separately rejected
candidate. The 46-package audit and cross-compilation claims apply to that full
comparison lockfile, not a minimal leading production graph.

## License, security, and platform notes

- Exact published manifests declare `MIT OR Apache-2.0` and Rust 1.66 for
  [`anstyle-parse`](https://docs.rs/crate/anstyle-parse/1.0.0/source/Cargo.toml),
  [`anstyle-lossy`](https://docs.rs/crate/anstyle-lossy/1.1.5/source/Cargo.toml),
  [`anstyle-query`](https://docs.rs/crate/anstyle-query/1.1.5/source/Cargo.toml),
  and `anstream`; `supports-color` declares Apache-2.0 and Rust 1.70.
- [`termprofile 0.2.4`'s manifest](https://docs.rs/crate/termprofile/0.2.4/source/Cargo.toml)
  declares MIT OR Apache-2.0 and Rust 1.88. Its selected normal dependencies are
  permissively licensed.
- `anstyle-parse` uses internal unsafe around `MaybeUninit`; `anstyle-query`
  uses dependency-owned Windows FFI to enable virtual-terminal processing.
  Both expose safe public APIs. This critical terminal path still requires a
  fresh adversarial dependency review before any later approval.
- Linux runtime output and three-target compilation are not native Windows or
  macOS terminal evidence. Windows profile parity additionally needs OS-build,
  ConEmu, ANSICON, redirected-stream, and virtual-terminal-mode probes.
- Strict oracle parity invokes `tmux info`; the requested no-subprocess rule
  must be resolved explicitly rather than hidden in a crate choice.

## Verification commands and observed evidence

Probes are outside the repository at `/tmp/ployz-terminal-go-probe` and
`/tmp/ployz-output-profile-probe`; they are not project artifacts.

```sh
GOTOOLCHAIN=local GOCACHE=/tmp/ployz-output-go-cache GOWORK=off \
  GOPROXY=off GOSUMDB=off /opt/go1.26.1/bin/go run -buildvcs=false .
GOTOOLCHAIN=local GOCACHE=/tmp/ployz-output-go-cache GOWORK=off \
  GOPROXY=off GOSUMDB=off /opt/go1.26.1/bin/go run -buildvcs=false . convert

cargo run --locked --offline \
  --manifest-path /tmp/ployz-output-profile-probe/Cargo.toml
cargo clippy --locked --offline \
  --manifest-path /tmp/ployz-output-profile-probe/Cargo.toml \
  --all-targets --all-features -- -D warnings
cargo check --locked --offline \
  --manifest-path /tmp/ployz-output-profile-probe/Cargo.toml \
  --target x86_64-unknown-linux-gnu
cargo check --locked --offline \
  --manifest-path /tmp/ployz-output-profile-probe/Cargo.toml \
  --target x86_64-apple-darwin
cargo check --locked --offline \
  --manifest-path /tmp/ployz-output-profile-probe/Cargo.toml \
  --target x86_64-pc-windows-gnu
cargo audit --no-fetch --deny warnings \
  --file /tmp/ployz-output-profile-probe/Cargo.lock
```

Observed: Rust 1.96.0 compiled and Clippy-checked the component probe; all three
target checks passed; RustSec found no advisory. The Rust probe exercised the
termprofile environment matrix and color conversions plus anstyle-parse actions
for `界`, plain ASCII, a non-SGR CSI, OSC, and C0 newline. One SGR was split
across two feed segments while retaining a parser to prove incremental parsing;
those segments were not represented as separate writes. A separate fixture
created a fresh parser for each of two writes split at `ESC[31` / `mX`: the
first parser emitted no action and the second printed `mX`. The matching Go
writer returned input lengths `4` and `2` and preserved the combined raw
`ESC[31mX`, proving that a Rust adapter must flush incomplete raw bytes at each
write boundary and must not retain parser state across writes. The parser
exposes actions, not a transformed writer, confirming the custom-performer and
raw-byte-buffering requirement. Go 1.26.1 executed the frozen exact dependency
versions offline and reproduced the matrix, conversions, and split-write
behavior.

## Known limitations and next decision

1. No native Windows or macOS terminal runtime was available, so platform
   behavior cannot be upgraded from compile evidence.
2. The probe sampled representative colors, not every 256/RGB input. One
   mismatch is sufficient to fail parity; a later deliberate compatibility
   implementation would need exhaustive indexed-color and boundary RGB tests.
3. The environment matrix and parser trace cover the choice-defining behavior,
   but no candidate writer exists to probe for partial writes/errors. A later
   exact implementation must additionally characterize terminfo lookup failure,
   tmux command absence/error/output, Windows build thresholds, duplicate
   environment keys, every relevant SGR form, and partial writes/errors.
4. The output-profile writer must not be presented as a sanitizer. The existing
   human decision over embedded ESC/C0/C1 remains separate.

To unblock, the controller must choose one path:

- narrow the contract to idiomatic binary pass/strip behavior and dispatch a
  revised request, where `anstream 1.0.0` is the leading candidate; or
- authorize a bespoke compatibility layer around `termprofile 0.2.4` and
  `anstyle-parse 1.0.0`, including deliberate fixes for the recorded detector
  and RGB-conversion differences, and decide how the no-subprocess rule replaces
  the oracle's tmux query.

Reviewer results on commits `0278445` and `750272a`: **changes required**. The
first omitted `termprofile 0.2.4`, over-scoped stderr, and under-probed the
candidate stack. The second conflated the proposed and actual probe manifests
and misstated UTF-8/write-boundary parser evidence. Those findings are addressed
above. A fresh adversarial re-review of the revised exact commit is still
required before any component could be approved.
