# Dependency decision: `interactive-terminal-runtime`

| Field | Value |
| --- | --- |
| Status | `blocked` — the required second adversarial review rejected the provisional pair after reproducing two hard behavior-gate failures |
| Capability | Cooked confirmation input plus the spinner's interactive terminal session: exact stdio TTY gates, raw stdin, synchronous events, stderr main-screen rendering, cursor/paste/keyboard modes, cancellation, and exact-stdout width. Rich widgets, layout, and full-screen rendering are out of scope. |
| Selected dependency | None. The provisional `crossterm` 0.29.0 plus `terminal_size` 0.4.4 pair is rejected with its exact proposed features below. |
| License | `MIT` (`crossterm`); `MIT OR Apache-2.0` (`terminal_size`) |
| Research date | `2026-08-11` UTC |
| Request | Delegated capability request for `upstream/uncloud/internal/cli/tui`; no on-disk request or package packet exists at base `e4c100daf293403270d7e3696eb187aff440ebb4` |

The direct pair replaced the rejected Ratatui proposal in commit
`0487171f4e84d0ff7d489225f6f9835cac9082c0`, but it also fails the gate. The
dependency gate remains closed. No root manifest, lockfile, registry, package
packet, or migration crate may consume the rejected entries.

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

Executable probes add three constraints that an ordinary unbounded
`BufRead::read_line` adapter would miss:

- the exact stderr prompt is SGR bold yellow followed by reset and one space,
  for example `\x1b[1;33mProceed? [y/N]\x1b[m `; success adds the form's one
  newline, while EOF/read error adds the scanner newline and then the form
  newline;
- Go's default `bufio.Scanner` token limit is observable: a 65,537-byte token
  without a newline is treated as a scanner error and therefore returns the
  false default with two newlines; and
- every prompt/error/newline write error is ignored. A valid `y` read through
  an always-failing writer still produces `(true, nil)`; a read error preserves
  the bound default and is also returned as `nil`.

The Rust confirmation path needs a capped, scanner-compatible cooked reader and
an error-suppressing `Write` adapter. It must not enter raw mode, use Crossterm
events, emit cursor/display-mode escapes, require a TTY, or construct Ratatui
state. Exact SGR and newline bytes require fixture tests.

### Spinner fallback and interactive split

[`RunSpinner`](../../upstream/uncloud/internal/cli/tui/spinner.go) checks the
exact stdin and exact stderr with the helpers in `prompt.go`.

- If either is not a terminal, it writes exactly `title + "\n"` to stderr,
  calls the action synchronously with the original context, and returns the
  action error unchanged. The title write error is ignored. A pre-cancelled
  context does not prevent the action, and an action panic propagates on this
  path.
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

The keyboard transitions are not Crossterm's progressive-keyboard stack
commands. The fork emits xterm modifyOtherKeys level 2 (`\x1b[>4;2m`) and sets
Kitty basic disambiguation (`\x1b[=1;1u`), queries the Kitty flags
(`\x1b[?u`), then resets modifyOtherKeys (`\x1b[>4m`) and Kitty flags
(`\x1b[=0;1u`). Crossterm's `PushKeyboardEnhancementFlags`/`Pop...` instead
emit `\x1b[>1u`/`\x1b[<1u`; using them would be an observable mismatch.

A Linux `TERM=xterm` PTY capture also contained Bubble Tea's environment- and
termios-dependent synchronized-output/unicode-mode queries, background-color
query, tab-stop setup, main-screen clears, and cursor movement. These are
package-level byte behavior, not evidence that a rich renderer is needed, but
the eventual adapter must characterize and deliberately preserve them rather
than claiming the six core cursor/paste/keyboard sequences are the complete
oracle output.

See the exact fork's
[`tty_unix.go`](https://github.com/unlabs-dev/bubbletea/blob/c0b347143f3f43d584b010f68f44c21448fa8a86/tty_unix.go),
[`View` fields](https://github.com/unlabs-dev/bubbletea/blob/c0b347143f3f43d584b010f68f44c21448fa8a86/tea.go#L128-L189),
and
[`cursed_renderer.go`](https://github.com/unlabs-dev/bubbletea/blob/c0b347143f3f43d584b010f68f44c21448fa8a86/cursed_renderer.go#L83-L187).

### Spinner event, race, cancellation, EOF, and panic semantics

These are application/reducer semantics that a terminal adapter must implement;
Crossterm does not decide them. Its display/event types cover several pieces,
but the selected event source cannot satisfy the EOF/cancellation contract.

- Huh matches Bubble Tea's `KeyMsg` interface, not only `KeyPressMsg`.
  Ctrl-C therefore interrupts on a press, repeat, or release if that event is
  delivered. Basic Unix sessions normally deliver presses because event-type
  reporting is not requested; Windows console input can deliver releases. The
  Rust reducer must test all three `crossterm::event::KeyEventKind` variants and
  must not silently filter to `Press`. Huh compares the exact key string
  `ctrl+c`; extra Shift, Alt, Meta, Hyper, or Super modifiers must not
  interrupt. A Rust `.contains(CONTROL)` predicate is too broad.
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
- An interactive action panic is caught in Bubble Tea's batch-command
  goroutine. That path diagnoses the CRLF-normalized panic and stack on the
  process's actual stderr while the terminal can still be raw/cursor-hidden;
  the main program loop then performs shutdown and returns a killed/panic
  error. The non-TTY path does not catch the panic. Cleanup-before-diagnostic is
  safer but is not the oracle's observable order; any intentional parity
  exception requires controller/human authority.
- Input and resize-listener failures are primary killed errors. EOF is a clean
  end of the reader and sends no error or event. Periodic/final renderer flush,
  renderer close, and terminal-restore errors are ignored by the fork. The
  action/cancellation/interrupt result therefore still governs an output or
  cleanup failure.

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

The provisional
[`terminal_size::terminal_size_of(&std::io::stdout())`](https://docs.rs/terminal_size/0.4.4/terminal_size/fn.terminal_size_of.html)
does target the exact descriptor, but it is not behaviorally sufficient. Its
Unix implementation returns `None` unless **both** rows and columns are
positive. A PTY set to `(rows=0, columns=80)` produced `None` in Rust, while
Go's `term.GetSize(stdout)` produced `(80, 0, nil)` and `TerminalWidth`
returned `80`, because the oracle validates only width. This is a hard blocker.

The successor must query the exact stdout descriptor/handle and validate only
the returned width. It must not use `terminal_size::terminal_size()`, which
tries stdout, then stderr, then stdin, or `crossterm::terminal::size()`, whose
Unix implementation opens `/dev/tty`, falls back to stdout only when that fails,
and may invoke `tput`.

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
- With the proposed features, Crossterm's Unix reader selects its Mio source.
  In the exact 0.29.0
  [`mio.rs`](https://github.com/crossterm-rs/crossterm/blob/36d95b26a26e64b0f8c12edfe11f410a6d56a812/src/event/source/unix/mio.rs#L67-L152),
  the inner TTY loop advances only for positive reads, breaks only for
  `WouldBlock`, and retries `Interrupted`; zero-byte EOF and errors such as PTY
  `EIO` neither return nor break. A native PTY hangup reproduced an indefinite
  97–98% CPU loop inside `poll(200ms)` until SIGKILL. Action completion,
  cancellation, and cleanup cannot regain control. This is a hard blocker.
- Crossterm's exact
  [`Cargo.toml`](https://docs.rs/crate/crossterm/0.29.0/source/Cargo.toml.orig)
  declares MIT, Rust 1.63, and defaults `bracketed-paste`, `events`, `windows`,
  and `derive-more`. Direct selection keeps the first three and omits the
  convenience proc-macro feature.
- `terminal_size` 0.4.4's exact
  [`Cargo.toml`](https://docs.rs/crate/terminal_size/0.4.4/source/Cargo.toml.orig)
  declares `MIT OR Apache-2.0`, Rust 1.71, no feature flags, Rustix on Unix,
  and `windows-sys` on Windows. Its exact
  [`unix.rs`](https://github.com/eminence/terminal-size/blob/cad29f6450c6873fe2f719b93d17bba14c15737e/src/unix.rs#L21-L40)
  rejects a valid positive width when rows are zero, unlike the oracle.
- Official crates.io metadata captured on 2026-08-11 reported Crossterm at
  171,694,045 total / 40,990,622 recent downloads and 7,068 reverse-dependent
  crates; `terminal_size` at 187,221,455 / 34,751,154 and 667 dependents; and
  Ratatui at 43,071,523 / 15,857,467 and 5,294 dependents. Adoption therefore
  strongly supports the direct low-level candidates; rejecting Ratatui is based
  on required behavior and weight, not lack of popularity.

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Cooked confirmation; exact stdio gates; raw spinner input; stderr main-screen output; cursor, paste, basic keyboard modes; typed events; bounded cancellation polling; exact stdout width | The proposed Mio event source can spin forever on EOF/EIO, and `terminal_size_of(stdout)` rejects positive width when rows are zero. The record also prescribed non-oracle keyboard bytes, Ctrl-C matching, scanner limits, and failure precedence. | **`fail`** |
| License and security | Permissive direct/transitive licenses; no known advisory in the exact resolution | Direct licenses are MIT and MIT/Apache-2.0. The focused lock's declared licenses were MIT, Apache-2.0, accepted combinations, or Apache-2.0 WITH LLVM-exception. `cargo audit` scanned 29 packages against 1,211 RustSec advisories and exited clean. | `pass` for the rejected graph; successor must be re-audited |
| Platforms and targets | Shipped CLI targets Linux/macOS amd64/arm64; honest Windows statement | Rust 1.96 native Linux and macOS cross-compilation passed. No native macOS PTY probe or recorded verification exception exists. Windows GNU cross-compiles, but runtime stderr/VT/Console/ConPTY and exact raw restore are not established. The oracle release matrix comments Windows out. | **`fail`** pending native macOS evidence/exception; Windows runtime explicitly unapproved |
| Maintenance and Rust version | Maintained, established, Rust <= 1.96 | Exact MSRVs are 1.63 and 1.71. Crossterm 0.29.0 and `terminal_size` 0.4.4 are their current non-yanked releases in official crates.io metadata; both have high current download volume. Rust 1.96 checks passed. | `pass` |
| Architectural constraints | No rich renderer; exact streams; testable state machine; cleanup after every exit; no async runtime or proc-macro convenience graph | A pure reducer/direct writer remains appropriate, but the chosen input backend can permanently trap the driver thread and the width API cannot expose required data. | **`fail`** for the provisional pair |

## Candidate comparison

| Candidate | Hard-gate fit | Weight and ergonomics | Decision |
| --- | --- | --- | --- |
| `crossterm` 0.29.0 + `terminal_size` 0.4.4 with the proposed features | Covers many low-level display primitives and typed events, but the selected Mio reader spins on EOF/EIO and the width API rejects `(rows=0, columns>0)`. Its keyboard push/pop commands also do not match the oracle bytes. | Two direct crates. On Linux the focused normal graph has no async runtime, futures, Serde, Ratatui, or `derive_more`; Crossterm's unconditional `document-features` is the sole proc macro. | **Rejected by hard behavior gates.** |
| Ratatui 0.30.2 + Crossterm | Can render terminal-cell buffers, widgets, viewports, and diffs, but still delegates input, session policy, action races, and cleanup. No oracle path requires its abstraction: confirmation is cooked and the spinner is one main-screen line. | Adds a second renderer, buffer/layout/widget surface, re-export layer, and transitives. Its test backend does not remove the need for a pure reducer and injected writer/event tests. | **Rejected as over-selected; no required behavior proves it necessary.** |
| Termion 4.0.6 | Supplies Unix raw mode, events, escapes, and FD size, but its documented platform scope is Redox/macOS/Linux or ANSI terminals and it lacks Crossterm's portable typed event-kind model. | Lighter on some Unix builds, but substantially less adopted. | Not approved; a successor review would need exact EOF/EIO, zero-row width, macOS, and event-semantics probes. |
| Termwiz / Termina | Broad terminal models and parsers can cover the primitives. | Considerably broader surface or much lower adoption; neither fixes application-owned race and cleanup semantics. | Not approved; reconsider only with complete hard-gate evidence. |
| Bespoke termios/WinAPI/ANSI runtime | Could control every byte and descriptor. | Duplicates platform-sensitive raw/event/parser code, introduces direct unsafe/FFI responsibility, and has no ecosystem maintenance. | Last resort only if maintained candidates cannot pass. |

## Rejected provisional integration

### Exact versions and features

These are the exact rejected entries. The integrator must not add them:

```toml
crossterm = { version = "=0.29.0", default-features = false, features = ["bracketed-paste", "events", "windows"] }
terminal_size = "=0.4.4"
```

`events` selects the failing Mio Unix reader; `bracketed-paste` supplies paste
events; and `windows` maintains only a compile path. `use-dev-tty` was tested as
a possible Crossterm successor: it avoided the permanent hang after an initial
poll but could spin until the timeout on hangup, changes the graph and
`/dev/tty` behavior, and has not received the required security/license/macOS
review. It is not an approved drop-in correction.

The rejected focused all-target lock contained 29 packages including the
temporary probe itself. It did not contain Ratatui, an async runtime, futures,
Serde, `derive_more`, or clipboard support. The Linux normal graph's only proc
macro was Crossterm's unconditional `document-features` plus its small `litrs`
parser. These weight advantages do not override failed behavior gates.

### Natural API and parity constraints

- Implement confirmation with a cooked scanner-compatible adapter capped at
  Go's default 64 KiB token limit and an injected error-suppressing `Write`;
  keep parsing in a pure function. Preserve exact SGR bytes, case folding,
  blank default, untrimmed nonblank validation, reprompt text, EOF/read-error
  default, suppressed write errors, and newline behavior. Do not route it
  through the terminal session.
- Gate interactive spinner setup with exact
  `std::io::stdin().is_terminal() && std::io::stderr().is_terminal()` before any
  Crossterm call. Never permit Crossterm's controlling-TTY fallback to override
  this decision.
- Render only the managed spinner line to stderr on the main screen. A pure
  reducer should accept tick, action-result, cancellation, and typed key events;
  a small generic-`Write` renderer should emit/clear that line. Do not enter the
  alternate screen or enable mouse/focus modes.
- Do not use Crossterm's keyboard push/pop commands. An explicit byte adapter
  must set/reset xterm modifyOtherKeys and Kitty basic disambiguation with the
  oracle's exact sequences and queries. Do not request event-type reporting,
  alternate keys, or all-keys-as-escape-codes. Match exactly Control+C with no
  extra modifiers across `Press`, `Repeat`, and `Release` when delivered.
- Use an input primitive proven by native PTY hangup tests to return control on
  EOF/EIO within its bound. Each 25–100 ms timeout must recheck action
  completion and cancellation. A reader that traps its thread or leaks a
  permanent busy-loop fails even if the main result can detach from it.
- Run the action outside every application mutex and outside any borrowed
  stderr lock; do not hold a blocking `MutexGuard` across user action
  execution. Because raw mode is process-global, a nonblocking active-session
  lease would be the safe Rust design, but the Go oracle has no concurrent-run
  rejection. That observable safety deviation needs explicit controller/human
  authorization after an overlap characterization test; it is not silently
  approved by this record.
- Preserve the first observed action/cancellation/interrupt winner rather than
  inventing deterministic priority. Preserve the original action error when it
  wins. Document and test that Ctrl-C may return while an action that ignores
  its context continues.
- Query only the exact stdout descriptor/handle and accept positive width even
  when height is zero. Never call a fallback size API.

### Mandatory executable RAII state machine

Crossterm exposes paired operations, not an application session guard. Any
successor must encode and test this state machine independently of the rejected
pair:

1. Acquire the nonblocking active-session lease and perform the exact stdin /
   stderr TTY preflight.
2. Enable raw mode; record `raw_enabled` only after success.
3. For each stderr transition, conservatively mark the mode as “may be active”
   **before** its independent write/flush, because a writer can accept bytes and
   then report failure: hide cursor, enable bracketed paste, set
   modifyOtherKeys, and set Kitty basic disambiguation.
4. Start the action with no application lock or writer lock held. Drive ticks,
   cancellation, action result, and input through bounded polling.
5. On terminal/raw/input setup error, listener error, action result/error,
   cancellation, interrupt, or panic, attempt every applicable cleanup
   independently. Preserve Bubble Tea's multi-stage ordering in byte fixtures:
   reset modifyOtherKeys and Kitty flags; move to the bottom and flush that
   movement; erase below, show cursor, and disable bracketed paste; flush the
   remaining buffered display transitions; then restore input raw mode. A
   cleanup failure must never short-circuit later attempts.
6. Model error precedence explicitly. Terminal/raw/input setup and
   input/resize-listener failures are primary. Initial/periodic/final stderr
   transition or render writes, renderer close, flush, and restore failures are
   suppressed by the oracle; they do not trigger an early exit or replace
   success/another winner. Returning a cleanup error when no other error exists
   is not parity.
7. Provide an explicit internal `finish()` that attempts every cleanup and can
   collect failures for injected-test assertions, while the public spinner
   result suppresses that cleanup report. Add a non-panicking `Drop` backstop
   that retries all still-marked transitions after partial setup and during
   unwind. Make cleanup idempotent.
8. For exact interactive panic parity, place `catch_unwind` at the action-thread
   boundary, carry the panic as the primary outcome, emit the required
   CRLF-normalized diagnostic and stack to actual process stderr, then complete
   cleanup and convert it to the package's spinner panic error. Let the
   non-interactive action unwind normally. If cleanup-before-diagnostic is
   selected for safety, record an explicit parity exception before approval.

Setup and cleanup must not use one multi-command `execute!` whose first error
skips later work. Inject a terminal-operations backend in tests so every setup
step and every cleanup step can fail independently; assert rollback flags, all
cleanup attempts, oracle error suppression/precedence, and idempotent `Drop`.

## Windows scope and known limitations

The shipped `uc` matrix in
[`.goreleaser.yaml`](../../upstream/uncloud/.goreleaser.yaml) is Linux/macOS
amd64/arm64; Windows is commented out. Any successor decision needs runtime
support for those shipped Linux/macOS targets. The rejected `windows` feature
maintains only a compile path; Windows runtime behavior is explicitly **not
approved** here.

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
  clean up best-effort even during unwind; rejecting overlap remains a pending
  safety/parity decision as described above.
- Crossterm's synchronous reader is a non-reset singleton. Any Crossterm
  successor requires one reader thread plus process-isolated, serialized
  live-terminal tests; `/dev/tty` fallback and stable-descriptor ownership must
  be explicit.
- The rejected Mio reader permanently spins after EOF/EIO. A successor must
  distinguish clean EOF (ignored by the oracle) from listener errors (primary)
  without trapping or leaking its driver thread.
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
  EOF, surrounding whitespace, an injected reader error, an always-failing
  writer, and a 65,537-byte over-limit token. Exact SGR/reset and one-versus-two
  newline behavior was captured;
- spinner probes covered action success/error, external cancellation, Ctrl-C,
  ignored EOF, pre-cancel on interactive and fallback paths, panic
  diagnostic/error, listener/output failures, and action-return races. One
  immediate action/Ctrl-C run produced both winners across 100 iterations; a
  Ctrl-C run returned while its context-ignoring action remained alive;
- the Go renderer ignored an injected output failure but surfaced an injected
  listener failure as a killed error, matching source error routing;
- five Rust unit tests passed for explicit oracle keyboard bytes, proof that
  Crossterm push differs, all Ctrl-C event kinds, exact-FD `None` on a regular
  file, and non-short-circuiting cleanup attempts;
- the Go PTY capture showed no alternate-screen, mouse, or focus sequence, but
  did show modifyOtherKeys plus Kitty set/query/reset and additional
  environment-dependent terminal queries; it was not limited to Crossterm's
  push/pop bytes;
- PTY termios snapshots matched after normal cleanup and after a Rust panic
  unwind (`rc=101`);
- an 80x24 PTY returned `Some(80x24)` for exact stdout, while redirecting stdout
  alone returned `None` even though the fallback API found stderr's 80x24 PTY;
- the hard width edge reproduced: `(rows=0, columns=80)` returned `80` from Go
  and `None`/`0` from `terminal_size`;
- the exact selected Crossterm Mio reader reproduced a hard PTY-hangup failure:
  `event::poll(200ms)` remained in running state at 97–98% CPU after one second
  and required SIGKILL;
- Rust 1.96 checks passed for Linux, macOS, and Windows GNU; this is compile-only
  evidence for macOS/Windows, not native PTY/Console/ConPTY runtime evidence;
  and
- the focused 29-package lock scanned clean against 1,211 RustSec advisories.

The passing checks establish useful candidate properties but cannot override
the reproduced EOF/EIO and zero-row-width failures.

## Review

This critical runtime received the required second fresh adversarial review on
the exact base. The reviewer made no repository edits and independently
reproduced both hard failures. Result: **reject / blocked**.

Before approval, a successor decision must:

1. replace, patch, or reconfigure the event primitive and prove with native
   Linux and macOS PTY hangup tests that EOF/EIO cannot trap or leak a busy
   reader and that action/cancellation still governs after clean EOF;
2. replace `terminal_size` with an exact-stdout winsize primitive that accepts
   positive width with zero height, or obtain an explicit parity exception;
3. re-audit the successor's exact feature and transitive graph for licenses,
   RustSec, Rust 1.96, and Linux/macOS targets;
4. specify and byte-test capped Confirm input, suppressed Confirm/fallback
   writes, exact modifier matching, modifyOtherKeys/Kitty sequences and queries,
   listener/output/cleanup precedence, panic diagnostic order, all races, and
   RAII rollback under injected partial failures;
5. run process-isolated serialized PTY tests because raw mode and Crossterm's
   reader are process-global, and record the stable-FD/`/dev/tty` ownership
   contract; characterize overlapping oracle sessions and obtain explicit
   authority for any Rust concurrent-session rejection; and
6. provide native macOS PTY evidence or an approved verification exception.
   Windows remains excluded unless native Console plus ConPTY/stderr behavior
   is separately proven and approved.

Until those requirements are satisfied and another exact candidate decision is
approved, the controller must not update `migration/DEPENDENCIES.tsv`, unblock a
package, or treat any terminal dependency as selected.

Affected package: Go `upstream/uncloud/internal/cli/tui`; future migration crate
`crates/ployz-internal-cli-tui`. No package packet exists at this base.
