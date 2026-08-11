# Dependency decision: `interactive-terminal-runtime`

| Field | Value |
| --- | --- |
| Status | `blocked` — revised candidate selected, but this critical runtime must receive another fresh adversarial dependency review before approval |
| Capability | Cooked confirmation input plus the spinner's interactive terminal session: exact stdio TTY gates, raw stdin, synchronous events, stderr main-screen rendering, cursor/paste/keyboard modes, cancellation, and exact-stdout width. Rich widgets, layout, and full-screen rendering are out of scope. |
| Selected dependency | `crossterm = { version = "=0.29.0", default-features = false, features = ["bracketed-paste", "events", "windows"] }`; `terminal_size = "=0.4.4"` |
| License | `MIT` (`crossterm`); `MIT OR Apache-2.0` (`terminal_size`) |
| Research date | `2026-08-11` UTC |
| Request | Delegated capability request for `upstream/uncloud/internal/cli/tui`; no on-disk request or package packet exists at base `a47272fab9ecef37b513d0ad8a47c81c75f86dc4` |

This replaces the rejected Ratatui proposal in commit
`0487171f4e84d0ff7d489225f6f9835cac9082c0`. The selection above is a
recommendation for re-review, not an approval. The dependency gate remains
closed while this record is `blocked`.

## Oracle contract

### `Confirm` is cooked accessible input

[`prompt.go`](../../upstream/uncloud/internal/cli/tui/prompt.go) always forces
Huh 2.0.1 accessible mode, reads the default stdin, and renders the prompt to
stderr. It does not call `IsTerminalAvailable`, Bubble Tea, raw mode, or a TUI
renderer. Huh describes accessible mode as basic prompting that bypasses its
renderer, and its exact
[`Form.runAccessible`](https://github.com/charmbracelet/huh/blob/c4753045be5675ae3c814b4753687670f158d517/form.go#L720-L739)
calls each field's accessible runner and prints a terminating newline.

The exact confirmation behavior comes from Huh's
[`Confirm.RunAccessible`](https://github.com/charmbracelet/huh/blob/c4753045be5675ae3c814b4753687670f158d517/field_confirm.go#L303-L315)
and
[`PromptBool`/`PromptString`](https://github.com/charmbracelet/huh/blob/c4753045be5675ae3c814b4753687670f158d517/internal/accessibility/accessibility.go#L63-L164):

- an empty title becomes `Do you want to continue?`;
- the visible accessible suffix is `[y/N]` because the bound Go `bool` starts
  false; the configured non-accessible button labels `Yes!` and `No` do not
  change accepted input;
- `y`, `yes`, `n`, and `no` are accepted case-insensitively;
- empty or all-whitespace input selects the false default;
- a nonempty token is lowercased but not trimmed, so surrounding whitespace is
  invalid;
- invalid input prints `invalid input. please try again` and reprompts;
- EOF or a scanner error prints a newline, returns false, and is not surfaced;
  the form then prints its own field-terminating newline; and
- the accessible form discards field errors and returns `nil`, so this wrapper
  currently returns `(answer, nil)`.

The Rust confirmation path therefore needs only ordinary buffered line input
and `Write` output. It must not enter raw mode, use Crossterm events, emit
cursor/display-mode escapes, require a TTY, or construct Ratatui state.

### Spinner fallback and interactive split

[`RunSpinner`](../../upstream/uncloud/internal/cli/tui/spinner.go) checks the
exact stdin and exact stderr with the helpers in `prompt.go`.

- If either is not a terminal, it writes exactly `title + "\n"` to stderr,
  calls the action synchronously with the original context, and returns the
  action error unchanged. A pre-cancelled context does not prevent the action,
  and an action panic propagates on this path.
- Only when both are terminals does it start Huh's spinner with default stdin,
  stderr output, the caller's context, and the action running concurrently.
  Stdout is irrelevant to this gate and stays clean.

The exact Bubble Tea fork is pinned by
[`go.mod`](../../upstream/uncloud/go.mod) to immutable commit
[`c0b347143f3f43d584b010f68f44c21448fa8a86`](https://github.com/unlabs-dev/bubbletea/tree/c0b347143f3f43d584b010f68f44c21448fa8a86).
Its zero-valued view and renderer source establish that the interactive spinner:

- makes exact stdin raw and treats stderr as the terminal output;
- stays on the main screen (`AltScreen=false`);
- hides the cursor while active and restores it afterward;
- enables bracketed paste by default and disables it afterward;
- enables `modifyOtherKeys` and Kitty basic key disambiguation, but does not
  request key event-type reporting;
- enables neither mouse capture nor focus reporting; and
- owns only a one-line animated glyph/title frame, not a widget tree, layout
  engine, viewport, or full-screen buffer.

See the exact fork's
[`tty_unix.go`](https://github.com/unlabs-dev/bubbletea/blob/c0b347143f3f43d584b010f68f44c21448fa8a86/tty_unix.go),
[`View` fields](https://github.com/unlabs-dev/bubbletea/blob/c0b347143f3f43d584b010f68f44c21448fa8a86/tea.go#L128-L189),
and
[`cursed_renderer.go`](https://github.com/unlabs-dev/bubbletea/blob/c0b347143f3f43d584b010f68f44c21448fa8a86/cursed_renderer.go#L83-L187).

### Spinner event, race, cancellation, EOF, and panic semantics

These are application semantics that Crossterm exposes enough primitives to
implement; Crossterm does not decide them.

- Huh matches Bubble Tea's `KeyMsg` interface, not only `KeyPressMsg`.
  Ctrl-C therefore interrupts on a press, repeat, or release if that event is
  delivered. Basic Unix sessions normally deliver presses because event-type
  reporting is not requested; Windows console input can deliver releases. The
  Rust reducer must test all three `crossterm::event::KeyEventKind` variants and
  must not silently filter to `Press`.
- Raw-mode Ctrl-C is input, not normally a Unix signal. The interactive result
  is Bubble Tea's killed/interrupted error.
- EOF ends Bubble Tea's input reader without sending a message or stopping the
  program. Interactive EOF is ignored; action completion or external
  cancellation still governs the spinner. Ctrl-D is a key and is not an EOF
  outcome. Crossterm has no EOF `Event`, so the port must not invent one.
- A pre-cancelled interactive context is returned before the action starts,
  unlike the fallback path.
- During an interactive run the action sees the caller's external context.
  External cancellation can be seen by both action and terminal loop and is
  returned as a killed/context error if it wins.
- Ctrl-C cancels only Bubble Tea's internal program. It does not cancel the
  context passed to the action. An action that ignores its context can continue
  after `RunSpinner` returns; Bubble Tea deliberately does not join long command
  goroutines during shutdown.
- Action completion/error, cancellation, and interrupt are concurrent. If the
  action result is accepted first, Huh returns that result (including the
  original action error); otherwise the cancellation/interrupt result wins.
  There is no deterministic priority promise for a simultaneous race.
- No renderer/event/application lock is held while user action code runs.
- An interactive action panic is caught in Bubble Tea's command goroutine,
  diagnosed with a CRLF-normalized panic and stack on the process's actual
  stderr, followed by cleanup and a killed/panic error. The non-TTY path does
  not catch the panic. Rust must at minimum unwind through terminal cleanup;
  exact catch/diagnose/error conversion remains a parity requirement.

The source path is Huh's exact
[`spinner.go`](https://github.com/charmbracelet/huh/blob/c4753045be5675ae3c814b4753687670f158d517/spinner/spinner.go#L179-L258)
plus Bubble Tea's
[`command execution`](https://github.com/unlabs-dev/bubbletea/blob/c0b347143f3f43d584b010f68f44c21448fa8a86/tea.go#L698-L958)
and
[`panic recovery`](https://github.com/unlabs-dev/bubbletea/blob/c0b347143f3f43d584b010f68f44c21448fa8a86/tea.go#L1267-L1314).

### `TerminalWidth` is the exact stdout descriptor

`TerminalWidth` calls `term.GetSize(os.Stdout.Fd())` and returns `0` on error or
nonpositive width. It does not ask for a controlling terminal and does not fall
back to stderr or stdin.

Use only
[`terminal_size::terminal_size_of(&std::io::stdout())`](https://docs.rs/terminal_size/0.4.4/terminal_size/fn.terminal_size_of.html)
and map `None` or zero width to `0`. The exact 0.4.4 Unix source applies
`isatty` and `tcgetwinsize` to the supplied descriptor; its Windows source calls
`GetConsoleScreenBufferInfo` on the supplied handle. Do not call
`terminal_size::terminal_size()`, which tries stdout, then stderr, then stdin.
Do not call `crossterm::terminal::size()`, whose Unix implementation opens
`/dev/tty`, falls back to stdout only when that fails, and may invoke `tput`.

## Primary-source dependency evidence

- Crossterm 0.29.0 provides
  [`enable_raw_mode`/`disable_raw_mode`](https://docs.rs/crossterm/0.29.0/crossterm/terminal/),
  cursor hide/show, bracketed-paste commands, keyboard enhancement push/pop,
  typed key/resize/paste events, and synchronous
  [`poll`/`read`](https://docs.rs/crossterm/0.29.0/crossterm/event/).
  `poll(timeout)` makes the subsequent `read()` nonblocking; the two calls must
  stay on one thread and must not be mixed with `EventStream`.
- Crossterm commands accept a supplied `Write`, so Unix ANSI commands can be
  directed to stderr. Raw mode remains application-global and Crossterm does
  not supply the application-level session guard required below.
- Crossterm's exact
  [`Cargo.toml`](https://docs.rs/crate/crossterm/0.29.0/source/Cargo.toml.orig)
  declares MIT, Rust 1.63, and defaults `bracketed-paste`, `events`, `windows`,
  and `derive-more`. Direct selection keeps the first three and omits the
  convenience proc-macro feature.
- `terminal_size` 0.4.4's exact
  [`Cargo.toml`](https://docs.rs/crate/terminal_size/0.4.4/source/Cargo.toml.orig)
  declares `MIT OR Apache-2.0`, Rust 1.71, no feature flags, Rustix on Unix,
  and `windows-sys` on Windows.
- Official crates.io metadata captured on 2026-08-11 reported Crossterm at
  171,694,045 total / 40,990,622 recent downloads and 7,068 reverse-dependent
  crates; `terminal_size` at 187,221,455 / 34,751,154 and 667 dependents; and
  Ratatui at 43,071,523 / 15,857,467 and 5,294 dependents. Adoption therefore
  strongly supports the direct low-level candidates; rejecting Ratatui is based
  on required behavior and weight, not lack of popularity.

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Cooked confirmation; exact stdio gates; raw spinner input; stderr main-screen output; cursor, paste, basic keyboard modes; typed events; bounded cancellation polling; exact stdout width | Standard I/O covers confirmation. Crossterm covers every spinner primitive. `terminal_size_of(stdout)` covers exact-FD width. Rust and PTY probes compiled and exercised these APIs. Ratatui is unnecessary. | `pass` for shipped Linux/macOS behavior, subject to re-review |
| License and security | Permissive direct/transitive licenses; no known advisory in the exact resolution | Direct licenses are MIT and MIT/Apache-2.0. The focused lock's declared licenses were MIT, Apache-2.0, accepted combinations, or Apache-2.0 WITH LLVM-exception. `cargo audit` scanned 29 packages against 1,211 RustSec advisories and exited clean. | `pass`, subject to re-review and integration re-audit |
| Platforms and targets | Shipped CLI targets Linux/macOS amd64/arm64; honest Windows statement | Rust 1.96 native Linux check and macOS cross-check passed. Windows GNU cross-compiles with the `windows` feature, but runtime stderr/VT/ConPTY and exact raw restore are not established. The oracle release matrix comments Windows out. | `pass` only for shipped Linux/macOS runtime; Windows runtime explicitly out of scope |
| Maintenance and Rust version | Maintained, established, Rust <= 1.96 | Exact MSRVs are 1.63 and 1.71. Crossterm 0.29.0 and `terminal_size` 0.4.4 are their current non-yanked releases in official crates.io metadata; both have high current download volume. Rust 1.96 checks passed. | `pass` |
| Architectural constraints | No rich renderer; exact streams; testable state machine; cleanup after every exit; no async runtime or proc-macro convenience graph | Direct Crossterm commands/events plus a pure one-line reducer fit. `terminal_size_of` preserves the exact descriptor. Defaults off excludes Ratatui, `derive-more`, futures, Serde, clipboard, and `/dev/tty` feature additions. | `pass`, provided the mandatory guard below is implemented and reviewed |

## Candidate comparison

| Candidate | Hard-gate fit | Weight and ergonomics | Decision |
| --- | --- | --- | --- |
| `crossterm` 0.29.0 + `terminal_size` 0.4.4 | Covers every required low-level spinner primitive, typed event kinds, bounded sync polling, and exact-FD width. Strong adoption, explicit MSRVs, permissive licenses. | Two direct crates. On Linux the focused normal graph has no async runtime, futures, Serde, Ratatui, or `derive_more`; Crossterm's unconditional `document-features` is the sole proc macro. | **Selected for fresh re-review.** |
| Ratatui 0.30.2 + Crossterm | Can render terminal-cell buffers, widgets, viewports, and diffs, but still delegates input, session policy, action races, and cleanup. No oracle path requires its abstraction: confirmation is cooked and the spinner is one main-screen line. | Adds a second renderer, buffer/layout/widget surface, re-export layer, and transitives. Its test backend does not remove the need for a pure reducer and injected writer/event tests. | **Rejected as over-selected; no required behavior proves it necessary.** |
| Termion 4.0.6 | Supplies Unix raw mode, events, escapes, and FD size, but its documented platform scope is Redox/macOS/Linux or ANSI terminals and it lacks Crossterm's portable typed event-kind model. | Lighter on some Unix builds, but substantially less adopted and loses the Windows compile path without improving shipped-target parity. | Rejected: narrower and less idiomatic for no behavior benefit. |
| Termwiz / Termina | Broad terminal models and parsers can cover the primitives. | Considerably broader surface or much lower adoption; neither fixes application-owned race and cleanup semantics. | Rejected: no parity advantage over direct Crossterm. |
| Bespoke termios/WinAPI/ANSI runtime | Could control every byte and descriptor. | Duplicates platform-sensitive raw/event/parser code, introduces direct unsafe/FFI responsibility, and has no ecosystem maintenance. | Rejected while a popular passing pair exists. |

## Selected integration

### Exact versions and features

Only the integrator may add these workspace dependencies and lockfile entries:

```toml
crossterm = { version = "=0.29.0", default-features = false, features = ["bracketed-paste", "events", "windows"] }
terminal_size = "=0.4.4"
```

`events` supplies synchronous input and typed key/resize events;
`bracketed-paste` supplies the oracle's paired mode and paste event;
`windows` is required because Crossterm deliberately fails Windows compilation
without it. Target-specific Windows dependencies do not enter Unix builds, and
this feature is not evidence of Windows runtime parity.

Do not enable `derive-more`, `event-stream`, `serde`, `osc52`, `use-dev-tty`,
`libc`, or defaults. In particular, do not add an async runtime, futures stream,
Ratatui, or a second terminal backend. Match enums directly instead of enabling
`derive-more`. The focused all-target lock contained 29 packages including the
temporary probe itself. The Linux normal graph's only proc macro was
Crossterm's unconditional
`document-features` (plus its small `litrs` parser).

### Natural API and parity constraints

- Implement confirmation with `BufRead::read_line`/equivalent cooked input and
  an injected `Write`; keep parsing in a pure function. Preserve case folding,
  blank default, untrimmed nonblank validation, reprompt text, EOF false, and
  newline behavior. Do not route it through the terminal session.
- Gate interactive spinner setup with exact
  `std::io::stdin().is_terminal() && std::io::stderr().is_terminal()` before any
  Crossterm call. Never permit Crossterm's controlling-TTY fallback to override
  this decision.
- Render only the managed spinner line to stderr on the main screen. A pure
  reducer should accept tick, action-result, cancellation, and typed key events;
  a small generic-`Write` renderer should emit/clear that line. Do not enter the
  alternate screen or enable mouse/focus modes.
- Push only `KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`; do not request
  `REPORT_EVENT_TYPES`, alternate keys, or all-keys-as-escape-codes. Still handle
  Ctrl-C across `Press`, `Repeat`, and `Release` when delivered.
- Use synchronous `event::poll` with a bounded 25–100 ms timeout followed by
  `event::read` on the same thread. Each timeout rechecks action completion and
  cancellation. Do not mix this reader with `EventStream`.
- Run the action outside every application mutex and outside any borrowed
  stderr lock. Because raw mode is process-global, reject a concurrent second
  interactive session with an atomic/nonblocking active-session lease; do not
  hold a blocking `MutexGuard` across user action execution.
- Preserve the first observed action/cancellation/interrupt winner rather than
  inventing deterministic priority. Preserve the original action error when it
  wins. Document and test that Ctrl-C may return while an action that ignores
  its context continues.
- Call only `terminal_size_of(&stdout)` for width and return zero for `None` or
  zero. Never call either fallback size API.

### Mandatory executable RAII state machine

Crossterm exposes paired operations, not an application session guard. The
package implementation must encode and test this state machine:

1. Acquire the nonblocking active-session lease and perform the exact stdin /
   stderr TTY preflight.
2. Enable raw mode; record `raw_enabled` only after success.
3. For each stderr transition, conservatively mark the mode as “may be active”
   **before** its independent write/flush, because a writer can accept bytes and
   then report failure: hide cursor, enable bracketed paste, push basic keyboard
   enhancement.
4. Start the action with no application lock or writer lock held. Drive ticks,
   cancellation, action result, and input through bounded polling.
5. On setup error, render/event error, action result/error, cancellation,
   interrupt, or panic, attempt every applicable cleanup independently in
   reverse order: pop keyboard enhancement, disable bracketed paste, clear the
   managed line, show cursor, flush stderr, and disable raw mode. A cleanup
   failure must never short-circuit later attempts.
6. Preserve the primary setup/action/cancellation/interrupt/panic outcome.
   Retain cleanup failures as secondary diagnostics; only when no primary error
   exists may cleanup failure become the returned error.
7. Provide an explicit fallible `finish()` for reportable normal cleanup and a
   non-panicking `Drop` backstop that retries all still-marked transitions after
   partial setup and during unwind. Make cleanup idempotent.
8. For exact interactive panic parity, place `catch_unwind` at the action-thread
   boundary, carry the panic as the primary outcome, complete cleanup, emit the
   required diagnostic, and convert it to the package's spinner panic error.
   Let the non-interactive action unwind normally. The fresh parity/Rust reviews
   must approve the exact mapping.

Setup and cleanup must not use one multi-command `execute!` whose first error
skips later work. Inject a terminal-operations backend in tests so every setup
step and every cleanup step can fail independently; assert rollback flags, all
cleanup attempts, primary-error retention, and idempotent `Drop`.

## Windows scope and known limitations

The shipped `uc` matrix in
[`.goreleaser.yaml`](../../upstream/uncloud/.goreleaser.yaml) is Linux/macOS
amd64/arm64; Windows is commented out. This decision recommends runtime support
only for those shipped Linux/macOS targets. The `windows` feature maintains a
compile path; Windows runtime behavior is explicitly **not approved** here.

Crossterm 0.29.0 source prevents a stronger claim:

- its bundled stderr example says it is only suited to Unix;
- Windows ANSI capability detection and legacy command fallbacks inspect/use
  the current stdout console handle, not necessarily the supplied stderr;
- Windows `disable_raw_mode` ORs line/echo/processed flags back rather than
  restoring the exact previous console-mode bitmask;
- progressive keyboard enhancement push/pop is unsupported in the legacy
  WinAPI path, and bracketed paste is unsupported there; and
- cross-compilation does not exercise Windows Console Host, Windows Terminal,
  stderr VT enablement, or ConPTY pipe routing.

If Windows becomes a release target, return to the dependency gate for a
Windows-specific terminal primitive/guard decision and real Console plus ConPTY
tests. Do not advertise the cross-check below as Windows runtime parity.

Other limitations:

- Raw mode and display modes are process-global. The required lease/guard must
  prevent overlapping sessions and clean up best-effort even during unwind.
- Crossterm's synchronous reader is global internally. One reader thread and
  serialized live-terminal tests are required.
- `event::poll` can report a ready source whose subsequent parse/read fails;
  treat that as a primary event error and run full cleanup. EOF is not such an
  error in the oracle.
- A clean focused audit is point-in-time evidence. Re-audit the integrated
  lockfile and review any future transitive/version change.

## Verification performed

Exact Go source and executable characterization used the frozen Huh 2.0.1 and
Bubble Tea fork, plus Rust/Cargo 1.96 temporary probes outside the repository.

```sh
env GOCACHE=/tmp/ployz-go126-oracle-cache GOTOOLCHAIN=local \
  GOPROXY=off GOSUMDB=off /opt/go1.26.1/bin/go test ./internal/cli/tui

cargo test --locked
cargo check --locked --target x86_64-unknown-linux-gnu
cargo check --locked --target x86_64-apple-darwin
cargo check --locked --target x86_64-pc-windows-gnu
cargo tree --locked --target x86_64-unknown-linux-gnu -e normal
cargo audit --db /tmp/ployz-rustsec-advisory-db --no-fetch
```

Results:

- the frozen Go package passed;
- confirmation probes covered `y`, `YES`, `n`, `No`, blank, invalid then valid,
  EOF, and surrounding whitespace; a PTY run showed cooked echo and no terminal
  mode sequences;
- spinner probes covered action success/error, external cancellation, Ctrl-C,
  ignored EOF, panic diagnostic/error, and action-return races; source tracing
  confirmed the event-kind and leaked-action behavior;
- three Rust unit tests passed for command bytes, all Ctrl-C event kinds, and
  exact-FD `None` on a regular file;
- PTY sessions emitted exactly cursor hide, paste enable, basic keyboard push,
  keyboard pop, paste disable, and cursor show, with no alternate-screen,
  mouse, or focus sequence;
- PTY termios snapshots matched after normal cleanup and after a Rust panic
  unwind (`rc=101`);
- an 80x24 PTY returned `Some(80x24)` for exact stdout, while redirecting stdout
  alone returned `None` even though the fallback API found stderr's 80x24 PTY;
- Rust 1.96 checks passed for Linux, macOS, and Windows GNU; this is compile-only
  evidence for Windows; and
- the focused 29-package lock scanned clean against 1,211 RustSec advisories.

Package acceptance must additionally run injected setup/cleanup failure tests,
serialized live-PTY tests, and the exact confirmation/spinner race matrix in the
owned crate. Dependency probes cannot substitute for those package tests.

## Review

This capability is a critical runtime, so
[`migration/dependencies/README.md`](README.md) requires a second fresh
adversarial dependency researcher. The earlier adversarial review rejected the
Ratatui selection and oracle claims; this revision addresses those findings but
has not been reviewed by another fresh agent.

**Required next action:** dispatch a fresh adversarial dependency reviewer who
independently rechecks:

1. cooked confirmation versus raw spinner boundaries, including confirmation
   EOF/newlines and untrimmed nonblank input;
2. spinner main-screen modes, Ctrl-C across all event kinds, ignored EOF,
   path-dependent pre-cancel/panic behavior, action leakage, and race winner;
3. the exact dependency versions/features and complete absence of Ratatui,
   async event streams, and optional proc-macro convenience features;
4. exact-stdout width with no controlling-TTY or stderr/stdin fallback;
5. the executable RAII contract: conservative partial-setup flags, all cleanup
   attempts, primary-error preservation, unwind cleanup, no lock during user
   action, session serialization, and bounded single-reader polling;
6. license, Rust 1.96, feature tree, RustSec, Linux/macOS PTY evidence; and
7. the explicit exclusion of Windows runtime parity, or else new real Windows
   Console/ConPTY evidence and a separate approved primitive.

Until that reviewer records a clean result in this section, status remains
`blocked`; the controller must not update `migration/DEPENDENCIES.tsv`, unblock a
package, or treat this selection as approved.

Affected package: Go `upstream/uncloud/internal/cli/tui`; future migration crate
`crates/ployz-internal-cli-tui`. No package packet exists at this base.
