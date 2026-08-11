# Dependency decision: `terminal-styling-layout`

| Field | Value |
| --- | --- |
| Status | `blocked` |
| Capability | Full-fidelity string styling, ANSI-aware Unicode width/padding, terminal size, and borderless table layout |
| Provisional selected dependencies and exact versions | `console = 0.16.4`; `comfy-table = 8.0.0` |
| Exact Cargo entries if the blockers are cleared | `console = { version = "=0.16.4", default-features = false, features = ["ansi-parsing", "std", "unicode-width"] }`; `comfy-table = { version = "=8.0.0", default-features = false, features = ["custom_styling"] }` |
| License | MIT for both direct crates |
| Research date | 2026-08-11 UTC |
| Base | `a47272fab9ecef37b513d0ad8a47c81c75f86dc4` (`main`) |
| Supersedes | Rejected proposal `3194ff6766c4a315d8bdeee14fbc724f3caa17e7`; inspected with `git show`, not applied |
| Request | Delegated capability request; no request file exists at this base |

## Decision summary

`console 0.16.4` plus `comfy-table 8.0.0` remains the best **technical**
choice for the capability in this record. On the tested 256-color Linux
terminal, the exact configuration below matches the oracle's visible styles,
ANSI-aware Unicode widths, borderless shape, bold headers, and three spaces of
right padding. It also preserves the oracle's raw styled-string behavior by
forcing styling at string construction and table rendering time.

It is not approved. Four gates remain open:

1. `console` emits byte-different SGR sequences. Most were visually equivalent
   in the tested terminal, but Console encodes bright colors as ANSI-256 rather
   than ANSI 90-97, so even presentation parity on an ANSI16-only terminal is
   not proved. A human parity authority must decide the required authority.
2. Oracle and Rust probes both preserve embedded ESC, C0, and C1 controls from
   values. A human must choose oracle preservation or visible escaping at the
   trust boundary because this is a parity/security conflict.
3. Some callers use Lip Gloss's profile-aware output functions. Profile
   detection and ANSI downsampling are a separate capability not provided by
   either provisional dependency; the precise dependency request appears below.
4. Only Linux runtime behavior and cross-target compilation were verified.
   Native macOS and Windows runtime verification, or an explicit verification
   exception, remains required.

A new fresh adversarial dependency review is required after those decisions and
the separate output-profile decision are recorded. Until then this record stays
`blocked` and must not be added to `migration/DEPENDENCIES.tsv` as approved.

## Oracle and caller evidence: three distinct layers

### 1. Full-fidelity string construction

[`style.go`](../../upstream/uncloud/internal/cli/tui/style.go) calls
`lipgloss.Style.Render` to construct reusable strings containing bold, faint,
underline, named colors, bright blue, and ANSI-256 color 152. Lip Gloss v2
documents and implements `Style.Render` as full-fidelity rendering; it does not
inspect a destination or environment. The v2
[`UPGRADE_GUIDE`](https://github.com/charmbracelet/lipgloss/blob/v2.0.1/UPGRADE_GUIDE_V2.md#printing-and-color-downsampling)
explicitly moved downsampling from `Style.Render` to the output layer.

Many callers then write those raw strings with `fmt.Println`, `fmt.Printf`, or
`fmt.Fprintln`, for example
[`cmd/uc/deploy.go`](../../upstream/uncloud/cmd/uc/deploy.go),
[`cmd/uc/service/scale.go`](../../upstream/uncloud/cmd/uc/service/scale.go), and
[`internal/cli/machine.go`](../../upstream/uncloud/internal/cli/machine.go).
`fmt` does not strip or downsample ANSI. Redirection therefore retains the raw
escape bytes for these call sites. The rejected proposal's rule to turn string
styling off whenever the eventual destination is not a TTY would change this
observable behavior.

[`table.go`](../../upstream/uncloud/internal/cli/tui/table.go) likewise constructs
a full-fidelity table string. The table itself has no TTY policy: no borders or
separators, bold headers, and right padding of three spaces on every column,
including the final column.

### 2. Profile-aware output

Other callers deliberately use `lipgloss.Println`, including
[`cmd/uc/volume/ls.go`](../../upstream/uncloud/cmd/uc/volume/ls.go),
[`cmd/uc/service/ls.go`](../../upstream/uncloud/cmd/uc/service/ls.go),
[`cmd/uc/machine/ls.go`](../../upstream/uncloud/cmd/uc/machine/ls.go), and
[`cmd/uc/image/ls.go`](../../upstream/uncloud/cmd/uc/image/ls.go). Lip Gloss's
[`writer.go`](https://github.com/charmbracelet/lipgloss/blob/v2.0.1/writer.go)
passes already-rendered ANSI through `colorprofile.Writer`, which detects a
five-level profile and strips or downsamples at write time. Image listing also
calls `colorprofile.Detect` directly to choose pill borders.

Thus the actual contract is call-site specific:

- `Style.Render`/table rendering followed by Go `fmt`: ANSI is retained even
  when redirected.
- `Style.Render`/table rendering followed by Lip Gloss output functions: the
  output profile may strip color, retain decorations only, or downsample color.
- Styling must not be made globally conditional on stdout or stderr TTY state.

### 3. Interactive terminal behavior

[`prompt.go`](../../upstream/uncloud/internal/cli/tui/prompt.go) checks stdin,
stdout, and stderr separately and maps failed or non-positive stdout size to
zero. [`spinner.go`](../../upstream/uncloud/internal/cli/tui/spinner.go) has its
own policy: unless stdin and stderr are both terminals it writes a plain title
to stderr. That explicit fallback is not evidence that ordinary `Style.Render`
is destination-aware.

The only existing upstream `tui` test covers RTT formatting. The future packet
therefore needs characterization tests for each output path above.

## Lip Gloss output-profile truth table

Primary source is `colorprofile 0.4.3`
[`env.go`](https://github.com/charmbracelet/colorprofile/blob/v0.4.3/env.go) and
[`writer.go`](https://github.com/charmbracelet/colorprofile/blob/v0.4.3/writer.go),
the exact version in the oracle's `go.mod` (commit
[`d085584`](https://github.com/charmbracelet/colorprofile/tree/d085584efb48f2ad470e96cd0f3dcb8cc68a034b)).
The oracle's Lip Gloss v2.0.1 snapshot is commit
[`bffdafb`](https://github.com/charmbracelet/lipgloss/tree/bffdafb703dd8ff09fafe4e410d29c7673ef2fdb).
The table was reproduced with the Go probe described below. Environment values
shown as `1` are parsed with Go's `strconv.ParseBool`; merely setting an empty
value does not activate them.

The input for every row was the raw `NameStyle` rendering
`ESC[1;38;5;152mX ESC[m` (the space before `ESC[m` is only for readability).

| Destination and environment | Detected profile | Exact output represented as an escaped string |
| --- | --- | --- |
| pipe, no relevant environment | `NoTTY` | `"X"` |
| pipe, `TERM=xterm` | `NoTTY` | `"X"` |
| pipe, `TERM=xterm CLICOLOR=1` | `NoTTY` | `"X"` |
| pipe, `TERM=xterm CLICOLOR=0` | `NoTTY` | `"X"` |
| pipe, `TERM=xterm CLICOLOR_FORCE=1` | `ANSI` | `"\x1b[1;96mX\x1b[m"` |
| pipe, `TERM=xterm-256color NO_COLOR=1 CLICOLOR_FORCE=1` | `ANSI256` | `"\x1b[1;38;5;152mX\x1b[m"` |
| TTY (or pipe with `TTY_FORCE=1`), `TERM=dumb` | `NoTTY` | `"X"` |
| TTY, `TERM=xterm` | `ANSI` | `"\x1b[1;96mX\x1b[m"` |
| TTY, `TERM=xterm CLICOLOR=0` | `ANSI` | `"\x1b[1;96mX\x1b[m"` |
| TTY, `TERM=xterm-256color` | `ANSI256` | `"\x1b[1;38;5;152mX\x1b[m"` |
| TTY, `TERM=xterm COLORTERM=truecolor` | `TrueColor` | `"\x1b[1;38;5;152mX\x1b[m"` |
| TTY, `TERM=xterm-256color NO_COLOR=1` | `ASCII` | `"\x1b[1mX\x1b[m"` |
| TTY, `TERM=xterm-256color NO_COLOR=1 CLICOLOR_FORCE=1` | `ASCII` | `"\x1b[1mX\x1b[m"` |
| TTY, `TERM=dumb CLICOLOR_FORCE=1` | `ANSI` | `"\x1b[1;96mX\x1b[m"` |
| TTY forced with `TTY_FORCE=1`, no `TERM`, `CLICOLOR=1` | `NoTTY` | `"X"` |

`TTY_FORCE=1` affects profile detection only; it makes a non-TTY file descriptor
enter the TTY branch. `TERM` absent or `TERM=dumb` still suppresses styling
unless `CLICOLOR_FORCE=1` applies. Notably, in this exact implementation
`CLICOLOR=0` does not disable color on an otherwise color-capable TTY, and the
pipe + `NO_COLOR=1 CLICOLOR_FORCE=1` combination yields color rather than the
documented TTY precedence. These quirks are part of the oracle version's
observable behavior and must be characterized rather than normalized silently.

Downsampling behavior is:

| Profile | Writer behavior |
| --- | --- |
| `NoTTY` | Strip ANSI sequences, including decorations |
| `ASCII` | Remove colors but retain non-color SGR attributes such as bold, faint, and underline |
| `ANSI` | Convert indexed/true color to the 16-color palette; index 152 became bright cyan SGR 96 in the probe |
| `ANSI256` | Preserve indexed colors and convert true color to the 256-color palette |
| `TrueColor` | Pass bytes through unchanged |

`console 0.16.4` has automatic on/off styling, but its
[`default_colors_enabled`](https://github.com/console-rs/console/blob/0.16.4/src/utils.rs#L15-L112)
rules and formatting path neither implement this five-profile matrix nor
downsample ANSI-256/true color. It cannot be credited with this capability.

## Exact SGR characterization

The following values were emitted by isolated Go and Rust probes under forced
full-fidelity styling. `ESC` denotes byte `0x1b`.

| Oracle style | Go `lipgloss 2.0.1` bytes | Provisional Rust `console 0.16.4` bytes | Result |
| --- | --- | --- | --- |
| no style | `X` | `X` | exact |
| faint | `ESC[2mX ESC[m` | `ESC[2mX ESC[0m` | visible match; reset differs |
| red | `ESC[31mX ESC[m` | `ESC[31mX ESC[0m` | visible match; reset differs |
| bold | `ESC[1mX ESC[m` | `ESC[1mX ESC[0m` | visible match; reset differs |
| bold red | `ESC[1;31mX ESC[m` | `ESC[31m ESC[1mX ESC[0m` | visible match; combined/order/reset differ |
| bold color 152 | `ESC[1;38;5;152mX ESC[m` | `ESC[38;5;152m ESC[1mX ESC[0m` | visible match; combined/order/reset differ |
| underlined bright blue | `ESC[4;94;4mX ESC[m` | `ESC[38;5;12m ESC[4mX ESC[0m` | same palette slot on the tested 256-color terminal; encoding/order/reset differ; ANSI16-only support can differ; Go repeats SGR 4 |

The exact Go bytes include its unusual duplicated underline parameter. Console's
typed API serializes colors before attributes as separate SGR sequences, uses
`ESC[0m`, and represents bright blue as indexed color 12. Method call order does
not change that serialization. Indexed 12 and standard bright-blue SGR 94 map
to the same conventional palette entry on 256-color terminals, but an
ANSI16-only terminal need not support `38;5;12`; this is a possible presentation
difference, not only a snapshot difference. Exact/ANSI16 parity would require
hand-written escape serialization, a low-level adapter whose purpose is to
imitate Go, or a different dependency. That conflicts with the port's
idiomatic-dependency rule and is not justified without a parity-authority
decision.

The same difference appears in table headers. With the required configuration,
the mixed styled Unicode fixture produced these strings:

```text
Go:   "\x1b[1mNAME\x1b[m   \x1b[1mVALUE\x1b[m   \n\x1b[32m界\x1b[mé    x       "
Rust: "\x1b[1mNAME\x1b[0m   \x1b[1mVALUE\x1b[0m   \n\x1b[32m界\x1b[0mé    x       "
```

Both lines have visible width 15; `界` has width 2 and `e` plus combining acute
has width 1. The shape, padding, and visual styles match, but the bytes do not.

## Primary-source evidence for the provisional dependencies

- [`console::Style 0.16.4`](https://docs.rs/console/0.16.4/console/struct.Style.html)
  provides bold, dim, underline, named colors, ANSI-256 colors,
  `force_styling`, and explicit stdout/stderr association.
  [`measure_text_width`](https://docs.rs/console/0.16.4/console/fn.measure_text_width.html),
  [`pad_str`](https://docs.rs/console/0.16.4/console/fn.pad_str.html), and
  [`Term::size_checked`](https://docs.rs/console/0.16.4/console/struct.Term.html#method.size_checked)
  cover non-table width/padding and terminal size.
- The exact `console 0.16.4`
  [manifest](https://docs.rs/crate/console/0.16.4/source/Cargo.toml) declares
  Rust 1.71, MIT, and default features `std`, `unicode-width`, and
  `ansi-parsing`.
- [`comfy-table::Table 8.0.0`](https://docs.rs/comfy-table/8.0.0/comfy_table/struct.Table.html)
  exposes `enforce_styling`, `style_text_only`, and `to_string`/`Display`;
  [`NOTHING`](https://docs.rs/comfy-table/8.0.0/comfy_table/presets/constant.NOTHING.html)
  removes all borders and separators; columns expose
  [`set_padding`](https://docs.rs/comfy-table/8.0.0/comfy_table/struct.Column.html#method.set_padding).
- The exact `comfy-table 8.0.0`
  [manifest](https://docs.rs/crate/comfy-table/8.0.0/source/Cargo.toml)
  declares MIT and shows that `custom_styling` uniquely enables `ansi-str`,
  `console`, and `tty`. `reexport_crossterm` is not needed. Its
  [`content_format.rs`](https://github.com/nukesor/comfy-table/blob/v8.0.0/src/utils/formatting/content_format.rs#L278-L328)
  applies `style_text_only` before padding, which is necessary to keep the three
  padding spaces outside the header style like Lip Gloss.
- Exact reviewed source snapshots are
  [`console 0.16.4` commit `598eca9`](https://github.com/console-rs/console/tree/598eca9fe9e3f9b93d2b49fdccf2d395d809bd94)
  and
  [`comfy-table 8.0.0` commit `cf91e01`](https://github.com/nukesor/comfy-table/tree/cf91e018992e1df48aa2008ab8a9d3246d6b1d2c).
  `comfy-table` forbids unsafe code; `console` encapsulates Unix libc and Windows
  console FFI behind safe public APIs.

## Hard-gate results

| Gate | Result | Evidence and remaining work |
| --- | --- | --- |
| Visible styling and layout | `pass on tested 256-color Linux terminal; provisional elsewhere` | Linux probe covered every oracle style, ANSI-aware mixed Unicode width, no borders, bold headers, `(0, 3)` padding, and final-column trailing spaces; ANSI16 bright-color presentation remains open |
| Exact byte parity | `open` | SGR and reset differences above require human parity authority |
| Output profile/downsampling | `open; separate dependency request` | Required by actual `lipgloss.Println` and direct `colorprofile.Detect` callers; neither provisional dependency supplies it |
| Terminal-control handling | `open; human decision required` | Both oracle and Rust preserve injected ESC/C0/C1 controls; security and strict parity conflict |
| License | `pass for provisional pair` | Both direct crates are MIT; the probed target-inclusive closure used only permissive/compatible licenses |
| Security and unsafe FFI | `pass pending adversarial review` | `cargo audit` found no advisory in 34 locked packages using RustSec DB `d0861df1eab469d3c58d6b836ce48b5766e5f217` dated 2026-08-11; package-owned `unsafe` is not needed, but `console`/terminal transitives contain platform FFI |
| Rust/toolchain | `pass` | `console` declares MSRV 1.71; `comfy-table` uses edition 2024 (Rust 1.85 floor); exact pair compiled with Rust 1.96.0 |
| Linux runtime | `pass` | Native Linux process and pseudo-TTY probes passed |
| macOS runtime | `open` | `x86_64-apple-darwin` compile passed; no native macOS process or terminal was run |
| Windows runtime | `open` | `x86_64-pc-windows-gnu` compile passed; no native Windows console, redirection, or Unicode-width process was run |
| Synchronous architecture | `pass for provisional pair` | Formatting is synchronous and brings no async runtime, shell, process, or service API |

## Candidate comparison

| Candidate | Hard-gate and integration result | Decision |
| --- | --- | --- |
| `console 0.16.4` + `comfy-table 8.0.0` | Complete visible styling/layout API, shared console dependency, strong adoption, current releases, exact versions compile on all target triples | **Provisional selection; blocked on the gates above** |
| `anstyle 1.0.14` + `anstream 1.0.0` + `comfy-table 8.0.0` | Credible style/stream APIs, but `custom_styling` still brings `console`; adds a second styling stack and still needs a separate exact profile/downsampling proof | Rejected for redundant integration surface |
| `owo-colors 4.3.0` + `comfy-table 8.0.0` | Credible styles and conditional color, but table still brings `console`; terminal size and non-table ANSI width remain separate | Rejected for weaker cohesion |
| `tabled 0.21.0` + styling/output crates | ANSI-aware and maintained, but broader table transformation API and lower direct fit for the small fixed layout | Rejected for excess integration surface |
| `termcolor 1.4.1` + table/width crates | Stream-oriented automatic color is awkward for composed styled strings and does not close layout/profile parity alone | Rejected on API fit and completeness |
| `nu-ansi-term 0.50.3` or `colored 3.1.1` + table/width crates | Incomplete combined capability; `colored` also uses MPL-2.0 rather than the requested permissive posture | Rejected on completeness; `colored` fails license gate |
| Hand-written table/ANSI renderer | Could force byte identity | Reimplements ANSI parsing, Unicode width, styling, and table layout specifically to mimic Go | Rejected unless human exact-byte authority overrides the idiomatic-dependency rule |

The candidate comparison does not select an output-profile/downsampling crate.
That is intentionally delegated to the separate dependency request below.

## Required features and exact table configuration

Only an integrator or dependency steward may add the exact Cargo entries from
the header after this record is approved. Disable `console` defaults and enable
exactly `std`, `unicode-width`, and `ansi-parsing` (the same three features as
its 0.16.4 defaults, stated explicitly for the dependency gate). For
`comfy-table`, disable defaults and enable only `custom_styling`; it already
implies `tty`. Do not enable `windows-console-colors`, `reexport_crossterm`, or
direct dependencies on `unicode-width`, `ansi-str`, or `crossterm`.

Required construction rules if visual parity is authorized:

1. Reusable string styles use typed `console::Style` values with
   `force_styling(true)`. This matches raw `Style.Render`; do **not** decide
   string styling from stdout/stderr TTY state.
2. Use `console::measure_text_width` and `pad_str` for non-table composed ANSI
   strings. Map `Term::stdout().size_checked()` from `(rows, columns)` to
   columns and preserve the oracle's zero-on-error/non-positive behavior.
3. Build tables with `Table::new()`, `load_style(NOTHING)`,
   `force_no_tty()`, then `enforce_styling()`, and a bold `Cell` for every
   header. `force_no_tty()` prevents ambient terminal width/styling decisions;
   `enforce_styling()` then restores raw cell attributes deterministically.
   Call `style_text_only()` separately (it returns `()`) so the reset occurs
   before padding. After headers/rows have created columns, call
   `column.set_padding((0, 3))` for every column.
4. Render with `to_string()`/`Display`; do not use a trimming formatter because
   the last column's three trailing spaces are observable.
5. Port a mixed fixture containing an internally styled `界` followed by
   `e\u{301}` and assert equal visible widths for header/data lines, no borders,
   header style ending before padding, and exactly three trailing spaces.
6. Output of the resulting full-fidelity string must be handled at the call
   site: raw `fmt`-equivalent writes remain raw, while profile-aware writes use
   the separately approved dependency.

## Separate dependency request: terminal output-profile detection and downsampling

This is a unique external capability and must receive its own request, decision,
and fresh researcher. The controller should create
`migration/dependencies/requests/terminal-output-profile-downsampling.md` with:

```text
Capability: Detect a destination stream's terminal color profile and transform
already-rendered ANSI at write time with behavioral parity to Go
github.com/charmbracelet/colorprofile v0.4.3 as used by Lip Gloss v2.0.1.

Required behavior:
- Treat stdout and stderr independently and accept already-rendered composed ANSI.
- Reproduce TTY detection plus NO_COLOR, CLICOLOR, CLICOLOR_FORCE, TTY_FORCE,
  TERM, and COLORTERM precedence/quirks in the truth table in
  terminal-styling-layout.md.
- Implement profiles NoTTY, ASCII, ANSI, ANSI256, and TrueColor.
- NoTTY strips ANSI; ASCII removes colors but retains decorations; ANSI and
  ANSI256 downsample higher color depths; TrueColor passes through.
- Reproduce direct profile queries used to choose layout glyphs, not only a
  writer wrapper.
- Preserve call-site distinction: raw writes do not invoke the transformer.
- Define treatment of non-SGR ESC/C0/C1 only after the terminal-control
  parity/security decision; the profile layer is not a sanitizer.

Platforms: Linux, macOS, and Windows native runtime behavior; stdout/stderr TTY
and redirected files/pipes.
Constraints: synchronous; no async runtime, shell, subprocess, or service;
permissive Apache-2.0-compatible license; Rust 1.96 compatibility; safe public API.

Oracle evidence: lipgloss/writer.go v2.0.1, colorprofile env.go/writer.go v0.4.3,
cmd/uc/{volume,service,machine,image,context}/ listing call sites, cmd/uc/ps.go,
cmd/uc/wg/wg.go, and cmd/uc/image/ls.go direct Detect call.
```

Do not add `anstream`, `supports-color`, `colorchoice`, or another crate to this
record without that independent hard-gate research and exact matrix probe.

## Terminal-control injection: required human decision

The provisional APIs treat dynamic text as bytes/string content, but that does
not make it terminal-inert. Oracle table cells contain server- or user-derived
service names, volume names/drivers, machine names, image references, endpoints,
container status, and similar values. An embedded `ESC[2J`, OSC sequence, BEL
(C0), or 8-bit CSI (C1 U+009B) can be interpreted by a terminal.

The probes used `"a\x1b[2Jb\x07c\u009bd"`. Both Go `Style.Render` and Rust
`Style::apply_to(...).force_styling(true)` preserved every embedded control
payload inside their outer red style. `Cell::new`/`to_string` likewise is not a
sanitization boundary. `custom_styling` parses trusted ANSI for width; it must
not be treated as validating untrusted content.

One of these choices must be recorded before approval:

- **Preserve oracle parity:** pass embedded ESC/C0/C1 through unchanged and add
  characterization tests that make the risk explicit.
- **Escape for terminal safety:** before styling or width calculation, visibly
  encode terminal controls from untrusted fields (for example `ESC` as
  `\\x1b`, other C0/C1 as an agreed visible form), and record this deliberate
  security divergence plus exact width fixtures.

Escaping only at table rendering is insufficient because raw styled strings and
progress/log output have the same injection surface. Conversely, the
profile/downsampling writer is not a security boundary: raw `fmt` paths bypass
it and non-SGR sequences can pass through profile conversion.

## Verification commands and evidence

The isolated probes are outside the repository at
`/tmp/ployz-terminal-go-probe` and `/tmp/ployz-terminal-probe`; they are not
project artifacts.

```sh
GOTOOLCHAIN=go1.25.12 GOCACHE=/tmp/ployz-go-cache GOWORK=off GOPROXY=off \
  go run -buildvcs=false .

cargo run --locked --offline \
  --manifest-path /tmp/ployz-terminal-probe/Cargo.toml
cargo clippy --locked --offline \
  --manifest-path /tmp/ployz-terminal-probe/Cargo.toml \
  --all-targets --all-features -- -D warnings
cargo check --locked --offline \
  --manifest-path /tmp/ployz-terminal-probe/Cargo.toml \
  --target x86_64-unknown-linux-gnu
cargo check --locked --offline \
  --manifest-path /tmp/ployz-terminal-probe/Cargo.toml \
  --target x86_64-apple-darwin
cargo check --locked --offline \
  --manifest-path /tmp/ployz-terminal-probe/Cargo.toml \
  --target x86_64-pc-windows-gnu
cargo audit --no-fetch --deny warnings \
  --file /tmp/ployz-terminal-probe/Cargo.lock
```

Observed:

- Go probe ran under Go 1.25.12, including an actual Linux pseudo-TTY and pipe,
  and reproduced the raw bytes, table bytes, control injection, and profile
  matrix above.
- Rust probe ran under Rust 1.96.0, printed the exact SGR bytes above, preserved
  the control fixture, and reported mixed table line widths `[15, 15]`.
- Warnings-denied all-target/all-feature Clippy passed.
- All three target compile checks passed. The macOS and Windows results are
  compile evidence only, not runtime evidence.
- RustSec scanned 34 locked packages with the database commit stated in the
  hard-gate table and exited successfully with no advisory finding.

## Known limitations

- Unicode display width follows `unicode-width`/terminal conventions; emulator
  handling of ambiguous-width characters and new emoji can differ. Test exact
  fixtures rather than promising pixel-level identity.
- `comfy-table/custom_styling` adds ANSI parsing and TTY/platform transitives and
  is slower than unstyled table rendering. It is required for composed ANSI
  strings and accurate widths.
- `enforce_styling()` bypasses Comfy Table's own TTY gate only. Built-in cell
  colors are serialized by Crossterm 0.29, which independently suppresses color
  when `NO_COLOR` is nonempty and does no profile downsampling; attributes such
  as bold and pre-rendered forced Console ANSI are separate. The required table
  configuration uses built-in bold headers and pre-rendered forced string
  colors, and must not be generalized into a claim that all Comfy styling is
  environment-independent.
- `comfy-table` documents a stable/feature-frozen posture while still shipping
  fixes. Recheck maintenance before any future major upgrade.
- Platform compile checks do not exercise Windows console mode, macOS terminal
  sizing, native redirection, or emulator-specific width.
- The provisional dependencies cover safe public APIs but encapsulate platform
  FFI in dependencies. This is why a new fresh adversarial review is required.

## Review and blockers

Reviewer result: **pending; no prior review may be reused.** A new fresh
adversarial dependency reviewer must re-read this revised record, the oracle,
the exact source tags, the probe outputs, the output-profile decision, and both
human choices. Review focus:

- raw Render/`fmt` versus profile-aware output separation;
- exact environment and downsampling matrix, including its quirks;
- exact SGR differences and the recorded parity authority;
- `NOTHING` + `force_no_tty` + `enforce_styling` + `style_text_only` +
  `(0, 3)` + `to_string`;
- mixed styled Unicode widths and final-column spaces;
- terminal-control injection and the recorded preserve/escape policy;
- native Linux/macOS/Windows runtime evidence or explicit exception;
- unsafe FFI, license, advisory, maintenance, and exact feature surface.

Blockers/requests, exactly:

1. Human parity authority: exact SGR bytes or visual/layout parity.
2. Human security authority: preserve or visibly escape ESC/C0/C1 from
   untrusted values.
3. New dependency request and approved decision for
   `terminal-output-profile-downsampling`.
4. Native macOS and Windows runtime probes, or a recorded verification
   exception under `PORTING.md`.
5. New fresh adversarial dependency re-review after items 1-4 are resolved.

Affected package packet: future packet for
`upstream/uncloud/internal/cli/tui` (none exists at this base). The separate
profile request also affects the command packages named above. No package may
use the provisional dependencies until the controller validates a later
approved revision of this record.
