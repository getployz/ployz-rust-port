# Dependency decision: `ansi-aware-unicode-grapheme-display-width-truncation`

| Field | Value |
| --- | --- |
| Status | `blocked` -- no inspected dependency satisfies the hard behavior and architecture gates |
| Selected dependency and exact version | None |
| Required features and configuration | None approved |
| License | Not applicable; no dependency is selected |
| Research date | 2026-08-12 UTC |
| Request | Existing registry row; no separate request record |
| Affected package | `pkg/client/deploy/operation` |

## Decision

Do not approve a replacement dependency under the durable prospective
routine-dependency authority. In particular, do not approve the initially
promising `vtparse 0.7.0`: adversarial source review found observable string
payload, malformed-input, and panic-safety failures that its public API cannot
repair without a package-local ANSI state machine.

`unicode-segmentation 1.13.3` passes the grapheme-boundary and platform gates in
isolation with `default-features = false`. It does not provide ANSI
classification or cell width and therefore is not an approval for this
capability. `unicode-width 0.2.2` is also rejected: differential review found
observable width differences from the exact Go `displaywidth 0.11.0` oracle.

The existing `console 0.16.4` authority remains observably insufficient. Its
public `AnsiCodeIterator` consumes a limited ESC/CSI grammar, does not consume
OSC strings, and recognizes only the DCS introducer rather than its payload and
terminator. OSC-8 and DCS payload bytes can consequently be measured or split
as visible text.

## Oracle contract

The frozen package calls `ansi.Truncate(command, maximumWidth, "…")` in
[`predeploy.go`](../../upstream/uncloud/pkg/client/deploy/operation/predeploy.go).
Its exact Go dependency is `github.com/charmbracelet/x/ansi v0.11.7`.

Primary oracle evidence:

- [`truncate.go`](https://github.com/charmbracelet/x/blob/ansi/v0.11.7/ansi/truncate.go)
  walks the original byte stream, preserves non-visible sequence bytes after a
  visible cut, and calls `FirstGraphemeCluster` on the original suffix. It does
  **not** remove ANSI and segment one reconstructed visible stream.
- [`truncate_test.go`](https://github.com/charmbracelet/x/blob/ansi/v0.11.7/ansi/truncate_test.go)
  requires SGR, DCS, OSC-8 using ESC/ST and raw C1 OSC/ST, combining and emoji
  graphemes, wide characters, newlines, ellipsis budgeting, and exact output.
- The exact generated
  [`transition_table.go`](https://github.com/charmbracelet/x/blob/ansi/v0.11.7/ansi/parser/transition_table.go)
  recognizes ESC and raw C1 forms of CSI, DCS, OSC, SOS, PM, APC, and ST. OSC
  accepts BEL, ESC-ST, and C1 ST termination. DCS accepts ESC-ST and C1 ST.
  More generally, any ESC exits OSC, DCS, SOS, PM, or APC into Escape state;
  the following byte need not be `\`. CAN/SUB terminate those strings with an
  ignore action rather than the ground-state execute action, which changes
  whether `Truncate` preserves the byte after visible truncation begins.
  DCS payload overrides bytes `0x80..=0xff` except ST; OSC payload overrides
  `0x20..=0xff` except its terminators. SOS/PM/APC accept their specified byte
  ranges and allow the table's remaining raw C1 transitions to take effect.
  Global UTF-8-lead transitions remain observable in states where later
  state-specific entries do not override them, including malformed control
  states. OSC and DCS string states instead override those bytes as payload.

These are byte-output and cell-width requirements. Semantic decoding of CSI,
OSC, or DCS commands is unnecessary, but exact state boundaries are not.

## Hard-gate results

| Gate | Primary evidence and result |
| --- | --- |
| Behavior | **Fail.** No inspected crate matches the oracle's combination of ESC/C1 introducers, OSC/DCS high-byte payload rules, SOS/PM/APC transitions, BEL/ST termination, and malformed-state UTF-8 behavior. Concrete failures are detailed below. |
| Lossless narrow seam | **Fail.** Candidates that expose original ranges omit required controls; candidates with broader state machines expose only semantic callbacks or insufficient state to correct their transition differences. Repairing them requires recognizing string state and terminators locally, which is the forbidden second ANSI parser. |
| Graphemes and width | **Fail as a combined capability.** [`unicode-segmentation 1.13.3`](https://github.com/unicode-rs/unicode-segmentation/tree/66a032fd8d667bc47ac5b640b151dff3f5356d07) provides UAX #29 extended grapheme iteration and declares Rust 1.85, but the package must segment the oracle's original suffix, not a control-stripped concatenation. [`unicode-width 0.2.2`](https://github.com/unicode-rs/unicode-width/tree/v0.2.2) declares Rust 1.66 but does not match the oracle's cluster-width policy: `U+0600` followed by `A` is one grapheme measured as 0 by [`displaywidth 0.11.0`](https://github.com/clipperhouse/displaywidth/blob/v0.11.0/width.go) from its first code point, but as 2 by `UnicodeWidthStr::width`; `⌛` plus VS15 is 2 versus 1. Its `cjk` feature can expose `width_cjk`, but selecting a method at each call still must reproduce the oracle's process-wide [`RUNEWIDTH_EASTASIAN`](https://github.com/charmbracelet/x/blob/ansi/v0.11.7/ansi/method.go) switch. Cell width is an additional unresolved dependency blocker. |
| License and security | **No selected graph.** The rejected established focused parser graphs use acceptable MIT or MIT/Apache-2.0 licensing. A RustSec probe at database commit `69f93e1d081d8b6fbee010e48f0b5e0d13661415` (1,216 loaded advisories, 2026-08-12) found no advisory in the provisional `vtparse 0.7.0`, `utf8parse 0.2.2`, `unicode-segmentation 1.13.3`, and `unicode-width 0.2.2` graph. Advisory cleanliness does not cure the behavior failures. The `emux-vt 0.2.0` artifact declares MIT but does not package a license file, an additional unresolved license-evidence concern. |
| Platforms and build | **Pass only for the rejected provisional graph.** Its exact published manifests set `build = false`, `cargo tree -e normal` contains only the four external crates listed below, source inspection found no FFI, and it compiled on Rust 1.96 for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, and `x86_64-pc-windows-gnu`. `unicode-segmentation` denies unsafe and `unicode-width` forbids it; rejected `vtparse` and `utf8parse` contain internal table/validated-code-point unsafe. No package unsafe or native/system library was required by the probe. No passing behavior/width graph exists to approve. |
| Maintenance and adoption | **Fail for the closest new alternative.** The established candidates fail behavior. The 2026-08-12 [`emux-vt` crates.io snapshot](https://crates.io/api/v1/crates/emux-vt) and [repository snapshot](https://api.github.com/repos/IISweetHeartII/emux) show two March 2026 releases, about 374 total downloads, and eight stars; it also fails behavior. That is not durable prospective routine-dependency evidence. |

## Candidate comparison and rejected alternatives

| Candidate | Primary source and ecosystem evidence | Rejection |
| --- | --- | --- |
| [`vtparse 0.7.0`](https://crates.io/api/v1/crates/vtparse) | Established WezTerm focused parser. Exact published source is commit [`3bfe6df191af37bc33fe5fe6de12a40f7067f101`](https://github.com/wezterm/wezterm/tree/3bfe6df191af37bc33fe5fe6de12a40f7067f101/vtparse); the public seam is `VTParser`, `VTActor`, and `is_ground`. | Its [`transitions.rs`](https://github.com/wezterm/wezterm/blob/3bfe6df191af37bc33fe5fe6de12a40f7067f101/vtparse/src/transitions.rs) lets global C1 transitions interrupt high-byte OSC/DCS payload that the oracle retains. UTF-8 in DCS/SOS/PM/APC can enter `Utf8Sequence`, while [`next_utf8`](https://github.com/wezterm/wezterm/blob/3bfe6df191af37bc33fe5fe6de12a40f7067f101/vtparse/src/lib.rs) only handles Ground and OSC return states and can panic. UTF-8 in malformed CSI is swallowed or cannot reproduce the oracle's visible reset behavior. Public `is_ground` cannot reveal the prior string state needed to repair these cases. The rejected `std` feature also buffers OSC/APC payloads in growable vectors, an input-proportional resource risk for a boundary-only seam. **Fails behavior, panic-safety, and seam gates.** |
| [`emux-vt 0.2.0`](https://crates.io/api/v1/crates/emux-vt) | Small, zero-normal-dependency parser with Rust 1.85 MSRV and an MIT manifest declaration. Exact published source commit [`c0d4b63a895109abc0568b6a103c5de9f855d8f1`](https://github.com/IISweetHeartII/emux/tree/c0d4b63a895109abc0568b6a103c5de9f855d8f1/crates/emux-vt) handles ground-state raw C1 CSI/DCS/OSC/SOS/PM/APC and BEL/C1-ST. | Its private-state [`parser.rs`](https://github.com/IISweetHeartII/emux/blob/c0d4b63a895109abc0568b6a103c5de9f855d8f1/crates/emux-vt/src/parser.rs) exposes no ground/boundary query; silent bytes are ambiguous. It globally reports CAN/SUB as execute even where the oracle treats them as string-ending ignore, omits C1 “anywhere” starts outside Ground, collapses SOS/PM/APC, and consumes invalid UTF-8 lossily. Its action stream cannot support a compliant correction without local state tracking. The artifact lacks a license file, and the crates.io/repository evidence shows only two lightly adopted releases from a project created in 2026, insufficient for durable authority even absent behavior failures. |
| [`vte 0.15.0`](https://crates.io/api/v1/crates/vte) | Alacritty's established focused parser with a small pure-Rust graph. | Its exact [documentation/source](https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/lib.rs) explicitly supports only 7-bit codes. Raw C1 introducers are executions rather than string/CSI starts. **Fails behavior.** |
| [`anstyle-parse 1.0.0`](https://crates.io/api/v1/crates/anstyle-parse) | Highly adopted, actively released Rust CLI parser with a small graph. | Its exact [state generator](https://github.com/rust-cli/anstyle/blob/3048fe7820055a6c995170d822e725c62b3d63e1/crates/anstyle-parse/src/state/codegen.rs) starts required strings through ESC forms but lacks raw C1 introducer transitions and documents incomplete 8-bit behavior. Local C1 additions would be a second parser. **Fails behavior and seam gates.** |
| [`vt-push-parser 0.13.1`](https://crates.io/api/v1/crates/vt-push-parser/0.13.1) | Focused streaming parser with borrowed original-byte `VTEvent` slices and public `is_ground`; exact published source commit [`f2ff0fbe264b2019b0aa073b01a0438b50f97341`](https://github.com/mmastrac/vt-push-parser/tree/f2ff0fbe264b2019b0aa073b01a0438b50f97341/crates/vt-push-parser). The manifest declares Apache-2.0 OR MIT, no default features, no build script, and normal dependencies `hex` and `smallvec`; it declares no MSRV. | Exact [`lib.rs`](https://github.com/mmastrac/vt-push-parser/blob/f2ff0fbe264b2019b0aa073b01a0438b50f97341/crates/vt-push-parser/src/lib.rs) emits ground-state raw C1 introducers/ST as `Raw` instead of entering controls. Its held-ESC OSC/DCS/SOS-PM-APC states return ESC followed by non-`\` to string data rather than applying the oracle's any-ESC exit, and it collapses SOS/PM/APC. Repair needs local ANSI state. **Fails behavior and seam gates.** |
| [`termwiz 0.23.3`](https://crates.io/api/v1/crates/termwiz) | Established WezTerm terminal library. Its exact [`Cargo.toml`](https://github.com/wezterm/wezterm/blob/a87358516004a652ad840bc1661bdf65ffc89b43/termwiz/Cargo.toml) demonstrates the broad terminal/UI graph. | Its exact [`escape/parser/mod.rs`](https://github.com/wezterm/wezterm/blob/a87358516004a652ad840bc1661bdf65ffc89b43/termwiz/src/escape/parser/mod.rs) wraps `vtparse`, inheriting its low-level transition mismatch, and produces semantic actions rather than all exact source ranges. **Fails behavior and is over-selection.** |
| [`console 0.16.4`](https://crates.io/api/v1/crates/console/0.16.4) | Popular console utility already authorized for the blocked package. | The exact [`AnsiCodeIterator` source](https://github.com/console-rs/console/blob/598eca9fe9e3f9b93d2b49fdccf2d395d809bd94/src/ansi.rs) does not consume OSC strings or DCS bodies. **Fails behavior and losslessness.** |

## Smallest acceptable future integration architecture

This is a constraint for renewed research, not an approved implementation:

1. A dependency must accept original bytes incrementally and expose exact
   source ranges or sufficient public state/events to classify visible UTF-8,
   executed C0 controls, and zero-width ESC/C1 control bytes. The package may
   coordinate offsets but may not recognize ANSI introducers, string states,
   or terminators itself.
2. Preserve borrowed slices/ranges of the original input; never decode and
   reconstruct control sequences. Preserve incomplete control tails according
   to the oracle.
3. Match the oracle's grapheme model. The dependency must expose an
   oracle-equivalent transition into UTF-8 processing from every applicable
   state, including malformed control states, independent of semantic
   visibility. At that offset, segment the **original suffix** with
   `unicode-segmentation 1.13.3`. ANSI control bytes therefore break grapheme
   context. Do not concatenate all visible text before segmentation.
4. A still-unselected width dependency/policy must reproduce
   `displaywidth 0.11.0` cluster widths and both values of the process-wide
   `RUNEWIDTH_EASTASIAN` switch. Summing per-scalar widths or using
   `unicode-width 0.2.2` directly is not sufficient.
5. Replay exact source spans: drop printable/execute bytes after the cut while
   retaining the control/string bytes the oracle retains, including closing
   OSC-8/ST, DCS payload and termination, ordinary escapes, and trailing SGR
   reset sequences.

## Accepted limitations

None of the hard parity requirements is waived. In particular:

- a Rust `String` cannot represent raw single-byte C1 input, so any future
  dependency seam must be byte-oriented internally even if the ordinary API is
  `&str`;
- semantic interpretation or bounded parameter arrays may be ignored only if
  exact source-state boundaries continue through arbitrary payload and excess
  parameters;
- a local parser, state repair layer, or hand-written mapping from C1 to ESC
  forms is not an accepted workaround;
- combining characters, emoji ZWJ sequences, and regional indicators separated
  by ANSI must preserve the oracle's boundaries, even when concatenating
  visible text would produce a more natural Unicode cluster;
- the oracle's first-code-point cluster width, VS15/VS16 behavior, and
  `RUNEWIDTH_EASTASIAN` environment switch are observable and are not waived.

## Completed rejected probe

The preliminary probe established only toolchain, graph, license/advisory, and
platform facts; its incomplete happy-path behavior assertions are superseded by
the adversarial failures above. Its exact manifest was:

```toml
[package]
name = "ployz-ansi-decision-probe"
version = "0.0.0"
edition = "2024"
rust-version = "1.96"
publish = false

[dependencies]
vtparse = { version = "=0.7.0", default-features = false, features = ["std"] }
unicode-segmentation = { version = "=1.13.3", default-features = false }
unicode-width = { version = "=0.2.2", default-features = false, features = ["cjk"] }
```

The locked external graph was exactly `vtparse 0.7.0 -> utf8parse 0.2.2`,
`unicode-segmentation 1.13.3`, and `unicode-width 0.2.2`. The `cjk` feature was
enabled only so the rejected probe could compare both Rust width entry points;
it is not an approved configuration. With `rustc 1.96.0
(ac68faa20 2026-05-25)`, these commands passed:

```text
cargo +1.96.0 tree --manifest-path /tmp/ployz-ansi-decision-probe/Cargo.toml --locked -e normal
cargo +1.96.0 run --manifest-path /tmp/ployz-ansi-decision-probe/Cargo.toml --locked
cargo +1.96.0 clippy --manifest-path /tmp/ployz-ansi-decision-probe/Cargo.toml --locked --all-targets -- -D warnings
cargo +1.96.0 check --manifest-path /tmp/ployz-ansi-decision-probe/Cargo.toml --locked --target x86_64-pc-windows-gnu
cargo +1.96.0 check --manifest-path /tmp/ployz-ansi-decision-probe/Cargo.toml --locked --target x86_64-apple-darwin
cargo +1.96.0 check --manifest-path /tmp/ployz-ansi-decision-probe/Cargo.toml --locked --target aarch64-unknown-linux-gnu
cargo audit --file /tmp/ployz-ansi-decision-probe/Cargo.lock --db /home/codex/.cargo/advisory-db
```

The audit loaded 1,216 advisories at database commit
`69f93e1d081d8b6fbee010e48f0b5e0d13661415` and reported no vulnerability in
the five-package lockfile (including the probe root). Published-manifest and
source inspection found `build = false` for all four external crates, no
`links` declarations or FFI, no target-specific normal dependencies, and no
native/system library. `unicode-segmentation` denies unsafe and `unicode-width`
forbids it; `vtparse` uses internal unchecked table access/transmute and
`utf8parse` uses `char::from_u32_unchecked`. These facts do not overcome the
parser, width, or architecture failures.

## Required verification for renewed research

Port all applicable upstream `truncate_test.go` cases and compare exact bytes
and widths with Go `ansi.Truncate`. A candidate must additionally pass, without
panic, table-driven ESC and raw C1 fixtures for CSI, ordinary ESC, OSC, DCS,
SOS, PM, APC, BEL where applicable, ESC-ST, and C1 ST. Include:

- UTF-8 and each raw C1 introducer inside OSC, DCS, SOS, PM, and APC payloads;
- high bytes immediately before/after terminators and incomplete strings at
  EOF;
- arbitrary ESC followed by a non-`\` byte, dangling ESC at EOF, and CAN/SUB
  inside each of OSC, DCS, SOS, PM, and APC, both before and after the visible
  cut; assert their state-sensitive exact retention, not merely termination;
- invalid UTF-8 lead and continuation bytes plus truncated UTF-8 at EOF in
  Ground and every applicable CSI/string state, with byte-exact differential
  output and explicit panic-freedom;
- malformed CSI containing visible UTF-8 (for example
  `A ESC [ é B X`) and excess/invalid parameters;
- ANSI inserted between a base and combining mark, between emoji and ZWJ,
  between ZWJ and emoji, and between regional indicators;
- exact preservation of closing OSC-8/ST, DCS payload/ST, ordinary ESC, and
  trailing SGR resets after the visible cut;
- C0 execute behavior before and after the cut, empty/zero/tail-only budgets,
  wide CJK, ambiguous width, combining clusters, and ellipsis budgeting;
- valid UTF-8 C1 scalar values distinguished from raw single-byte C1 controls.
- leading Prepend and spacing-mark clusters (including `U+0600` plus ASCII),
  VS15 and VS16 clusters, and ambiguous-width characters under both values of
  `RUNEWIDTH_EASTASIAN`.

For any exact candidate graph, also run on Rust 1.96:

```text
cargo test
cargo clippy --all-targets -- -D warnings
cargo audit
cargo check --target x86_64-pc-windows-gnu
cargo check --target x86_64-apple-darwin
cargo check --target aarch64-unknown-linux-gnu
```

## Review

The first fresh adversarial review was not clean:

- `P01` found `vtparse`'s high-byte/raw-C1 string-state mismatch and panic path;
- `P02` found that segmenting a reconstructed visible stream changes oracle
  behavior when ANSI interrupts a grapheme;
- independent review findings `D01`-`D03` confirmed those failures and added
  malformed-CSI evidence; `D04` noted unbounded `std`-feature payload allocation
  in the rejected provisional `vtparse` graph;
- the first re-review found an additional `displaywidth`/`unicode-width`
  mismatch plus evidence, reproducibility, and oracle-wording gaps;
- the second fresh re-review found missing any-ESC, CAN/SUB, and malformed
  UTF-8 fixtures plus an `emux-vt` release-count error;
- the third fresh re-review required the credible borrowed-slice
  `vt-push-parser` candidate and its rejection to be recorded explicitly.

Corrections applied: the unsupported approval and exact selection were removed;
the decision is blocked; the `vtparse` behavior, panic, and resource failures
are explicitly rejected; the architecture now requires original-suffix
grapheme segmentation and oracle-equivalent UTF-8 transitions; cell width is a
separate unresolved blocker; primary-source links and the exact rejected probe
are recorded; any-ESC/CAN/SUB and invalid/truncated UTF-8 semantics are covered;
and the adversarial fixtures above are mandatory.

Fourth fresh clean re-review result: `clean`; no actionable findings remained.

## Unlock impact

No package is unblocked. `pkg/client/deploy/operation` remains
`dependency-blocked`; its downstream packages remain unchanged. The controller
must not update dependency/package/task registries from this record until a new
candidate passes every hard gate or an explicit human authority accepts a
specified parity exception.
