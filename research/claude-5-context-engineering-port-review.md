# Resetting issue #2 for faithful, maintainable parity

## Conclusion

Restart issue #2 from `848d05e`, keeping the first attempt only as a failure corpus. The first trial did not fail because it lacked rules. It failed because its context mixed the outcome, implementation mechanics, review workflow, and a growing edge-case checklist; then two reviewers received the same broad rubric. That simultaneously overconstrained Rust structure and left correlated behavioral blind spots.

The replacement should be a thin outcome contract, the frozen Go code and tests as rich references, a short discovery artifact handed to a fresh implementation context, and two **different** verification rubrics: behavioral parity and Rust maintainability. Fix and re-run only the failed rubric until it has no blockers.

## Rules to take from Anthropic

1. **State outcomes and invariants; leave implementation judgment to the model.** Anthropic reports removing over 80% of Claude Code's system prompt after finding newer models were overconstrained, and recommends matching the surrounding code's naming and idiom rather than prescribing brittle mechanics. [The new rules of context engineering](https://claude.com/blog/the-new-rules-of-context-engineering-for-claude-5-generation-models)
2. **Use the smallest high-signal context at the right altitude.** Prompts should be specific enough to define the result but flexible enough to avoid hard-coded decision trees. Context has an attention cost. [Effective context engineering for AI agents](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
3. **Prefer source and tests as references.** Anthropic calls source code the strongest reference and explicitly notes that a function in another codebase or a test suite can serve as the specification. The model should retrieve callers and relevant dependencies just in time instead of receiving a universal checklist. [The new rules of context engineering](https://claude.com/blog/the-new-rules-of-context-engineering-for-claude-5-generation-models), [Fable field guide](https://claude.com/blog/a-field-guide-to-claude-fable-finding-your-unknowns)
4. **Discover unknowns before committing to a design.** Start with a blind-spot pass, surface decisions likely to change architecture, then begin implementation in a fresh context with the resulting artifact. Record deviations encountered during implementation. [Fable field guide](https://claude.com/blog/a-field-guide-to-claude-fable-finding-your-unknowns)
5. **Separate creation from verification and give verifiers focused rubrics.** Fresh isolated contexts reduce self-preferential bias and goal drift. Anthropic recommends adversarial verification, focused subagents, and looping to an objective stop condition rather than running a fixed number of generic passes. [Dynamic workflows in Claude Code](https://claude.com/blog/a-harness-for-every-task-dynamic-workflows-in-claude-code)
6. **Do not build a workflow platform.** Anthropic's advice remains to do the simplest thing that works, and says ordinary coding tasks do not need large reviewer panels. For this trial, two specialized reviewers and a fixer are enough. [Effective context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents), [Dynamic workflows](https://claude.com/blog/a-harness-for-every-task-dynamic-workflows-in-claude-code)

## How the first trial failed

| Evidence in the repository | Context failure | Correction |
| --- | --- | --- |
| `PORTING_GUIDE.md:3` requires preservation of function boundaries and control flow. `config.rs` consequently recreates `equals`, `compare`, and a manual `Clone` implementation. | It specifies Go-shaped means instead of the parity outcome and conflicts with the user's maintainability goal. | Preserve observable behavior, file/module correspondence, and exported-concept traceability. Permit derives, traits, enums, RAII, ownership, typed errors, structured concurrency, and helper reshaping. |
| `PORTING_GUIDE.md:21` lists selected JSON concerns, while `config.rs:618-651` tests only exact PascalCase spelling, zero omission, defaults, and one base64 case. | A checklist and a few examples bounded exploration. No one treated `encoding/json` as a behavioral subsystem whose decode and encode surface had to be discovered from the oracle. | Inventory each imported Go API at a boundary and use executable Go probes for ambiguous behavior. Derive Rust assertions from probe output, not from memory or prose. The rerun must include case-insensitive names, duplicates, base64 whitespace, and escaping as regression cases. |
| `proxy.rs:311-343` wraps the custom dial future in a Rust timeout and drops it at the deadline; `proxy.rs:621-659` checks that a token was cancelled and an error appeared, but not what happens when the dialer ignores cancellation. | The test validated the chosen Rust mechanism, not the complete Go lifecycle. The guide's generic cancellation instruction was too coarse. | Describe cancellation as a timeline: who signals, who remains blocked, what is dropped, when errors surface, and what `Run` waits for. Probe a non-cooperative Go dialer and require the Rust behavior to match, even when that preserves a limitation. |
| Go selects `(&net.Dialer{}).DialContext`; Rust substitutes `TcpStream::connect` in `proxy.rs:238-245`. | Reviewers compared local control flow but did not audit the semantics delegated to a standard-library object. | Every substituted library boundary needs evidence of equivalence or an explicit, human-approved deviation. Default dialing behavior is part of the trial, not an internal detail. |
| Go has one `fmt.Errorf("chown %q: %w", ...)` layer; `fs.rs:102-110` adds `PathOperationError`, and its test at `fs.rs:188-201` explicitly expects the extra `ENOENT:` text. Unknown-user cases at `fs.rs:58-62` and `85-89` are newly constructed strings rather than wrapped typed causes. | Expected values were authored from the Rust implementation and then blessed by tests. The checklist emphasis on source chains encouraged synthetic structure that was not checked against Go output. | Run the same failing operation in the frozen Go oracle first. Compare user-visible text and only the classifications callers can observe; do not invent intermediate errors to satisfy an abstract rule. |
| `PORTING_GUIDE.md:9-11` gives both reviewers identical criteria and explicitly rejects a separate standards/spec role. `TRIAL_REVIEW.md:22-36` shows substantial overlap in findings but neither review perspective covered the four remaining gaps. | Fresh contexts alone do not provide diversity when the task and rubric are identical. The process also removed the counterweight against Go-shaped Rust. | Give one reviewer a behavioral-oracle rubric and the other a Rust soundness/maintainability rubric. Neither receives the implementer's rationale or the other report. |
| `TRIAL_REVIEW.md:62-76` declares success based on Rust build/test checks and oracle immutability. | Those checks prove internal consistency, not equivalence; the tests themselves encoded incorrect expectations. | Gate on oracle-derived cross-language probes plus Rust checks. A green Rust-only suite is necessary but insufficient. |

## Minimal issue #2 rerun packet

### Objective

From a fresh branch at `848d05e`, port these three Go source/test pairs to Rust:

- `internal/proxy/proxy.go` and `proxy_test.go`
- `internal/fs/fs.go` and `fs_test.go`
- `pkg/api/config.go` and `config_test.go`

Produce the same externally observable behavior, including limitations and flaws, while writing maintainable Rust. Keep an obvious module/file home and mapping for each Go package and exported concept. Do **not** preserve Go function boundaries, control flow, manual methods, error scaffolding, or concurrency mechanisms when a smaller normal Rust representation has the same behavior. Do not add features or fix upstream behavior.

Observable parity includes returned values, serialized bytes, accepted inputs, externally visible error text and classification, side effects, ordering, timing/cancellation, resource lifetime, and platform behavior when the Go callers can observe them.

### Context packet

Give each agent only:

- this objective;
- the frozen paths above and their direct callers;
- `UPSTREAM_ORACLE.md`;
- the compact discovery note produced in phase 1;
- the agent's own role rubric.

Do not give implementers the failed Rust port, `TRIAL_REVIEW.md`, or the old prescriptive guide. Keep those only as held-out regression evidence for reviewers. Avoid copying source, examples, and edge-case lists into a permanent all-purpose prompt; point to the code and let agents retrieve what is relevant.

### Execution loop

1. **Blind-spot pass, no Rust code.** Inspect the three Go pairs, direct callers, and every imported API that owns observable behavior. Run small Go probes where semantics are uncertain. Write one temporary compact note containing: observable surfaces, lifecycle timelines, library substitutions requiring proof, known unknowns, and proposed parity checks. Escalate only decisions that would change a public type or concurrency architecture.
2. **Fresh implementation context.** Implement from the objective, Go references, tests, and discovery note. Favor standard Rust derives and types, RAII, ownership, and structured async. Record only real deviations or unresolved questions in the note. Tests must encode Go-observed results, not merely exercise the Rust implementation.
3. **Fresh parity verifier.** Assume the port is wrong. Compare Go source, direct callers, Rust diff, and runtime behavior. Add or propose oracle probes, especially at standard-library, serialization, OS, error, and cancellation boundaries. The four missed classes above are mandatory regression cases, not implementation recipes.
4. **Fresh Rust verifier.** Assume the design is needlessly Go-shaped. Review public types, ownership, async/task lifetime, cancellation safety, error representation, trait objects, manual trait-equivalent methods, and dependency use. Block only soundness problems, costly one-way doors, or unnecessary complexity—not subjective formatting or harmless source-order differences.
5. **Separate fixer.** Resolve all blocker findings with the smallest change. If parity and idiom conflict, preserve observable behavior and choose the simplest Rust mechanism; escalate only when no reasonable Rust design can reproduce the behavior. Re-run only the affected verifier in a fresh context. Stop when both return zero blockers and all checks pass.
6. **Human gate.** Present a short exported-concept map, oracle probe results, unresolved deviations (expected to be none), and verifier dispositions. Do not begin bulk translation before approval.

### Acceptance criteria

- The original Go tests still pass and the frozen tree is unchanged.
- Rust formatting, compilation, Clippy, and tests pass with no ignored or weakened tests.
- The parity verifier has executed Go-oracle probes for each relevant serialization, error, library-boundary, and lifecycle seam.
- Regression evidence covers: Go JSON field matching/duplicates/base64 whitespace/escaping; a custom dialer that ignores cancellation; default `net.Dialer` behavior or an explicit approved equivalence strategy; and exact filesystem/unknown-user error behavior.
- Each exported Go concept has an obvious Rust counterpart, but Rust derives and idioms replace manual Go-shaped mechanics where behavior permits.
- Both specialized verifiers report zero blockers after the final fix.
- The permanent `PORTING_GUIDE.md` contains only the short outcome principles and **confirmed reusable gotchas** from the rerun. Workflow mechanics stay in the issue/workflow, not in every coding context.

## What not to build

No custom differential framework, semantic database, generalized porting harness, lifetime registry, or panel beyond the two focused verifiers. Shell commands and ordinary tests are sufficient for the three-file calibration. Add tooling only after a repeated failure demonstrates that manual probes no longer scale.
